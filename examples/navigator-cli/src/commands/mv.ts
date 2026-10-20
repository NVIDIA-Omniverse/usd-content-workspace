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

async function moveFolder(
  folderSvc: ReturnType<typeof fileFolderService>,
  objSvc: ReturnType<typeof fileObjectService>,
  srcFolder: string,
  dstFolder: string,
  log: Logger,
  concurrency: number,
  onConflict: ConflictPolicy,
  skipped: string[],
  completed: PreviewOperation[],
): Promise<boolean> {
  const result = await folderSvc.listAll(srcFolder);
  // Create every destination, including empty folders, before removing sources.
  await folderSvc.createFolder(dstFolder);
  completed.push({ action: "create-folder", source: dstFolder });
  let hasSkipped = false;

  await forEachWithConcurrency(
    result.subResourceAddresses,
    concurrency,
    async (fileAddr) => {
      const dstAddr = `${dstFolder}${fileAddr.slice(srcFolder.length)}`;
      log("mv", `${fileAddr} → ${dstAddr}`);
      try {
        await objSvc.move(fileAddr, dstAddr, onConflict !== "overwrite");
        completed.push({
          action: "move",
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
          hasSkipped = true;
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
      const childSkipped = await moveFolder(
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
      if (childSkipped) hasSkipped = true;
      else {
        await folderSvc.deleteFolder(subFolder);
        completed.push({ action: "delete-folder", source: subFolder });
      }
    },
  );
  return hasSkipped;
}

export async function runMv(
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
  log("mv", `${resolvedSrc} → ${resolvedDst}`);

  if (resolvedSrc.endsWith("/")) {
    const concurrency = getConcurrencyLimit();
    const srcFolder = resolvedSrc;
    const dstFolder = ensureFolderPath(resolvedDst);
    if (dstFolder.startsWith(srcFolder))
      throw new CliError(
        "Cannot move a folder into itself or its child path.",
        "INVALID_PATH",
      );
    const folderSvc = fileFolderService(api, log);
    if (opts.dryRun) {
      printPreview(
        await planFolderTransfer(folderSvc, srcFolder, dstFolder, "move"),
      );
      return;
    }
    const objSvc = fileObjectService(api, log);
    const skipped: string[] = [];
    await withMutationProgress(async (completed) => {
      const hasSkipped = await moveFolder(
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
      if (!hasSkipped) {
        await folderSvc.deleteFolder(srcFolder);
        completed.push({ action: "delete-folder", source: srcFolder });
      }
    });
    printSuccess(`Moved ${srcFolder} to ${dstFolder}`, {
      source: srcFolder,
      destination: dstFolder,
      skipped,
    });
  } else {
    if (opts.dryRun) {
      printPreview([
        { action: "move", source: resolvedSrc, destination: resolvedDst },
      ]);
      return;
    }
    try {
      await fileObjectService(api).move(
        resolvedSrc,
        resolvedDst,
        onConflict !== "overwrite",
      );
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
    printSuccess(`Moved ${resolvedSrc} to ${resolvedDst}`, {
      source: resolvedSrc,
      destination: resolvedDst,
      skipped: [],
    });
  }
}
