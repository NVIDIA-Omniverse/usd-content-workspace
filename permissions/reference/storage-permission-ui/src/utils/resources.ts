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

import type { ResourceAddress } from "../types/ResourceAddress.js";

export function encodeResource(resource: ResourceAddress): string {
  return `${resource.type}::"${encodeResourceUrl(resource.address)}"`;
}

/**
 * Builds the chain of resources to evaluate for inherited permissions: the
 * selection itself followed by each parent folder up to the root, ordered from
 * closest to farthest. Works for both special (http/https) and non-special
 * schemes such as omniverse://.
 *
 * @param resource Selected resource to start from.
 * @returns The selection followed by its ancestor folders, closest first.
 */
export function getResourceAncestors(
  resource: ResourceAddress,
): ResourceAddress[] {
  const url = new URL(resource.address);
  const segments = decodeURIComponent(url.pathname)
    .split("/")
    .filter((segment) => segment.length > 0);

  const base = `${url.protocol}//${url.host}`;
  const ancestors: ResourceAddress[] = [resource];
  for (let depth = segments.length - 1; depth >= 0; depth--) {
    const path = segments.slice(0, depth).join("/");
    const address = `${base}/${path ? `${path}/` : ""}`;
    ancestors.push({ address, type: "folder" });
  }

  return ancestors;
}

export function encodeResourceUrl(address: string): string {
  const url = new URL(address);
  const decodedPathname = decodeURIComponent(url.pathname);
  url.pathname = decodedPathname
    .split("")
    .map((char) => {
      if (unreservedChars.test(char)) {
        return char;
      }
      return encodeURIComponent(char);
    })
    .join("");

  return url.toString();
}

const unreservedChars = /[A-Za-z0-9._\-~/]/;
