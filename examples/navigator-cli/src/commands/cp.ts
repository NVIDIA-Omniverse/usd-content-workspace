// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import {
  CliError,
  noopLogger,
  printPreview,
  printSuccess,
  type Logger,
  type PreviewOperation,
} from "@/output";
import { isHttpError, type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import {
  forEachWithConcurrency,
  getConcurrencyLimit,
} from "@/util/concurrency";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath } from "@/util/url";
import {
  withMutationProgress,
  type ConflictPolicy,
  type MutationOptions,
} from "@/util/mutation";
import { planFolderTransfer } from "@/util/transferPlan";

async function copyFolder(
  folderSvc: ReturnType<typeof fileFolderService>,
  objSvc: ReturnType<typeof fileObjectService>,
  srcFolder: string,
  dstFolder: string,
  log: Logger,
  concurrency: number,
  onConflict: ConflictPolicy,
  skipped: string[],
  completed: PreviewOperation[],
): Promise<void> {
  const result = await folderSvc.listAll(srcFolder);
  await folderSvc.createFolder(dstFolder);
  completed.push({ action: "create-folder", source: dstFolder });

  await forEachWithConcurrency(
    result.subResourceAddresses,
    concurrency,
    async (fileAddr) => {
      const dstAddr = `${dstFolder}${fileAddr.slice(srcFolder.length)}`;
      log("cp", `${fileAddr} → ${dstAddr}`);
      const { resourceIdentity } = await objSvc.stat(fileAddr);
      if (!resourceIdentity) {
        throw new Error(
          `Cannot copy "${fileAddr}": no resource identity found.`,
        );
      }
      try {
        await objSvc.copy(
          resourceIdentity,
          dstAddr,
          onConflict !== "overwrite",
        );
        completed.push({
          action: "copy",
          source: fileAddr,
          destination: dstAddr,
        });
      } catch (error) {
        if (
          onConflict === "skip" &&
          isHttpError(error) &&
          error.response.status === 409
        ) {
          skipped.push(dstAddr);
          return;
        }
        throw error;
      }
    },
  );

  await forEachWithConcurrency(
    result.subfolderAddresses,
    1,
    async (subFolder) => {
      const dstSubFolder = `${dstFolder}${subFolder.slice(srcFolder.length)}`;
      await copyFolder(
        folderSvc,
        objSvc,
        subFolder,
        dstSubFolder,
        log,
        concurrency,
        onConflict,
        skipped,
        completed,
      );
    },
  );
}

export async function runCp(
  api: ApiInstance,
  src: string,
  dst: string,
  log: Logger = noopLogger(),
  cwd?: string,
  opts: MutationOptions = {},
): Promise<void> {
  const onConflict = opts.onConflict ?? "fail";
  const caps = capabilitiesService(api);
  const resolvedSrc = await resolveStoragePath(caps, src, cwd);
  const resolvedDst = await resolveStoragePath(caps, dst, cwd);
  log("cp", `${resolvedSrc} → ${resolvedDst}`);

  if (resolvedSrc.endsWith("/")) {
    const concurrency = getConcurrencyLimit();
    const srcFolder = resolvedSrc;
    const dstFolder = ensureFolderPath(resolvedDst);
    if (dstFolder.startsWith(srcFolder))
      throw new CliError(
        "Cannot copy a folder into itself or its child path.",
        "INVALID_PATH",
      );
    const folderSvc = fileFolderService(api, log);
    if (opts.dryRun) {
      printPreview(
        await planFolderTransfer(folderSvc, srcFolder, dstFolder, "copy"),
      );
      return;
    }
    const objSvc = fileObjectService(api, log);
    const skipped: string[] = [];
    await withMutationProgress(async (completed) => {
      await copyFolder(
        folderSvc,
        objSvc,
        srcFolder,
        dstFolder,
        log,
        concurrency,
        onConflict,
        skipped,
        completed,
      );
    });
    printSuccess(`Copied ${srcFolder} to ${dstFolder}`, {
      source: srcFolder,
      destination: dstFolder,
      skipped,
    });
  } else {
    const service = fileObjectService(api);
    const srcStat = await service.stat(resolvedSrc);
    const identity = srcStat.resourceIdentity;
    if (!identity) {
      throw new Error(
        `Cannot copy "${resolvedSrc}": no resource identity found.`,
      );
    }
    if (opts.dryRun) {
      printPreview([
        { action: "copy", source: resolvedSrc, destination: resolvedDst },
      ]);
      return;
    }
    try {
      await service.copy(identity, resolvedDst, onConflict !== "overwrite");
    } catch (error) {
      if (
        onConflict !== "skip" ||
        !isHttpError(error) ||
        error.response.status !== 409
      )
        throw error;
      printSuccess(`Skipped ${resolvedDst}`, {
        source: resolvedSrc,
        destination: resolvedDst,
        skipped: [resolvedDst],
      });
      return;
    }
    printSuccess(`Copied ${resolvedSrc} to ${resolvedDst}`, {
      source: resolvedSrc,
      destination: resolvedDst,
      skipped: [],
    });
  }
}
