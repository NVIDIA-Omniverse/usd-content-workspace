// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { CliError, printJson, printSuccess } from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { metadataService } from "@/services/storage/metadataService";
import { resolveStoragePath } from "@/util/resolve";

export async function runTags(
  api: ApiInstance,
  action: "list" | "set" | "remove",
  path: string,
  options: { key?: string; value?: string; json: boolean },
  cwd?: string,
): Promise<void> {
  const resolvedPath = await resolveStoragePath(
    capabilitiesService(api),
    path,
    cwd,
  );
  const service = metadataService(api);
  const metadata = await service.get(resolvedPath);
  const tags = metadata.tags.value;
  if (action === "list") {
    if (options.json)
      printJson({ path: resolvedPath, etag: metadata.tags.etag, tags });
    else
      for (const [key, value] of Object.entries(tags).sort(([a], [b]) =>
        a.localeCompare(b),
      ))
        process.stdout.write(`${key}=${value}\n`);
    return;
  }

  const key = options.key;
  if (!key) throw new CliError("tag key is required", "INVALID_ARGUMENT");
  if (action === "remove" && !Object.hasOwn(tags, key))
    throw new CliError(`tag '${key}' does not exist`, "NOT_FOUND", {
      path: resolvedPath,
      key,
    });
  const updated = { ...tags };
  if (action === "set") {
    if (options.value === undefined)
      throw new CliError("tag value is required", "INVALID_ARGUMENT");
    updated[key] = options.value;
  } else delete updated[key];
  await service.setTags(resolvedPath, updated, metadata.tags.etag);
  printSuccess(`Updated tags for ${resolvedPath}`, {
    path: resolvedPath,
    key,
    tags: updated,
  });
}
