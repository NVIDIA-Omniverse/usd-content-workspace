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

import { z } from "zod";
import { getDiscoveredApi } from "@/context";
import { type ApiInstance } from "@/services/api";
import { fromSnakeCaseSchema } from "@/util/schemas";

const PathMetadataSchema = fromSnakeCaseSchema(
  z.object({
    acl: z
      .object({ etag: z.string(), value: z.array(z.enum(["read", "write"])) })
      .default({ etag: "", value: ["read", "write"] }),
    tags: z
      .object({ etag: z.string(), value: z.record(z.string()) })
      .default({ etag: "", value: {} }),
    thumbnails: z.array(z.unknown()).default([]),
  }),
);
export type PathMetadata = z.infer<typeof PathMetadataSchema>;

export function metadataService(api: ApiInstance) {
  return {
    async get(resourceAddress: string): Promise<PathMetadata> {
      if ((await getDiscoveredApi(api, "metadata"))?.version === "v1beta") {
        const result: Record<string, unknown> = {};
        let continuationHandle: string | null | undefined;
        do {
          const response = await api.post("v1alpha/metadata/query", {
            body: JSON.stringify({
              resource_location: { resource_address: resourceAddress },
              ...(continuationHandle
                ? { continuation_handle: continuationHandle }
                : {}),
            }),
          });
          const page = QueryMetadataResponseSchema.parse(await response.json());
          for (const entry of page.results) {
            if (entry.user_metadata)
              result[entry.user_metadata_key] = entry.user_metadata;
          }
          continuationHandle = page.next_continuation_handle;
        } while (continuationHandle);
        return PathMetadataSchema.parse(result);
      }
      const response = await api.post(
        `v1alpha/metadata/${encodeURIComponent(resourceAddress)}`,
        {
          body: JSON.stringify([]),
        },
      );
      return PathMetadataSchema.parse(await response.json());
    },

    async setTags(
      resourceAddress: string,
      tags: Record<string, string>,
      etag: string,
    ): Promise<void> {
      if ((await getDiscoveredApi(api, "metadata"))?.version === "v1beta") {
        const response = await api.put("v1alpha/metadata", {
          body: JSON.stringify({
            requests: [
              {
                resource_location: { resource_address: resourceAddress },
                user_metadata_key: "tags",
                user_metadata: tags,
                ...(etag ? { expected_etag: etag } : {}),
              },
            ],
          }),
        });
        const results = BatchResponseSchema.parse(
          await response.json(),
        ).results;
        for (const { outcome } of results) {
          const error = outcome?.Error;
          if (error) {
            const detail = BatchErrorSchema.parse(error);
            throw new Error(
              `Failed to update metadata: ${detail.code ?? "unknown"}${detail.message ? ` - ${detail.message}` : ""}`,
            );
          }
        }
        return;
      }
      const headers = new Headers();
      if (etag) {
        headers.set("If-Match", etag);
      }
      await api.put(
        `v1alpha/metadata/${encodeURIComponent(resourceAddress)}/tags`,
        {
          body: JSON.stringify(tags),
          headers,
        },
      );
    },
  };
}

const QueryMetadataResponseSchema = z.object({
  results: z
    .array(
      z.object({
        user_metadata_key: z.string(),
        user_metadata: z
          .object({ etag: z.string(), value: z.unknown() })
          .nullish(),
      }),
    )
    .default([]),
  next_continuation_handle: z.string().nullish(),
});

const BatchResponseSchema = z.object({
  results: z
    .array(z.object({ outcome: z.record(z.string(), z.unknown()).nullish() }))
    .default([]),
});

const BatchErrorSchema = z.object({
  code: z.union([z.string(), z.number()]).optional(),
  message: z.string().optional(),
});
