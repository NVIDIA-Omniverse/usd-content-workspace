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
  withBatchOutput,
  type Logger,
  type PreviewOperation,
} from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import {
  forEachWithConcurrency,
  getConcurrencyLimit,
} from "@/util/concurrency";
import { withMutationProgress } from "@/util/mutation";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath } from "@/util/url";

async function collectDeletions(
  svc: ReturnType<typeof fileFolderService>,
  folderAddress: string,
  files: string[],
  folders: string[],
  traversalConcurrency: number,
): Promise<void> {
  const result = await svc.listAll(folderAddress);
  files.push(...result.subResourceAddresses);
  await forEachWithConcurrency(
    result.subfolderAddresses,
    1,
    async (subfolder) => {
      await collectDeletions(
        svc,
        subfolder,
        files,
        folders,
        traversalConcurrency,
      );
    },
  );
  // Push folder after children so deletion order is leaf-first
  folders.push(folderAddress);
}

export async function runRm(
  api: ApiInstance,
  path: string,
  opts: { recursive: boolean; dryRun?: boolean },
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolvedPath = await resolveStoragePath(caps, path, cwd);
  log("rm", resolvedPath);

  if (opts.dryRun) {
    printPreview(await planRm(api, resolvedPath, opts.recursive));
    return;
  }

  if (opts.recursive) {
    const concurrency = getConcurrencyLimit();
    const folderPath = ensureFolderPath(resolvedPath);
    const folderSvc = fileFolderService(api);
    const objSvc = fileObjectService(api);

    const files: string[] = [];
    const folders: string[] = [];
    await collectDeletions(folderSvc, folderPath, files, folders, concurrency);

    await withMutationProgress(async (completed) => {
      await forEachWithConcurrency(files, concurrency, async (file) => {
        await objSvc.delete(file);
        completed.push({ action: "delete", source: file });
      });
      for (const folder of folders) {
        await folderSvc.deleteFolder(folder);
        completed.push({ action: "delete-folder", source: folder });
      }
    });

    printSuccess(`Deleted ${folderPath}`, {
      path: folderPath,
      files: files.length,
      folders: folders.length,
    });
  } else if (resolvedPath.endsWith("/")) {
    throw new Error(`"${resolvedPath}" is a folder. use -r to remove folders.`);
  } else {
    await fileObjectService(api).delete(resolvedPath);
    printSuccess(`Deleted ${resolvedPath}`, { path: resolvedPath });
  }
}

async function planRm(
  api: ApiInstance,
  resolvedPath: string,
  recursive: boolean,
): Promise<PreviewOperation[]> {
  if (!recursive) {
    if (resolvedPath.endsWith("/"))
      throw new Error(
        `"${resolvedPath}" is a folder. use -r to remove folders.`,
      );
    return [{ action: "delete", source: resolvedPath }];
  }
  const files: string[] = [];
  const folders: string[] = [];
  await collectDeletions(
    fileFolderService(api),
    ensureFolderPath(resolvedPath),
    files,
    folders,
    getConcurrencyLimit(),
  );
  return [
    ...files.map((source) => ({ action: "delete", source })),
    ...folders.map((source) => ({ action: "delete-folder", source })),
  ];
}

/**
 * Deletes multiple remote paths concurrently. The `opts.recursive` flag applies to every line.
 * Per-line errors are collected and reported after all lines are attempted.
 */
export async function runRmLines(
  api: ApiInstance,
  lines: string[],
  opts: { recursive: boolean; dryRun?: boolean },
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  if (opts.dryRun) {
    const caps = capabilitiesService(api);
    const plans: PreviewOperation[][] = [];
    for (const line of lines)
      plans.push(
        await planRm(
          api,
          await resolveStoragePath(caps, line, cwd),
          opts.recursive,
        ),
      );
    printPreview(plans.flat());
    return;
  }
  const errors: string[] = [];
  const completed: string[] = [];
  const failureDetails: { path: string; details: unknown }[] = [];
  await withBatchOutput(() =>
    forEachWithConcurrency(lines, 1, async (line) => {
      try {
        await runRm(api, line, opts, log, cwd);
        completed.push(line);
      } catch (e) {
        failureDetails.push({
          path: line,
          details: e instanceof CliError ? e.details : undefined,
        });
        errors.push(`${line}: ${(e as Error).message}`);
      }
    }),
  );
  if (errors.length > 0)
    throw new CliError(
      `${errors.length} error(s): ${errors.join("; ")}`,
      "PARTIAL_FAILURE",
      { failures: errors, completed, failureDetails },
    );
  printSuccess(`Deleted ${lines.length} paths`, { paths: lines.length });
}
