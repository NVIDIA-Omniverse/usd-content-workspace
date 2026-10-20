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

import { format } from "date-fns";
import { type ApiInstance } from "@/services/api";
import {
  formatSize,
  noopLogger,
  printJson,
  printSuccess,
  type Logger,
} from "@/output";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { versioningService } from "@/services/storage/versioningService";
import { resolveStoragePath } from "@/util/resolve";

export async function runVersionsList(
  api: ApiInstance,
  path: string,
  opts: { json: boolean },
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolvedPath = await resolveStoragePath(caps, path, cwd);
  log("versions", `listing versions for ${resolvedPath}`);
  const items = await versioningService(api).listAll(resolvedPath);

  if (opts.json) {
    return printJson(
      items.map(({ resourceInfo: { resourceIdentity, metadata } }) => ({
        resourceIdentity,
        dataObjectSize: metadata.dataObjectSize,
        lastModifiedTimestamp: metadata.lastModifiedTimestamp ?? null,
      })),
    );
  }

  process.stdout.write(
    `${"IDENTITY".padEnd(60)}  ${"SIZE".padStart(9)}  ${"DATE".padStart(16)}\n`,
  );
  for (const {
    resourceInfo: { resourceIdentity, metadata },
  } of items) {
    const size = formatSize(metadata.dataObjectSize).padStart(9);
    const date = metadata.lastModifiedTimestamp
      ? format(
          new Date(metadata.lastModifiedTimestamp),
          "yyyy-MM-dd HH:mm",
        ).padStart(16)
      : "".padStart(16);
    process.stdout.write(`${resourceIdentity.padEnd(60)}  ${size}  ${date}\n`);
  }
}

export async function runVersionsRestore(
  api: ApiInstance,
  path: string,
  identity: string,
  log: Logger = noopLogger(),
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolvedPath = await resolveStoragePath(caps, path, cwd);
  log("versions", `restoring ${resolvedPath} @ ${identity}`);
  await fileObjectService(api).copy(identity, resolvedPath);
  printSuccess(`Restored ${identity} to ${resolvedPath}`, {
    path: resolvedPath,
    identity,
  });
}
