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

import ky, { HTTPError, Progress } from "ky";
import { list } from "radash";
import { z } from "zod";
import { ApiInstance } from "@/services/api";
import { MetadataSchema } from "@/services/storage/fileFolderService";
import { Batch } from "@/util/batch";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { abortError, delay, exponentialBackoff } from "@/util/sleep";

export function fileObjectService(api: ApiInstance) {
  return {
    async copy(sourceIdentity: string, destinationAddress: string, failIfExists?: boolean): Promise<CopyResponse> {
      const response = await api.post(`fileobject/by-identity/${encodeURIComponent(sourceIdentity)}/copy`, {
        body: JSON.stringify({
          destination_resource_address: destinationAddress,
          fail_if_exists: failIfExists,
        }),
        headers: {
          "Content-Type": "application/json",
        },
      });
      return CopyResponseSchema.parse(await response.json());
    },

    async move(sourceAddress: string, destinationAddress: string, failIfExists?: boolean): Promise<MoveResponse> {
      const response = await api.post(`fileobject/by-address/${encodeURIComponent(sourceAddress)}/move`, {
        body: JSON.stringify({
          destination_resource_address: destinationAddress,
          fail_if_exists: failIfExists,
        }),
        headers: {
          "Content-Type": "application/json",
        },
      });
      return MoveResponseSchema.parse(await response.json());
    },

    async delete(resourceAddress: string) {
      const response = await api.delete(`fileobject/by-address/${encodeURIComponent(resourceAddress)}`, {
        throwHttpErrors: false,
      });

      const allowedStatuses = [200, 204, 404];
      if (!allowedStatuses.includes(response.status)) {
        throw new Error(`Failed to delete "${resourceAddress}" -- HTTP${response.status}`);
      }
    },

    async download(
      resource: string,
      resourceType: "address" | "identity",
      props: {
        signal: AbortSignal;
        onProgress?: (progress: Progress, chunk: Uint8Array) => void;
      }
    ) {
      const { signal, onProgress } = props;
      let redirectInfo: DownloadRedirectResponse | null = null;

      const pathPrefix = resourceType === "address" ? "by-address" : "by-identity";
      const response = await api.get(`fileobject/${pathPrefix}/${encodeURIComponent(resource)}`, {
        throwHttpErrors: false,
        onDownloadProgress: onProgress,
        signal,
      });
      if (response.status === 200 && response.body) {
        return response;
      }
      if (response.status === 300) {
        const json = await response.json();
        redirectInfo = DownloadRedirectResponseSchema.parse(json);
        if (!redirectInfo) {
          throw new Error(`No redirect info for ${resource}`);
        }
        return ky.get(redirectInfo.redirectTargetUrl, {
          headers: redirectInfo.additionalHeaders,
          cache: "no-store",
          onDownloadProgress: onProgress,
          signal,
        });
      }
      throw new Error(`Failed to download "${resource}" -- HTTP${response.status}`);
    },

    async upload(
      resourceAddress: string,
      file: File,
      onProgress: (progress: Progress) => void,
      abortSignal: AbortSignal
    ) {
      const params = new URLSearchParams();
      params.set("data_object_size", file.size.toString());

      // The initial request determines whether the service accepts a direct upload or redirects it.
      // Hold its progress until the response is known so a redirect does not report a complete probe
      // followed by the real transfer restarting from zero.
      let lastProbeProgress: Progress | undefined;
      let response: Response;
      try {
        response = await api.put(`fileobject/by-address/${encodeURIComponent(resourceAddress)}?${params}`, {
          body: file,
          retry: 2,
          throwHttpErrors: true,
          signal: abortSignal,
          headers: {
            Expect: "100-continue",
            "Content-Type": "application/octet-stream",
          },
          onUploadProgress: (progress) => {
            lastProbeProgress = progress;
          },
        });
      } catch (error) {
        if (!(error instanceof HTTPError)) {
          throw error;
        }
        response = error.response;
      }

      if (response.status === 201) {
        if (lastProbeProgress) {
          onProgress(lastProbeProgress);
        }
        return;
      }

      if (response.status !== 300) {
        throw new Error(`Failed to upload "${resourceAddress}" -- HTTP${response.status}`);
      }

      const json = await response.json();
      let redirectInfo: UploadResponse | null = null;

      redirectInfo = UploadResponseSchema.parse(json);

      if (!redirectInfo) {
        throw new Error(`No redirect info for ${resourceAddress}`);
      }
      if (redirectInfo.redirect) {
        return uploadViaRedirect(api, resourceAddress, redirectInfo.redirect, file, onProgress, abortSignal);
      }
      if (redirectInfo.multipart) {
        return uploadMultipart({
          api,
          resourceAddress,
          multipartInfo: redirectInfo.multipart,
          file,
          onProgress,
          abortSignal,
        });
      }

      throw new Error(`Unknown upload redirect kind: ${redirectInfo as unknown as string}`);
    },

    async stat(resourceAddress: string) {
      const response = await api.head(`fileobject/by-address/${encodeURIComponent(resourceAddress)}`);
      if (response.status !== 204) {
        throw new Error(`Failed to stat "${resourceAddress}" - HTTP${response.status}`);
      }

      return {
        metadata: MetadataSchema.parse(JSON.parse(response.headers.get("X-Nvidia-Omniverse-Storage-Metadata") || "{}")),
        resourceIdentity: response.headers.get("X-Nvidia-Omniverse-Storage-Resource-Identity"),
      };
    },
  };
}

