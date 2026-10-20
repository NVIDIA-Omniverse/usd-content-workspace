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

import { createReadStream, createWriteStream, statSync } from "fs";
import { link, rename, unlink } from "fs/promises";
import { randomUUID } from "crypto";
import { dirname, join } from "path";
import { Readable, Transform } from "stream";
import { pipeline } from "stream/promises";
import ky from "ky";
import { z } from "zod";
import { forEachWithConcurrency } from "@/util/concurrency";
import { type ApiInstance } from "@/services/api";
import {
  CliError,
  noopLogger,
  type Logger,
  type ProgressCallback,
} from "@/output";
import { ensureDownloadDirectory } from "@/util/downloadPath";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { MetadataSchema } from "./fileFolderService";

function progressTransform(onChunk: (n: number) => void): Transform {
  return new Transform({
    transform(chunk, _enc, cb) {
      onChunk((chunk as Buffer).length);
      cb(null, chunk);
    },
  });
}

interface DownloadOptions {
  overwrite?: boolean;
  /** Canonical user-selected base for validating remote-derived directories. */
  root?: string;
}

async function streamToFile(
  src: Readable,
  localPath: string,
  contentLength: number,
  onProgress?: ProgressCallback,
  options: DownloadOptions = {},
): Promise<void> {
  const total = contentLength;
  let bytesTransferred = 0;
  const startMs = Date.now();
  if (options.root) ensureDownloadDirectory(options.root, dirname(localPath));
  const temporary = join(dirname(localPath), `.sn-${randomUUID()}`);
  const dest = createWriteStream(temporary, { flags: "wx" });
  try {
    if (onProgress) {
      const transform = progressTransform((n) => {
        bytesTransferred += n;
        const speed =
          bytesTransferred / Math.max((Date.now() - startMs) / 1000, 0.001);
        onProgress(bytesTransferred, total, speed);
      });
      await pipeline(src, transform, dest);
      if (total === 0 && bytesTransferred > 0) {
        const speed =
          bytesTransferred / Math.max((Date.now() - startMs) / 1000, 0.001);
        onProgress(bytesTransferred, bytesTransferred, speed);
      }
    } else {
      await pipeline(src, dest);
    }
    if (options.root) ensureDownloadDirectory(options.root, dirname(localPath));
    if (options.overwrite) await rename(temporary, localPath);
    else await link(temporary, localPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST")
      throw new CliError(
        `File already exists: ${localPath} (use --overwrite to replace it)`,
        "CONFLICT",
      );
    throw error;
  } finally {
    await unlink(temporary).catch(() => undefined);
  }
}

const CopyResponseSchema = fromSnakeCaseSchema(
  z.object({ resourceIdentity: z.string() }),
);
type CopyResponse = z.infer<typeof CopyResponseSchema>;

const MoveResponseSchema = fromSnakeCaseSchema(
  z.object({ resourceIdentity: z.string() }),
);
type MoveResponse = z.infer<typeof MoveResponseSchema>;

const DownloadRedirectResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirectTargetUrl: z.string(),
    additionalHeaders: z.record(z.string(), z.string()),
  }),
);

const HeaderEntrySchema = z.object({ name: z.string(), value: z.string() });

const RedirectResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirectTargetUrl: z.string(),
    method: z.string(),
    additionalHeaders: z.array(HeaderEntrySchema),
    completionHeaderNames: z.array(z.string()),
  }),
);
type RedirectResponse = z.infer<typeof RedirectResponseSchema>;

const UploadResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirect: RedirectResponseSchema.optional(),
    multipart: fromSnakeCaseSchema(
      z.object({
        uploadId: z.string(),
        firstPartWriteRedirect: RedirectResponseSchema,
        minimumSizePerPart: z.number().int().nonnegative(),
        maximumSizePerPart: z.number().int().positive(),
        maximumPartsNumber: z.number().int().positive(),
      }),
    ).optional(),
  }),
);

const UploadPrepareResponseSchema = fromSnakeCaseSchema(
  z.object({
    partWriteRedirects: z.array(RedirectResponseSchema),
  }),
);

