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

import chalk from "chalk";
import { format } from "date-fns";
import { formatSize, printJson } from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { metadataService } from "@/services/storage/metadataService";
import { resolveStoragePath } from "@/util/resolve";
import { getPathName } from "@/util/url";

export async function runMeta(
  api: ApiInstance,
  path: string,
  opts: { json: boolean },
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolvedPath = await resolveStoragePath(caps, path, cwd);
  const isFolder = resolvedPath.endsWith("/");
  const [result, metadata] = await Promise.all([
    isFolder ? null : fileObjectService(api).stat(resolvedPath),
    metadataService(api).get(resolvedPath),
  ]);

  if (opts.json) {
    return printJson({
      path: resolvedPath,
      type: isFolder ? "folder" : "file",
      resourceIdentity: result?.resourceIdentity ?? null,
      size: result?.metadata.dataObjectSize ?? null,
      modifiedAt: result?.metadata.lastModifiedTimestamp ?? null,
      acl: metadata.acl,
      tags: metadata.tags,
    });
  }

  const modifiedAt = result?.metadata.lastModifiedTimestamp
    ? format(
        new Date(result.metadata.lastModifiedTimestamp),
        "MMM dd, yyyy HH:mm:ss",
      )
    : "unknown";

  const label = (s: string) => chalk.dim(s);
  process.stdout.write(`${label("Path:    ")} ${resolvedPath}\n`);
  process.stdout.write(`${label("Name:    ")} ${getPathName(resolvedPath)}\n`);
  if (result) {
    process.stdout.write(
      `${label("Size:    ")} ${formatSize(result.metadata.dataObjectSize)}\n`,
    );
    process.stdout.write(`${label("Modified:")} ${modifiedAt}\n`);
    process.stdout.write(
      `${label("Identity:")} ${result.resourceIdentity ?? "unknown"}\n`,
    );
  }
  process.stdout.write(
    `${label("ACL:     ")} ${metadata.acl.value.join(", ")}\n`,
  );
  process.stdout.write(
    `${label("Tags:    ")} ${JSON.stringify(metadata.tags.value)}\n`,
  );
}