// The maximum size (in bytes) allowed for a single multipart upload chunk.
// Having more (smaller) parts allows the batch processor to upload several chunks in parallel,
// which can increase throughput on high-bandwidth / high-latency links.
const maxPartSize = 1024 * 1024 * 512; // 512 MiB

// XMLHttpRequest is used here instead of ky/fetch because ky's upload progress tracking relies on
// streaming the request body (`Request` with `duplex: "half"`), which not every browser supports --
// it throws outright rather than degrading gracefully. XHR's upload progress events have universal
// support, which matters most for exactly this kind of large, direct-to-storage transfer.
const uploadRetryLimit = 2; // matches ky's default retry limit, used for the direct upload probe
const retryableUploadStatusCodes = [408, 413, 429, 500, 502, 503, 504]; // matches ky's default retryable statuses

class UploadStatusError extends Error {
  constructor(
    message: string,
    public readonly status: number
  ) {
    super(message);
    this.name = "UploadStatusError";
  }
}

// XHR reports a status of 0 for a network-level failure (e.g. connection dropped), so treat that as
// retryable alongside the transient HTTP statuses above.
function isRetryableUploadError(error: unknown): boolean {
  return (
    error instanceof UploadStatusError && (error.status === 0 || retryableUploadStatusCodes.includes(error.status))
  );
}

async function uploadToRedirect(
  redirect: RedirectResponse,
  body: Blob,
  onProgress: (progress: Progress) => void,
  abortSignal: AbortSignal
): Promise<Header[]> {
  let attempt = 0;
  for (;;) {
    try {
      return await sendToRedirect(redirect, body, onProgress, abortSignal);
    } catch (error) {
      if (attempt >= uploadRetryLimit || abortSignal.aborted || !isRetryableUploadError(error)) {
        throw error;
      }
      attempt += 1;
      // Mirrors ky's default backoff curve so a redirected/multipart part retries on the same
      // schedule as the direct upload probe.
      await delay(exponentialBackoff(attempt), abortSignal);
    }
  }
}

