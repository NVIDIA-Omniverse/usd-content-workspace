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

import { readdirSync, statSync } from "fs";
import { basename, join, relative, sep } from "path";
import {
  CliError,
  createFileProgressCallback,
  createFolderProgressCallback,
  noopLogger,
  printPreview,
  printSuccess,
  withBatchOutput,
  type Logger,
} from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import {
  forEachWithConcurrency,
  getConcurrencyLimit,
} from "@/util/concurrency";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath } from "@/util/url";
import { withMutationProgress, type MutationOptions } from "@/util/mutation";

function encodeLocalPath(path: string): string {
  return path.split(sep).map(encodeURIComponent).join("/");
}

function collectFiles(dir: string, directories: string[]): string[] {
  directories.push(dir);
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectFiles(full, directories));
    } else if (entry.isFile()) {
      files.push(full);
    }
  }
  return files;
}

export async function runPut(
  api: ApiInstance,
  localPath: string,
  remotePath: string | undefined,
  log: Logger = noopLogger(),
  cwd?: string,
  progress = false,
  opts: MutationOptions = {},
): Promise<void> {
  const { uploads, folderRoot, folders } = await planUploads(
    api,
    localPath,
    remotePath,
    cwd,
  );
  if (opts.dryRun) {
    printPreview([
      ...folders.map((source) => ({ action: "create-folder", source })),
      ...uploads.map(({ source, destination }) => ({
        action: "upload",
        source,
        destination,
      })),
    ]);
    return;
  }
  const policy = opts.onConflict ?? "fail";
  const skipped: string[] = [];
  if (policy !== "overwrite") {
    await forEachWithConcurrency(
      uploads,
      getConcurrencyLimit(),
      async ({ destination }) => {
        if (await remoteExists(api, destination)) skipped.push(destination);
      },
    );
  }
  if (policy === "fail" && skipped.length) {
    throw new CliError(
      `Destination already exists: ${skipped.join(", ")}`,
      "CONFLICT",
    );
  }
  const skippedSet = new Set(skipped);
  const pending = uploads.filter(
    ({ destination }) => !skippedSet.has(destination),
  );
  const service = fileObjectService(api, log);

  if (folderRoot) {
    await withMutationProgress(async (completed) => {
      const folderService = fileFolderService(api, log);
      for (const folder of folders) {
        await folderService.createFolder(folder);
        completed.push({ action: "create-folder", source: folder });
      }
      const folderStartMs = Date.now();
      let bytesCompleted = 0;
      const getAggregateSpeed = () =>
        bytesCompleted / Math.max((Date.now() - folderStartMs) / 1000, 0.001);

      await forEachWithConcurrency(
        pending,
        getConcurrencyLimit(),
        async ({ source, destination }, index) => {
          const rel = relative(localPath, source).replace(/\\/g, "/");
          const size = statSync(source).size;
          const fileOnProgress = progress
            ? createFolderProgressCallback(
                "Uploading",
                rel,
                index + 1,
                pending.length,
                getAggregateSpeed,
              )
            : undefined;
          await service.upload(source, destination, fileOnProgress);
          completed.push({ action: "upload", source, destination });
          bytesCompleted += size;
        },
      );
    });
    if (progress && pending.length > 0) process.stderr.write("\n");
    printSuccess(`Uploaded ${pending.length} files to ${folderRoot}`, {
      source: localPath,
      destination: folderRoot,
      files: pending.length,
      skipped,
    });
    return;
  }
  const resolvedPath = uploads[0].destination;
  if (skipped.length > 0) {
    printSuccess(`Skipped ${resolvedPath}`, {
      source: localPath,
      destination: resolvedPath,
      skipped,
    });
    return;
  }
  const onProgress = progress
    ? createFileProgressCallback("Uploading", basename(localPath))
    : undefined;
  await service.upload(localPath, resolvedPath, onProgress);
  printSuccess(`Uploaded ${localPath} to ${resolvedPath}`, {
    source: localPath,
    destination: resolvedPath,
    skipped,
  });
}

async function planUploads(
  api: ApiInstance,
  localPath: string,
  remotePath: string | undefined,
  cwd?: string,
) {
  const caps = capabilitiesService(api);
  if (statSync(localPath).isDirectory()) {
    if (!remotePath && !cwd)
      throw new Error(
        "no current folder set. Use 'sn cd <path>' to set one, or specify a remote path.",
      );
    const base = remotePath
      ? await resolveStoragePath(caps, remotePath, cwd)
      : cwd!;
    const folderRoot = ensureFolderPath(
      `${ensureFolderPath(base)}${encodeURIComponent(basename(localPath))}`,
    );
    const directories: string[] = [];
    const uploads = collectFiles(localPath, directories).map((source) => ({
      source,
      destination: `${folderRoot}${encodeLocalPath(relative(localPath, source))}`,
    }));
    const folders = directories.map((directory) =>
      ensureFolderPath(
        `${folderRoot}${encodeLocalPath(relative(localPath, directory))}`,
      ),
    );
    return { uploads, folderRoot, folders };
  }
  if (!remotePath && !cwd)
    throw new Error(
      "no current folder set. Use 'sn cd <path>' to set one, or specify a remote path.",
    );
  const destination = await resolveStoragePath(
    caps,
    remotePath ?? `./${encodeURIComponent(basename(localPath))}`,
    cwd,
  );
  return {
    uploads: [{ source: localPath, destination }],
    folderRoot: undefined,
    folders: [] as string[],
  };
}

async function remoteExists(
  api: ApiInstance,
  address: string,
): Promise<boolean> {
  const response = await api.head(
    `v1alpha/fileobject/by-address/${encodeURIComponent(address)}`,
    { throwHttpErrors: false },
  );
  if (response.status === 204) return true;
  if (response.status === 404) return false;
  throw new Error(
    `Failed to check destination ${address}: HTTP${response.status}`,
  );
}

/**
 * Uploads multiple local paths concurrently to `cwd/<basename>` for each.
 * Per-line errors are collected and reported after all lines are attempted.
 */
export async function runPutLines(
  api: ApiInstance,
  lines: string[],
  log: Logger = noopLogger(),
  cwd?: string,
  progress = false,
  opts: MutationOptions = {},
): Promise<void> {
  if (opts.dryRun) {
    const plans: Awaited<ReturnType<typeof planUploads>>[] = [];
    for (const line of lines)
      plans.push(await planUploads(api, line, undefined, cwd));
    printPreview(
      plans.flatMap(({ uploads, folders }) => [
        ...folders.map((source) => ({ action: "create-folder", source })),
        ...uploads.map(({ source, destination }) => ({
          action: "upload",
          source,
          destination,
        })),
      ]),
    );
    return;
  }
  const errors: string[] = [];
  const completed: string[] = [];
  const failureDetails: { path: string; details: unknown }[] = [];
  await withBatchOutput(() =>
    forEachWithConcurrency(lines, 1, async (line) => {
      try {
        await runPut(api, line, undefined, log, cwd, progress, opts);
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
  printSuccess(`Uploaded ${lines.length} paths`, { paths: lines.length });
}