export function fileObjectService(
  api: ApiInstance,
  log: Logger = noopLogger(),
) {
  return {
    async stat(resourceAddress: string) {
      const response = await api.head(
        `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}`,
        {
          throwHttpErrors: false,
        },
      );
      if (response.status !== 204) {
        throw new Error(
          `Failed to stat "${resourceAddress}" — HTTP${response.status}`,
        );
      }
      return {
        metadata: MetadataSchema.parse(
          JSON.parse(
            response.headers.get("X-Nvidia-Omniverse-Storage-Metadata") ?? "{}",
          ),
        ),
        resourceIdentity: response.headers.get(
          "X-Nvidia-Omniverse-Storage-Resource-Identity",
        ),
      };
    },

    async delete(resourceAddress: string): Promise<void> {
      const response = await api.delete(
        `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}`,
        {
          throwHttpErrors: false,
        },
      );
      if (![200, 204, 404].includes(response.status)) {
        throw new Error(
          `Failed to delete "${resourceAddress}" — HTTP${response.status}`,
        );
      }
    },

    async move(
      sourceAddress: string,
      destinationAddress: string,
      failIfExists?: boolean,
    ): Promise<MoveResponse> {
      const response = await api.post(
        `v1alpha/fileobject/by-address/${encodeURIComponent(sourceAddress)}/move`,
        {
          body: JSON.stringify({
            destination_resource_address: destinationAddress,
            ...(failIfExists !== undefined
              ? { fail_if_exists: failIfExists }
              : {}),
          }),
          headers: { "Content-Type": "application/json" },
        },
      );
      return MoveResponseSchema.parse(await response.json());
    },

    async copy(
      sourceIdentity: string,
      destinationAddress: string,
      failIfExists?: boolean,
    ): Promise<CopyResponse> {
      const response = await api.post(
        `v1alpha/fileobject/by-identity/${encodeURIComponent(sourceIdentity)}/copy`,
        {
          body: JSON.stringify({
            destination_resource_address: destinationAddress,
            ...(failIfExists !== undefined
              ? { fail_if_exists: failIfExists }
              : {}),
          }),
          headers: { "Content-Type": "application/json" },
        },
      );
      return CopyResponseSchema.parse(await response.json());
    },

    async download(
      resourceAddress: string,
      localPath: string,
      onProgress?: ProgressCallback,
      options: DownloadOptions = {},
    ): Promise<void> {
      log("download", `${resourceAddress} → ${localPath}`);
      const response = await api.get(
        `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}`,
        {
          throwHttpErrors: false,
        },
      );

      if (response.status === 200 && response.body) {
        const total = Number(response.headers.get("Content-Length") ?? 0);
        const src = Readable.fromWeb(response.body as ReadableStream);
        await streamToFile(src, localPath, total, onProgress, options);
        log("download", "complete");
        return;
      }

      if (response.status === 300) {
        log("download", "using redirect");
        const redirect = DownloadRedirectResponseSchema.parse(
          await response.json(),
        );
        const redirectResponse = await ky.get(redirect.redirectTargetUrl, {
          headers: redirect.additionalHeaders,
        });
        const total = Number(
          redirectResponse.headers.get("Content-Length") ?? 0,
        );
        const src = Readable.fromWeb(redirectResponse.body as ReadableStream);
        await streamToFile(src, localPath, total, onProgress, options);
        log("download", "complete");
        return;
      }

      throw new Error(
        `Failed to download "${resourceAddress}" — HTTP${response.status}`,
      );
    },

    async upload(
      localPath: string,
      resourceAddress: string,
      onProgress?: ProgressCallback,
    ): Promise<void> {
      const { size } = statSync(localPath);
      const params = new URLSearchParams({ data_object_size: size.toString() });
      log(
        "upload",
        `${localPath} → ${resourceAddress} (${(size / 1024 / 1024).toFixed(1)} MB)`,
      );

      let bytesTransferred = 0;
      const startMs = Date.now();
      const onBytes = onProgress
        ? (n: number) => {
            bytesTransferred += n;
            const speed =
              bytesTransferred / Math.max((Date.now() - startMs) / 1000, 0.001);
            onProgress(bytesTransferred, size, speed);
          }
        : undefined;

      const makeReadStream = () => {
        const rs = createReadStream(localPath);
        return onBytes ? rs.pipe(progressTransform(onBytes)) : rs;
      };

      const response = await api.put(
        `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}?${params.toString()}`,
        {
          // If the server responds 300, the redirect/multipart helpers open a
          // second read stream with the same onBytes closure, so bytesTransferred
          // may be double-counted on those paths.
          body: Readable.toWeb(makeReadStream()) as ReadableStream,
          throwHttpErrors: false,
          timeout: false,
          headers: { "Content-Type": "application/octet-stream" },
        },
      );

      if (response.status === 201) {
        log("upload", "using direct upload");
        log("upload", "complete");
        return;
      }

      if (response.status === 300) {
        const json = UploadResponseSchema.parse(await response.json());
        if (json.redirect) {
          log("upload", "using redirect upload");
          await uploadViaSingleRedirect(localPath, json.redirect, onBytes);
          log("upload", "complete");
          return;
        }
        if (json.multipart) {
          await uploadMultipart(
            api,
            localPath,
            resourceAddress,
            json.multipart,
            log,
            onBytes,
          );
          log("upload", "complete");
          return;
        }
        throw new Error(`Unknown upload redirect kind from server.`);
      }

      throw new Error(
        `Failed to upload "${localPath}" — HTTP${response.status}`,
      );
    },
  };
}