function sendToRedirect(
  redirect: RedirectResponse,
  body: Blob,
  onProgress: (progress: Progress) => void,
  abortSignal: AbortSignal
): Promise<Header[]> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const onAbort = () => xhr.abort();

    xhr.upload.addEventListener("progress", (event) => {
      if (event.lengthComputable) {
        onProgress({
          percent: event.loaded / event.total,
          transferredBytes: event.loaded,
          totalBytes: event.total,
        });
      }
    });

    xhr.addEventListener("loadend", () => {
      abortSignal.removeEventListener("abort", onAbort);
      if (abortSignal.aborted) {
        reject(abortError(abortSignal));
        return;
      }
      if (xhr.readyState === 4 && xhr.status >= 200 && xhr.status < 300) {
        const headersList: Header[] = [];
        for (const headerName of redirect.completionHeaderNames) {
          const headerValue = xhr.getResponseHeader(headerName);
          if (headerValue !== null) {
            headersList.push({ name: headerName, value: headerValue });
          }
        }
        resolve(headersList);
      } else {
        reject(new UploadStatusError(`Failed to upload via redirect -- HTTP${xhr.status}`, xhr.status));
      }
    });

    abortSignal.addEventListener("abort", onAbort);

    xhr.open(redirect.method.toUpperCase(), redirect.redirectTargetUrl, true);
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    for (const { name, value } of redirect.additionalHeaders) {
      xhr.setRequestHeader(name, value);
    }

    xhr.send(body);
  });
}

async function uploadViaRedirect(
  api: ApiInstance,
  resourceAddress: string,
  redirect: RedirectResponse,
  file: File,
  onProgress: (progress: Progress) => void,
  abortSignal: AbortSignal
): Promise<boolean> {
  const completionHeaders = await uploadToRedirect(redirect, file, onProgress, abortSignal);

  const response = await api.post(`fileobject/by-address/${encodeURIComponent(resourceAddress)}/redirect/complete`, {
    json: {
      additional_headers: completionHeaders,
    },
    timeout: false,
    headers: { "Content-Type": "application/json" },
  });

  if (response.status !== 200) {
    throw new Error(`Failed to complete redirect upload -- HTTP${response.status}`);
  }

  return true;
}

interface UploadMultipartParams {
  api: ApiInstance;
  resourceAddress: string;
  multipartInfo: NonNullable<UploadResponse["multipart"]>;
  file: File;
  onProgress: (progress: Progress) => void;
  abortSignal: AbortSignal;
}

async function uploadMultipart({
  api,
  resourceAddress,
  multipartInfo,
  file,
  onProgress,
  abortSignal,
}: UploadMultipartParams): Promise<boolean> {
  const { uploadId, firstPartWriteRedirect, minimumSizePerPart, maximumSizePerPart, maximumPartsNumber } =
    multipartInfo;

  const selectPartSize = () => {
    let partSize = Math.min(maximumSizePerPart, maxPartSize);

    if (minimumSizePerPart && partSize < minimumSizePerPart) {
      partSize = minimumSizePerPart;
    }

    const parts = Math.ceil(file.size / partSize);
    if (parts > maximumPartsNumber) {
      throw new Error(
        `Cannot upload file: would require ${parts} parts at <= ${partSize} bytes each, but service allows only ${maximumPartsNumber}`
      );
    }
    return { partSize, parts };
  };

  const { partSize, parts: totalParts } = selectPartSize();

  const redirects: RedirectResponse[] = [firstPartWriteRedirect];

  if (totalParts > 1) {
    const prepareRes = await api.post(
      `fileobject/by-address/${encodeURIComponent(resourceAddress)}/multipart/prepare`,
      {
        json: {
          upload_id: uploadId,
          part_number: 1,
          part_count: totalParts - 1,
        },
        timeout: false,
        headers: { "Content-Type": "application/json" },
      }
    );

    if (prepareRes.status !== 200) {
      throw new Error(`Failed to prepare multipart upload -- HTTP${prepareRes.status}`);
    }

    const prepareResponse = UploadPrepareResponseSchema.parse(await prepareRes.json());
    redirects.push(...prepareResponse.partWriteRedirects);
  }

  const completedParts: CompletedPart[] = [];

  // Track per-part progress so that we can aggregate progress updates coming from parallel uploads.
  const partProgress = new Map<number, number>();
  let transferredBytes = 0;

  const updateAggregatedProgress = (partNumber: number, loaded: number) => {
    const prevLoaded = partProgress.get(partNumber) ?? 0;
    if (loaded <= prevLoaded) {
      return;
    }
    partProgress.set(partNumber, loaded);
    transferredBytes += loaded - prevLoaded;

    onProgress({
      percent: transferredBytes / file.size,
      transferredBytes,
      totalBytes: file.size,
    });
  };

  interface PartUploadResult {
    partNumber: number;
    completionHeaders: { name: string; value: string }[];
  }

  interface PartUploadItem {
    partNumber: number;
  }

  const uploadPart = async ({ partNumber }: PartUploadItem): Promise<PartUploadResult> => {
    const start = (partNumber - 1) * partSize;
    const end = Math.min(start + partSize, file.size);
    const chunk = file.slice(start, end);
    const redirect = redirects[partNumber - 1];

    const completionHeaders = await uploadToRedirect(
      redirect,
      chunk,
      (progress) => updateAggregatedProgress(partNumber, progress.transferredBytes),
      abortSignal
    );
    updateAggregatedProgress(partNumber, chunk.size);

    return { partNumber, completionHeaders };
  };

  // `Batch` intentionally suppresses whatever `onItemError` throws (see its own error-callback
  // handling), so a permanently failed part can't propagate by throwing here -- capture it instead
  // and rethrow once the batch settles, rather than silently completing with a partial part list.
  let firstPartError: Error | undefined;

  const batch = new Batch<PartUploadItem, PartUploadResult, Error>(uploadPart, {
    signal: abortSignal,
    batchSize: 10,
    onItemSuccess: (_item, result) => {
      completedParts.push({
        part_number: result.partNumber - 1,
        additional_headers: result.completionHeaders,
      });
    },
    onItemError: (_item, error) => {
      firstPartError ??= error;
    },
  });

  await batch.add(list(1, totalParts, (i: number) => ({ partNumber: i })));
  await batch.flush();

  if (firstPartError) {
    throw firstPartError;
  }

  const response = await api.post(`fileobject/by-address/${encodeURIComponent(resourceAddress)}/multipart/complete`, {
    json: {
      upload_id: uploadId,
      parts: completedParts,
    },
    timeout: false,
    headers: { "Content-Type": "application/json" },
  });

  if (response.status !== 200) {
    throw new Error(`Failed to complete multipart upload -- HTTP${response.status}`);
  }

  return true;
}

