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
  noopLogger,
  printJson,
  printLs,
  printLsLong,
  type LsLongEntry,
  type Logger,
} from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath, getPathName } from "@/util/url";

interface LsOptions {
  long: boolean;
  json: boolean;
}

export async function runLs(
  api: ApiInstance,
  path: string | undefined,
  opts: LsOptions,
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  const folders = fileFolderService(api, log);
  const caps = capabilitiesService(api);

  if (!path && !cwd) {
    // No path, no cwd → list top-level bucket names
    const result = await caps.listTopLevelAddresses();
    const names = result.subfolderAddresses.map((a) => `${getPathName(a)}/`);
    if (opts.json) {
      return printJson(
        result.subfolderAddresses.map((a) => ({
          name: getPathName(a),
          type: "folder" as const,
          resourceAddress: a,
        })),
      );
    }
    return printLs(names);
  }

  // Resolve path: if provided, resolve it (may be relative to cwd); otherwise use cwd directly
  const resolvedPath = path
    ? ensureFolderPath(await resolveStoragePath(caps, path, cwd))
    : cwd!; // cwd is always stored with a trailing slash by runCd

  if (opts.long) {
    if (opts.json) {
      // JSON: buffer all pages then emit a single array
      const allEntries: LsLongEntry[] = [];
      for await (const page of folders.listStatPages(resolvedPath)) {
        allEntries.push(
          ...page.subfolderAddresses.map((a) => ({
            name: `${getPathName(a)}/`,
            isFolder: true as const,
            resourceAddress: a,
          })),
          ...page.entries.map((e) => ({
            name: getPathName(e.resourceAddress),
            isFolder: false as const,
            resourceAddress: e.resourceAddress,
            size: e.metadata.dataObjectSize,
            createdAt: e.metadata.lastModifiedTimestamp
              ? new Date(e.metadata.lastModifiedTimestamp)
              : undefined,
          })),
        );
      }
      return printJson(
        allEntries.map((e) => ({
          name: e.isFolder ? e.name.replace(/\/$/, "") : e.name,
          resourceAddress: e.resourceAddress,
          type: e.isFolder ? "folder" : "file",
          size: e.size,
          modifiedAt: e.createdAt?.toISOString(),
        })),
      );
    }
    for await (const page of folders.listStatPages(resolvedPath)) {
      printLsLong([
        ...page.subfolderAddresses.map((a) => ({
          name: `${getPathName(a)}/`,
          isFolder: true as const,
        })),
        ...page.entries.map((e) => ({
          name: getPathName(e.resourceAddress),
          isFolder: false as const,
          size: e.metadata.dataObjectSize,
          createdAt: e.metadata.lastModifiedTimestamp
            ? new Date(e.metadata.lastModifiedTimestamp)
            : undefined,
        })),
      ]);
    }
    return;
  }

  if (opts.json) {
    // JSON: buffer all pages then emit a single array
    const allEntries: {
      name: string;
      type: "file" | "folder";
      resourceAddress: string;
    }[] = [];
    for await (const page of folders.listPages(resolvedPath)) {
      allEntries.push(
        ...page.subfolderAddresses.map((a) => ({
          name: getPathName(a),
          type: "folder" as const,
          resourceAddress: a,
        })),
        ...page.subResourceAddresses.map((a) => ({
          name: getPathName(a),
          type: "file" as const,
          resourceAddress: a,
        })),
      );
    }
    return printJson(allEntries);
  }

  for await (const page of folders.listPages(resolvedPath)) {
    printLs([
      ...page.subfolderAddresses.map((a) => `${getPathName(a)}/`),
      ...page.subResourceAddresses.map((a) => getPathName(a)),
    ]);
  }
}
