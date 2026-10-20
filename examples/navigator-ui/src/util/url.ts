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

/**
 * Ensures that the specified URI ends with a trailing slash.
 * @param uri
 */
export function ensureFolderPath(uri: string): string {
  return uri.endsWith("/") ? uri : `${uri}/`;
}

/**
 * Decodes a path segment or filename for display. Safe to use with already-decoded strings.
 */
export function decodePathName(name: string): string {
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

/**
 * Extracts the filename or parent folder name from a URI path by taking the last segment
 */
export function getPathName(folder: string): string {
  const parts = folder.endsWith("/") ? folder.split("/").slice(0, -1) : folder.split("/");
  return decodePathName(parts[parts.length - 1] ?? "");
}

/**
 * Returns the file extension based on the specified URL.
 * @param url
 */
export function getFileExtension(url: string): string {
  const parts = getPathName(url).split(".");
  if (parts.length > 1) {
    return parts.pop()?.toUpperCase() ?? "Unknown";
  }
  return "Unknown";
}

export function join(...args: string[]): string {
  return args.reduce((path, value, index) => {
    if (!value) {
      return path;
    }

    let segment = value;
    if (segment.endsWith("/")) {
      segment = segment.substring(0, segment.length - 1);
    }

    if (index !== 0 && !segment.startsWith("/")) {
      segment = `/${segment}`;
    }

    return path + segment;
  }, "");
}

/**
 * Replaces characters that cannot be used in the path name with underscores.
 * @param name
 */
export function sanitizePathName(name: string) {
  return name.replace(/[\\/:*?"<>|]/g, "_");
}

/**
 * Returns the full parent path of a given URI.
 */
export function getParentPath(path: string): string {
  const parents = getParents(path);
  return parents[parents.length - 1];
}

/**
 * Returns an array of parent paths for a given URL, including the hostname.
 * Each parent path is an absolute URL.
 *
 * @example
 * // Returns ['https://example.com', 'https://example.com/foo']
 * getParents('https://example.com/foo/bar')
 *
 * @param path - The URL to get parents for
 * @returns An array of parent URLs, starting from the root
 */
export function getParents(path: string): string[] {
  const parents: string[] = [];
  const fullPath = path.endsWith("/") ? path.slice(0, -1) : path;

  let pathSegments;
  let baseUrl;
  try {
    const url = new URL(fullPath);
    pathSegments = url.pathname.split("/").filter((segment) => segment.length > 0);
    baseUrl = `${url.protocol}//${url.host}`;
  } catch (error) {
    pathSegments = path.split("/").filter((segment) => segment.length > 0);
    baseUrl = path.startsWith("/") ? "/" : "";
  }

  if (baseUrl !== fullPath && baseUrl !== "") {
    parents.push(baseUrl);
  }

  let currentPath = baseUrl;
  for (const segment of pathSegments) {
    currentPath = currentPath === "" ? segment : join(currentPath, segment);
    if (currentPath !== fullPath) {
      parents.push(currentPath);
    }
  }

  return parents;
}
