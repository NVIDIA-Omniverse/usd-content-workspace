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
import ky from "ky";
import { z } from "zod";
import { printJson } from "@/output";
import { type ApiInstance } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import {
  thumbnailService,
  type ThumbnailEntry,
} from "@/services/thumbnailService";
import { resolveStoragePath } from "@/util/resolve";
import { fromSnakeCaseSchema } from "@/util/schemas";

// Width and height requested from the thumbnail service, as the Storage
// Navigator UI does.
const THUMBNAIL_SIZE = 256;

const DownloadRedirectSchema = fromSnakeCaseSchema(
  z.object({
    redirectTargetUrl: z.string(),
    additionalHeaders: z.record(z.string(), z.string()),
  }),
);

const MIME_BY_EXT: Record<string, string> = {
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  png: "image/png",
  gif: "image/gif",
  webp: "image/webp",
};

function mimeFromPath(filePath: string): string {
  const ext = filePath.split(".").pop()?.toLowerCase() ?? "";
  return MIME_BY_EXT[ext] ?? "image/jpeg";
}

export function detectImageProtocol(): "iterm2" | "kitty" | null {
  const { TERM_PROGRAM, ITERM_SESSION_ID, KITTY_WINDOW_ID, TERM } = process.env;
  if (
    TERM_PROGRAM === "iTerm.app" ||
    TERM_PROGRAM === "WezTerm" ||
    (ITERM_SESSION_ID !== undefined && ITERM_SESSION_ID !== "")
  ) {
    return "iterm2";
  }
  if (
    TERM_PROGRAM === "ghostty" ||
    (KITTY_WINDOW_ID !== undefined && KITTY_WINDOW_ID !== "") ||
    TERM === "xterm-kitty"
  ) {
    return "kitty";
  }
  return null;
}

const KITTY_CHUNK_SIZE = 4096;

export function renderInlineImage(
  buffer: Buffer,
  mimeType: string,
  protocol: "iterm2" | "kitty",
): boolean {
  if (buffer.length === 0) return false;
  if (protocol === "iterm2") {
    const b64 = buffer.toString("base64");
    process.stdout.write(
      `\x1b]1337;File=inline=1;preserveAspectRatio=1:${b64}\x07\n`,
    );
    return true;
  }
  if (protocol === "kitty") {
    if (mimeType !== "image/png") return false;
    const b64 = buffer.toString("base64");
    let offset = 0;
    let first = true;
    while (offset < b64.length) {
      const chunk = b64.slice(offset, offset + KITTY_CHUNK_SIZE);
      offset += KITTY_CHUNK_SIZE;
      const more = offset < b64.length ? 1 : 0;
      if (first) {
        process.stdout.write(`\x1b_Ga=T,f=100,q=2,m=${more};${chunk}\x1b\\`);
        first = false;
      } else {
        process.stdout.write(`\x1b_Gm=${more};${chunk}\x1b\\`);
      }
    }
    process.stdout.write("\n");
    return true;
  }
  return false;
}

export async function fetchThumbnailBytes(
  api: ApiInstance,
  entry: ThumbnailEntry,
): Promise<{ buffer: Buffer; mimeType: string } | null> {
  try {
    const response = await api.get(
      `v1alpha/fileobject/by-identity/${encodeURIComponent(entry.thumbnail.resourceIdentity)}`,
      { throwHttpErrors: false },
    );
    let raw: ArrayBuffer;
    let mimeType: string;

    if (response.status === 200 && response.body) {
      const ct = (response.headers.get("Content-Type") ?? "")
        .split(";")[0]
        .trim();
      mimeType =
        ct && ct !== "application/octet-stream"
          ? ct
          : mimeFromPath(entry.thumbnail.resourceAddress);
      raw = await response.arrayBuffer();
    } else if (response.status === 300) {
      const redirect = DownloadRedirectSchema.parse(await response.json());
      const rr = await ky.get(redirect.redirectTargetUrl, {
        headers: redirect.additionalHeaders,
      });
      mimeType =
        (rr.headers.get("Content-Type") ?? "").split(";")[0].trim() ||
        mimeFromPath(entry.thumbnail.resourceAddress);
      raw = await rr.arrayBuffer();
    } else {
      return null;
    }

    return { buffer: Buffer.from(raw), mimeType };
  } catch {
    return null;
  }
}

export async function runThumbnail(
  api: ApiInstance,
  thumbApi: ApiInstance,
  path: string,
  opts: { json: boolean },
  cwd?: string,
): Promise<void> {
  const caps = capabilitiesService(api);
  const resolved = await resolveStoragePath(caps, path, cwd);

  const { thumbnails } = await thumbnailService(thumbApi).getThumbnails(
    resolved,
    { width: THUMBNAIL_SIZE, height: THUMBNAIL_SIZE },
  );

  if (opts.json) {
    printJson(thumbnails);
    return;
  }

  if (thumbnails.length === 0) {
    process.stdout.write(`No thumbnails found for ${resolved}\n`);
    return;
  }

  const protocol = detectImageProtocol();
  const imageData = await Promise.all(
    thumbnails.map((t) => fetchThumbnailBytes(api, t)),
  );

  process.stdout.write(`Thumbnails for ${chalk.bold(resolved)}:\n\n`);
  for (let i = 0; i < thumbnails.length; i++) {
    const thumb = thumbnails[i];
    const data = imageData[i];
    process.stdout.write(
      `  ${chalk.dim("Identity:")} ${thumb.thumbnail.resourceIdentity}\n`,
    );
    process.stdout.write(
      `  ${chalk.dim("Address: ")} ${thumb.thumbnail.resourceAddress}\n`,
    );
    if (thumb.createTime) {
      process.stdout.write(`  ${chalk.dim("Created: ")} ${thumb.createTime}\n`);
    }
    if (data) {
      const rendered =
        protocol && renderInlineImage(data.buffer, data.mimeType, protocol);
      if (!rendered) {
        process.stdout.write(
          `  ${chalk.dim("URL:     ")} data:${data.mimeType};base64,${data.buffer.toString("base64")}\n`,
        );
      }
    }
    process.stdout.write("\n");
  }
  process.stdout.write(
    chalk.dim(
      `(${thumbnails.length} thumbnail${thumbnails.length !== 1 ? "s" : ""})\n`,
    ),
  );
}