interface Header {
  name: string;
  value: string;
}

interface CompletedPart {
  part_number: number;
  additional_headers: Header[];
}

export const DownloadRedirectResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirectTargetUrl: z.string(),
    additionalHeaders: z.record(z.string(), z.string()),
  })
);
export type DownloadRedirectResponse = z.infer<typeof DownloadRedirectResponseSchema>;

export const CopyResponseSchema = fromSnakeCaseSchema(
  z.object({
    resourceIdentity: z.string(),
  })
);
export type CopyResponse = z.infer<typeof CopyResponseSchema>;

export const MoveResponseSchema = fromSnakeCaseSchema(
  z.object({
    resourceIdentity: z.string(),
  })
);
export type MoveResponse = z.infer<typeof MoveResponseSchema>;

const RedirectResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirectTargetUrl: z.string(),
    additionalHeaders: z.array(z.object({ name: z.string(), value: z.string() })),
    method: z.string(),
    completionHeaderNames: z.array(z.string()),
  })
);

export type RedirectResponse = z.infer<typeof RedirectResponseSchema>;

const UploadPrepareResponseSchema = fromSnakeCaseSchema(
  z.object({
    partWriteRedirects: z.array(RedirectResponseSchema),
  })
);

export const UploadResponseSchema = fromSnakeCaseSchema(
  z.object({
    redirect: fromSnakeCaseSchema(
      z.object({
        redirectTargetUrl: z.string(),
        additionalHeaders: z.array(z.object({ name: z.string(), value: z.string() })),
        method: z.string(),
        completionHeaderNames: z.array(z.string()),
      })
    ).optional(),
    multipart: fromSnakeCaseSchema(
      z.object({
        uploadId: z.string(),
        firstPartWriteRedirect: RedirectResponseSchema,
        minimumSizePerPart: z.number().int().nonnegative(),
        maximumSizePerPart: z.number().int().positive(),
        maximumPartsNumber: z.number().int().positive(),
      })
    ).optional(),
  })
);

export type UploadResponse = z.infer<typeof UploadResponseSchema>;
