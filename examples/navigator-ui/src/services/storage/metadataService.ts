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
import { ApiInstance } from "@/services/api";
import { fromSnakeCaseSchema } from "@/util/schemas";

// Key the tag map is stored under in both metadata API versions.
const tagsKey = "tags";

/**
 * The storage service dual-serves metadata through two REST APIs with different shapes:
 *
 * - `v1alpha` exposes single-resource routes: `POST metadata/{uri}` returns every key of one
 *   resource and `PUT metadata/{uri}/{key}` updates one key, guarded by an `If-Match` header.
 * - `v1beta` is batch only. There is no "all keys" batch get (`user_metadata_key` is required per
 *   entry), so reading every key of one resource goes through `POST metadata/query`, and updating a
 *   key goes through `PUT metadata` with the expected ETag carried in the request entry.
 *
 * Which one is used is decided by discovery (see `useServiceApi`), which prefers `v1beta` and falls
 * back to `v1alpha` on deployments that do not advertise it. Both branches are normalized to the
 * same `PathMetadata` result so callers never have to care which version served the data.
 */
export type MetadataApi = ApiInstance & { apiVersion?: string };

export function metadataService(api: MetadataApi) {
  const client = api.apiVersion === "v1beta" ? v1betaClient(api) : v1alphaClient(api);

  return {
    async get(resourceAddress: string): Promise<PathMetadata> {
      return PathMetadataSchema.parse(transformMetadata(await client.getAll(resourceAddress)));
    },
    async setTags(resourceAddress: string, tags: Record<string, string>, etag: string): Promise<void> {
      await client.setTags(resourceAddress, tags, etag);
    },
  };
}

interface MetadataClient {
  // Every metadata key of one resource, as a flat map of key to `{ etag, value }`. Validation of
  // the individual entries is left to `PathMetadataSchema`, which already tolerates unknown keys.
  getAll: (resourceAddress: string) => Promise<Record<string, unknown>>;
  setTags: (resourceAddress: string, tags: Record<string, string>, etag: string) => Promise<void>;
}

function v1alphaClient(api: MetadataApi): MetadataClient {
  return {
    async getAll(resourceAddress) {
      const response = await api.post(`metadata/${encodeURIComponent(resourceAddress)}`, {
        // Request all tags since wildcards are not supported yet.
        body: JSON.stringify([]),
      });
      const json: Record<string, unknown> = await response.json();
      return json;
    },
    async setTags(resourceAddress, tags, etag) {
      const headers = new Headers();
      if (etag) {
        headers.set("If-Match", etag);
      }

      await api.put(`metadata/${encodeURIComponent(resourceAddress)}/${tagsKey}`, {
        body: JSON.stringify(tags),
        headers,
      });
    },
  };
}

function v1betaClient(api: MetadataApi): MetadataClient {
  return {
    async getAll(resourceAddress) {
      const result: Record<string, unknown> = {};
      let continuationHandle: string | null | undefined;

      // QueryMetadata is paginated, so keep reading until the service stops handing back a handle.
      do {
        const response = await api.post("metadata/query", {
          body: JSON.stringify({
            resource_location: { resource_address: resourceAddress },
            ...(continuationHandle ? { continuation_handle: continuationHandle } : {}),
          }),
        });
        const page = QueryMetadataResponseSchema.parse(await response.json());

        for (const entry of page.results) {
          if (entry.user_metadata) {
            result[entry.user_metadata_key] = entry.user_metadata;
          }
        }
        continuationHandle = page.next_continuation_handle;
      } while (continuationHandle);

      return result;
    },
    async setTags(resourceAddress, tags, etag) {
      const response = await api.put("metadata", {
        body: JSON.stringify({
          requests: [
            {
              resource_location: { resource_address: resourceAddress },
              user_metadata_key: tagsKey,
              user_metadata: tags,
              // The batch route carries the precondition per entry instead of in an If-Match header.
              ...(etag ? { expected_etag: etag } : {}),
            },
          ],
        }),
      });

      throwOnBatchFailure(await response.json(), "update metadata");
    },
  };
}

/**
 * The batch routes answer HTTP 200 even when an entry failed - an ETag conflict comes back as a
 * `failed_precondition` outcome rather than the HTTP 412 the v1alpha route returns. Callers such as
 * `useSetTagsMutation` rely on a rejected promise to surface the failure, so turn the per-entry
 * error back into a thrown error.
 */
function throwOnBatchFailure(body: unknown, operation: string): void {
  const { results } = BatchResponseSchema.parse(body);

  for (const { outcome } of results) {
    const error = outcome && "Error" in outcome ? BatchErrorSchema.safeParse(outcome.Error) : null;
    if (error?.success) {
      const code = error.data.code ?? "unknown";
      throw new Error(`Failed to ${operation}: ${code}${error.data.message ? ` - ${error.data.message}` : ""}`);
    }
  }
}

function transformMetadata(metadata: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  const thumbnails: unknown[] = [];

  for (const [key, value] of Object.entries(metadata)) {
    if (key.startsWith("thumbnail_") && typeof value === "object" && value !== null && "value" in value) {
      thumbnails.push(value.value);
    } else {
      result[key] = value;
    }
  }
  result.thumbnails = thumbnails;
  return result;
}

const MetadataEntrySchema = z.object({
  etag: z.string(),
  value: z.unknown(),
});

const QueryMetadataResponseSchema = z.object({
  results: z
    .array(
      z.object({
        user_metadata_key: z.string(),
        user_metadata: MetadataEntrySchema.nullish(),
      })
    )
    .default([]),
  next_continuation_handle: z.string().nullish(),
});

const BatchResponseSchema = z.object({
  results: z
    .array(
      z.object({
        request_index: z.number().optional(),
        outcome: z.record(z.string(), z.unknown()).nullish(),
      })
    )
    .default([]),
});

// `code` is documented as an integer but the service serializes the status as a string
// (e.g. "failed_precondition"), so accept either.
const BatchErrorSchema = z.object({
  code: z.union([z.string(), z.number()]).optional(),
  message: z.string().optional(),
});

const PathMetadataSchema = fromSnakeCaseSchema(
  z.object({
    acl: z
      .object({
        etag: z.string(),
        value: z.array(z.enum(["read", "write"])),
      })
      .default({ etag: "", value: ["read", "write"] }),
    tags: z
      .object({
        etag: z.string(),
        value: z.record(z.string()),
      })
      .default({ etag: "", value: {} }),
    // The thumbnail_* entries here are the raw thumbnail-generation task metadata,
    // whose shape is not consumed by the UI (actual thumbnails are fetched separately
    // via thumbnailService/usePathThumbnail). Keep this permissive so an unrelated
    // thumbnail metadata shape change can't fail the whole metadata parse and silently
    // wipe out valid acl/tags data.
    thumbnails: z.array(z.unknown()).default([]),
  })
);

export type PathMetadata = z.infer<typeof PathMetadataSchema>;
