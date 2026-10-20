// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { lstatSync, mkdirSync, realpathSync } from "fs";
import { isAbsolute, join, relative, resolve, sep, win32 } from "path";
import { CliError } from "@/output";

function invalidPath(path: string): CliError {
  return new CliError(`Unsafe download path: ${path}`, "INVALID_PATH");
}

function decodeSegment(segment: string): string {
  let decoded: string;
  try {
    decoded = decodeURIComponent(segment);
  } catch {
    throw invalidPath(segment);
  }
  if (
    !decoded ||
    decoded === "." ||
    decoded === ".." ||
    decoded.includes("/") ||
    decoded.includes("\\") ||
    decoded.includes("\0") ||
    /^[a-z]:/i.test(decoded) ||
    win32.isAbsolute(decoded)
  )
    throw invalidPath(segment);
  // Windows interprets colons as alternate data streams and trims dots/spaces.
  if (process.platform === "win32" && /[:<>"|?*]|[. ]$/.test(decoded))
    throw invalidPath(segment);
  return decoded;
}

export function downloadName(address: string): string {
  const path = address.endsWith("/") ? address.slice(0, -1) : address;
  return decodeSegment(path.slice(path.lastIndexOf("/") + 1));
}

/** Validate the address before translating URI segments into filesystem segments. */
export function relativeDownloadPath(folder: string, address: string): string {
  if (!address.startsWith(folder) || address === folder)
    throw invalidPath(address);
  const suffix = address.slice(folder.length).replace(/\/$/, "");
  return join(...suffix.split("/").map(decodeSegment));
}

/** The user chooses the base directory; remote names may not traverse symlinks beneath it. */
export function prepareDownloadRoot(directory: string): string {
  mkdirSync(directory, { recursive: true });
  return realpathSync(directory);
}

export function ensureDownloadDirectory(root: string, directory: string): void {
  const suffix = relative(root, resolve(directory));
  if (suffix === ".." || suffix.startsWith(`..${sep}`) || isAbsolute(suffix))
    throw invalidPath(directory);
  if (lstatSync(root).isSymbolicLink()) throw invalidPath(root);
  let current = root;
  for (const segment of suffix ? suffix.split(sep) : []) {
    current = join(current, segment);
    try {
      mkdirSync(current);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
    const stat = lstatSync(current);
    if (stat.isSymbolicLink() || !stat.isDirectory())
      throw invalidPath(current);
  }
}
