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

import { readConfig, updateConfig } from "@/config";
import { noopLogger, printSuccess, type Logger } from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { resolveStoragePath } from "@/util/resolve";
import { ensureFolderPath, getPathName } from "@/util/url";

export async function runCd(
  api: ApiInstance,
  path: string | undefined,
  hostName: string,
  log: Logger = noopLogger(),
): Promise<void> {
  const config = readConfig();
  const hostConfig = config.hosts[hostName];
  if (!hostConfig) {
    throw new Error(`host '${hostName}' not found.`);
  }

  if (!path || path === "/") {
    const prevCwd = hostConfig.cwd;
    await updateConfig((latest) => {
      const host = latest.hosts[hostName];
      if (!host) throw new Error(`host '${hostName}' not found.`);
      host.cwd = undefined;
    });
    printSuccess(
      prevCwd
        ? "Cleared working folder (now at root)"
        : "No current folder set.",
      { host: hostName, cwd: null },
    );
    return;
  }

  const caps = capabilitiesService(api);
  const resolved = ensureFolderPath(
    await resolveStoragePath(caps, path, hostConfig.cwd),
  );
  log("cd", resolved);

  // Compute short display path: find which top-level bucket the resolved URL is under,
  // then reconstruct "bucketName/sub/path/". Requires a second listTopLevelAddresses
  // call since resolveStoragePath consumed the first one internally.
  const { subfolderAddresses } = await caps.listTopLevelAddresses();
  const base = subfolderAddresses.find((a) => resolved.startsWith(a));
  const displayPath = base
    ? `${getPathName(base)}/${resolved.slice(base.length)}`
    : resolved;

  await updateConfig((latest) => {
    const host = latest.hosts[hostName];
    if (!host || host.discoveryUrl !== hostConfig.discoveryUrl)
      throw new Error(`host '${hostName}' changed during cd.`);
    host.cwd = resolved;
  });
  printSuccess(`Now in ${displayPath}`, { host: hostName, cwd: resolved });
}
