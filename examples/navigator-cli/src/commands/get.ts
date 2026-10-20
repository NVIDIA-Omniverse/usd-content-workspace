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

import { existsSync } from "fs";
import { dirname, join, resolve } from "path";
import {
  CliError,
  createFileProgressCallback,
  createFolderProgressCallback,
  noopLogger,
  printSuccess,
  withBatchOutput,
  type Logger,
} from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import {
  forEachWithConcurrency,
  getConcurrencyLimit,
} from "@/util/concurrency";
import { resolveStoragePath } from "@/util/resolve";
import {
  downloadName,
  ensureDownloadDirectory,
  prepareDownloadRoot,
  relativeDownloadPath,
} from "@/util/downloadPath";

async function collectRemoteFiles(
  service: ReturnType<typeof fileFolderService>,
  folderAddress: string,
  visited = new Set<string>(),
): Promise<string[]> {
  if (visited.has(folderAddress))
    throw new CliError(
      `Repeated folder in download listing: ${folderAddress}`,
      "INVALID_PATH",
    );
  visited.add(folderAddress);
  const result = await service.listAll(folderAddress);
  for (const file of result.subResourceAddresses) {
    relativeDownloadPath(folderAddress, file);
    if (file.endsWith("/"))
      throw new CliError(`Invalid file address: ${file}`, "INVALID_PATH");
  }
  for (const subfolder of result.subfolderAddresses) {
    relativeDownloadPath(folderAddress, subfolder);
    if (!subfolder.endsWith("/"))
      throw new CliError(
        `Invalid folder address: ${subfolder}`,
        "INVALID_PATH",
      );
  }
  const files = [...result.subResourceAddresses];
  for (const subfolder of result.subfolderAddresses) {
    files.push(...(await collectRemoteFiles(service, subfolder, visited)));
  }
  return files;
}

export async function runGet(
  api: ApiInstance,
  remotePath: string,
  localDest: string | undefined,
  log: Logger = noopLogger(),
  cwd?: string,
  progress = false,
  overwrite = false,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolvedPath = await resolveStoragePath(caps, remotePath, cwd);

  if (resolvedPath.endsWith("/")) {
    const folderName = downloadName(resolvedPath);
    const folder = fileFolderService(api, log);
    const folders = new Set<string>();
    const files = await collectRemoteFiles(folder, resolvedPath, folders);
    const base = prepareDownloadRoot(localDest ?? process.cwd());
    const localRoot = join(base, folderName);
    ensureDownloadDirectory(base, localRoot);
    for (const address of folders) {
      if (address !== resolvedPath)
        ensureDownloadDirectory(
          base,
          join(localRoot, relativeDownloadPath(resolvedPath, address)),
        );
    }
    const destinations = files.map((address) => ({
      address,
      relativePath: relativeDownloadPath(resolvedPath, address),
    }));
    for (const { relativePath } of destinations)
      ensureDownloadDirectory(base, dirname(join(localRoot, relativePath)));

    const folderStartMs = Date.now();
    let bytesCompleted = 0;
    const getAggregateSpeed = () =>
      bytesCompleted / Math.max((Date.now() - folderStartMs) / 1000, 0.001);

    const service = fileObjectService(api, log);
    await forEachWithConcurrency(
      destinations,
      getConcurrencyLimit(),
      async ({ address: fileAddress, relativePath: rel }, index) => {
        const localFilePath = join(localRoot, rel);

        if (!overwrite && existsSync(localFilePath)) {
          throw new CliError(
            `File already exists: ${localFilePath} (use --overwrite to replace it)`,
            "CONFLICT",
          );
        }

        let fileBytes = 0;
        let onProgress:
          | ((transferred: number, total: number, speed: number) => void)
          | undefined;
        if (progress) {
          const cb = createFolderProgressCallback(
            "Downloading",
            rel,
            index + 1,
            files.length,
            getAggregateSpeed,
          );
          onProgress = (transferred, total, speed) => {
            fileBytes = transferred;
            cb(transferred, total, speed);
          };
        }

        await service.download(fileAddress, localFilePath, onProgress, {
          overwrite,
          root: base,
        });
        bytesCompleted += fileBytes;
      },
    );
    if (progress && files.length > 0) process.stderr.write("\n");

    printSuccess(`Downloaded ${files.length} files to ${localRoot}`, {
      source: resolvedPath,
      destination: localRoot,
      files: files.length,
    });
    return;
  }

  const fileName = downloadName(resolvedPath);
  const localPath = localDest ?? join(process.cwd(), fileName);
  if (!overwrite && existsSync(localPath)) {
    throw new CliError(
      `File already exists: ${localPath} (use --overwrite to replace it)`,
      "CONFLICT",
    );
  }
  const service = fileObjectService(api, log);
  const onProgress = progress
    ? createFileProgressCallback("Downloading", fileName)
    : undefined;
  await service.download(resolvedPath, localPath, onProgress, { overwrite });
  printSuccess(`Downloaded to ${localPath}`, {
    source: resolvedPath,
    destination: localPath,
  });
}

/**
 * Downloads multiple remote paths concurrently. Lines ending with `/` are treated as
 * folders (dest passed through to runGet, which appends the folder name). File lines
 * are pre-resolved to `join(localDest, filename)` and passed as the full file path to
 * runGet, bypassing runGet's own filename-append logic.
 * Per-line errors are collected and reported after all lines are attempted.
 */
export async function runGetLines(
  api: ApiInstance,
  lines: string[],
  localDest: string | undefined,
  log: Logger = noopLogger(),
  cwd?: string,
  progress = false,
  overwrite = false,
): Promise<void> {
  const errors: string[] = [];
  const completed: string[] = [];
  const failureDetails: { path: string; details: unknown }[] = [];
  const usedDestinations = new Set<string>();
  await withBatchOutput(() =>
    forEachWithConcurrency(lines, 1, async (line) => {
      try {
        const dest = line.endsWith("/")
          ? localDest
          : join(localDest ?? process.cwd(), downloadName(line));
        if (!line.endsWith("/") && dest) {
          const target = resolve(dest);
          if (usedDestinations.has(target))
            throw new CliError(
              `Multiple input paths target the same local file: ${target}`,
              "CONFLICT",
            );
          usedDestinations.add(target);
        }
        await runGet(api, line, dest, log, cwd, progress, overwrite);
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
  printSuccess(`Downloaded ${lines.length} paths`, {
    paths: lines.length,
    destination: localDest ?? process.cwd(),
  });
}
