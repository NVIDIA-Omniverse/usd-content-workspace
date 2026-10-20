// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { randomUUID } from "node:crypto";
import { access, link, rename, unlink } from "node:fs/promises";
import { createWriteStream } from "node:fs";
import { basename, resolve } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { z } from "zod";
import { getDiscoveredApi, type CommandContext } from "@/context";
import { CliError, printSuccess } from "@/output";
import { create } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { resolveStoragePath } from "@/util/resolve";

const PrepareResponseSchema = z.object({
  download_action: z.string(),
  expires_at: z.string(),
});

export async function runArchive(
  context: CommandContext,
  paths: string[],
  output: string,
  overwrite: boolean,
): Promise<void> {
  const service = await getDiscoveredApi(context.api, "archive");
  if (!service)
    throw new CliError(
      "archive API is not available on this host",
      "SERVICE_UNAVAILABLE",
    );
  const caps = capabilitiesService(context.api);
  const items = await Promise.all(
    paths.map(async (path) => {
      const address = await resolveStoragePath(caps, path, context.cwd);
      return { type: address.endsWith("/") ? "folder" : "file", address };
    }),
  );
  const destination = resolve(output);
  const archiveName = basename(destination).replace(/\.zip$/i, "");
  if (!archiveName)
    throw new CliError("output must name a ZIP file", "INVALID_ARGUMENT");
  if (
    !overwrite &&
    (await access(destination).then(
      () => true,
      () => false,
    ))
  )
    throw new CliError(
      `local file already exists: ${destination}`,
      "CONFLICT",
      { output: destination },
    );
  const archiveApi = create(service.url, {
    hostName: context.hostName,
    discovery: context.discoveryUrl,
    verbose: context.verbose,
  });
  const prepare = await archiveApi.post("archive/prepare", {
    json: { items, archive_name: archiveName },
  });
  const { download_action: downloadAction } = PrepareResponseSchema.parse(
    await prepare.json(),
  );
  const cookie = prepare.headers.get("set-cookie")?.split(";", 1)[0];
  if (!cookie)
    throw new CliError(
      "archive prepare did not return a download cookie",
      "COMMAND_FAILED",
    );
  const action = new URL(
    downloadAction,
    `${service.baseUrl.replace(/\/$/, "")}/`,
  );
  if (action.origin !== new URL(service.baseUrl).origin)
    throw new CliError(
      "archive download action has a different origin",
      "COMMAND_FAILED",
    );
  const body = new URLSearchParams();
  items.forEach((item, index) => {
    body.set(`items[${index}][type]`, item.type);
    body.set(`items[${index}][address]`, item.address);
  });
  body.set("archive_name", archiveName);
  const response = await fetch(action, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      Cookie: cookie,
    },
    body,
    redirect: "manual",
  });
  if (!response.ok || !response.body)
    throw new CliError(
      `archive download failed: HTTP ${response.status}`,
      "COMMAND_FAILED",
    );

  const temporary = `${destination}.sn-${randomUUID()}`;
  try {
    await pipeline(
      Readable.fromWeb(response.body),
      createWriteStream(temporary, { flags: "wx" }),
    );
    if (overwrite) await rename(temporary, destination);
    else {
      await link(temporary, destination);
      await unlink(temporary);
    }
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    if ((error as NodeJS.ErrnoException).code === "EEXIST")
      throw new CliError(
        `local file already exists: ${destination}`,
        "CONFLICT",
        { output: destination },
      );
    throw error;
  }
  printSuccess(`Downloaded archive to ${destination}`, {
    output: destination,
    items,
  });
}