async function uploadViaSingleRedirect(
  localPath: string,
  redirect: RedirectResponse,
  onBytes?: (n: number) => void,
): Promise<void> {
  const { size } = statSync(localPath);
  const headers: Record<string, string> = {
    "Content-Type": "application/octet-stream",
    "Content-Length": String(size),
  };

  for (const { name, value } of redirect.additionalHeaders) {
    headers[name] = value;
  }

  const rs = createReadStream(localPath);
  const body = Readable.toWeb(
    onBytes ? rs.pipe(progressTransform(onBytes)) : rs,
  ) as ReadableStream;

  const res = await ky(redirect.redirectTargetUrl, {
    method: redirect.method.toUpperCase(),
    body,
    headers,
    throwHttpErrors: false,
    timeout: false,
  });
  if (!res.ok) {
    throw new Error(`Redirect upload failed — HTTP${res.status}`);
  }
}

const MAX_PART_SIZE = 1024 * 1024 * 512; // 512 MiB
const PART_CONCURRENCY = 10;

async function uploadMultipart(
  api: ApiInstance,
  localPath: string,
  resourceAddress: string,
  multipartInfo: NonNullable<z.infer<typeof UploadResponseSchema>["multipart"]>,
  log: Logger,
  onBytes?: (n: number) => void,
): Promise<void> {
  const {
    uploadId,
    firstPartWriteRedirect,
    minimumSizePerPart,
    maximumSizePerPart,
    maximumPartsNumber,
  } = multipartInfo;
  const { size } = statSync(localPath);

  let partSize = Math.min(maximumSizePerPart, MAX_PART_SIZE);
  if (minimumSizePerPart && partSize < minimumSizePerPart) {
    partSize = minimumSizePerPart;
  }
  const totalParts = Math.ceil(size / partSize);
  log(
    "upload",
    `using multipart upload (${totalParts} parts, ${(partSize / 1024 / 1024).toFixed(1)} MB each)`,
  );
  if (totalParts > maximumPartsNumber) {
    throw new Error(
      `Cannot upload: ${totalParts} parts needed but server allows only ${maximumPartsNumber}`,
    );
  }

  // Fetch write redirects for parts 2..N
  const redirects: RedirectResponse[] = [firstPartWriteRedirect];
  if (totalParts > 1) {
    const prepareRes = await api.post(
      `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}/multipart/prepare`,
      {
        body: JSON.stringify({
          upload_id: uploadId,
          part_number: 1,
          part_count: totalParts - 1,
        }),
        headers: { "Content-Type": "application/json" },
        throwHttpErrors: false,
      },
    );
    if (prepareRes.status !== 200) {
      throw new Error(
        `Failed to prepare multipart upload — HTTP${prepareRes.status}`,
      );
    }
    const prep = UploadPrepareResponseSchema.parse(await prepareRes.json());
    redirects.push(...prep.partWriteRedirects);
  }

  // Upload all parts in parallel batches
  interface CompletedPart {
    part_number: number;
    additional_headers: { name: string; value: string }[];
  }
  const completedParts: CompletedPart[] = [];

  for (let i = 0; i < totalParts; i += PART_CONCURRENCY) {
    const batch = Array.from(
      { length: Math.min(PART_CONCURRENCY, totalParts - i) },
      (_, j) => i + j + 1, // 1-indexed part numbers
    );
    await forEachWithConcurrency(
      batch,
      PART_CONCURRENCY,
      async (partNumber) => {
        const start = (partNumber - 1) * partSize;
        const end = Math.min(start + partSize, size);
        const redirect = redirects[partNumber - 1];

        const headers: Record<string, string> = {
          "Content-Type": "application/octet-stream",
          "Content-Length": String(end - start),
        };

        for (const { name, value } of redirect.additionalHeaders) {
          headers[name] = value;
        }

        const res = await ky(redirect.redirectTargetUrl, {
          method: redirect.method.toUpperCase(),
          body: (() => {
            const rs = createReadStream(localPath, { start, end: end - 1 });
            return Readable.toWeb(
              onBytes ? rs.pipe(progressTransform(onBytes)) : rs,
            ) as ReadableStream;
          })(),
          headers,
          throwHttpErrors: false,
          timeout: false,
        });
        if (!res.ok) {
          throw new Error(
            `Failed to upload part ${partNumber} — HTTP${res.status}`,
          );
        }

        const completionHeaders = redirect.completionHeaderNames.flatMap(
          (name) => {
            const value = res.headers.get(name);
            return value ? [{ name, value }] : [];
          },
        );

        log("upload", `part ${partNumber}/${totalParts} done`);
        completedParts.push({
          part_number: partNumber - 1,
          additional_headers: completionHeaders,
        });
      },
    );
  }

  // Sort parts by part_number before completing (batches may resolve out of order)
  completedParts.sort((a, b) => a.part_number - b.part_number);

  const completeRes = await api.post(
    `v1alpha/fileobject/by-address/${encodeURIComponent(resourceAddress)}/multipart/complete`,
    {
      body: JSON.stringify({ upload_id: uploadId, parts: completedParts }),
      headers: { "Content-Type": "application/json" },
      throwHttpErrors: false,
    },
  );
  if (completeRes.status !== 200) {
    throw new Error(
      `Failed to complete multipart upload — HTTP${completeRes.status}`,
    );
  }
}
