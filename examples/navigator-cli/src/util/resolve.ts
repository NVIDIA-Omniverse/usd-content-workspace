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

import { type CapsService } from "@/services/storage/capabilitiesService";
import { ensureFolderPath, getPathName } from "@/util/url";

/**
 * Resolve a user-supplied storage path to a fully-qualified storage address.
 *
 * Resolution rules (applied in order):
 * 1. Contains "://" → absolute, return as-is
 * 2. Starts with "./" + cwd set → relative, strip "./" and prepend cwd
 * 3. Starts with "./" + no cwd → error
 * 4. First segment matches a known top-level bucket → absolute bucket lookup
 * 5. No bucket match + cwd set → relative, prepend cwd
 * 6. No bucket match + no cwd → error (unknown path)
 *
 * Trailing slashes are preserved. Callers requiring folder semantics should
 * apply ensureFolderPath() after this function.
 */
function normalizeDots(url: string): string {
  try {
    return new URL(url).href;
  } catch {
    return url;
  }
}

export async function resolveStoragePath(
  caps: CapsService,
  path: string,
  cwd?: string,
): Promise<string> {
  if (path.includes("://")) {
    return normalizeDots(path);
  }

  if (
    path.startsWith("./") ||
    path === "." ||
    path.includes("/./") ||
    path.includes("/../") ||
    path.startsWith("../") ||
    path === ".."
  ) {
    if (!cwd) {
      throw new Error("No current folder set. Run 'sn cd' to set one.");
    }
    return normalizeDots(`${ensureFolderPath(cwd)}${path}`);
  }

  const { subfolderAddresses } = await caps.listTopLevelAddresses();

  const slashIdx = path.indexOf("/");
  const topName = slashIdx === -1 ? path : path.slice(0, slashIdx);
  const subPath = slashIdx === -1 ? "" : path.slice(slashIdx + 1);

  const baseAddress = subfolderAddresses.find(
    (a) => getPathName(a) === topName,
  );
  if (baseAddress) {
    return normalizeDots(subPath ? `${baseAddress}${subPath}` : baseAddress);
  }

  if (cwd) {
    return normalizeDots(`${ensureFolderPath(cwd)}${path}`);
  }

  const available = subfolderAddresses.map((a) => getPathName(a)).join(", ");
  throw new Error(`Unknown path "${topName}". Available: ${available}`);
}
