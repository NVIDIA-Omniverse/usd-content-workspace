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
import { ResourceInfoItemSchema } from "@/services/storage/fileFolderService";
import { fromSnakeCaseSchema } from "@/util/schemas";

export function versioningService(api: ApiInstance) {
  async function list({
    resourceAddress,
    continuationHandle,
    pageSize = 100,
  }: VersionListOptions): Promise<VersionPage> {
    const params = new URLSearchParams();
    params.set("max_page_size", String(pageSize));
    if (continuationHandle) {
      params.set("continuation_handle", continuationHandle);
    }

    const response = await api.get(`versioning/${encodeURIComponent(resourceAddress)}/versions?${params}`);
    const json = await response.json();
    return VersionPageSchema.parse(json);
  }

  async function listAll(resourceAddress: string): Promise<VersionPage> {
    // Use a larger page size for full-version loads to reduce pagination round trips.
    const allVersionsPageSize = 1000;
    const firstPage = await list({
      resourceAddress,
      continuationHandle: null,
      pageSize: allVersionsPageSize,
    });
    const items = [...firstPage.items];
    let nextContinuationHandle = firstPage.nextContinuationHandle;

    while (nextContinuationHandle) {
      const page = await list({
        resourceAddress,
        continuationHandle: nextContinuationHandle,
        pageSize: allVersionsPageSize,
      });
      items.push(...page.items);
      nextContinuationHandle = page.nextContinuationHandle;
    }

    return {
      ...firstPage,
      items,
      nextContinuationHandle: null,
    };
  }

  return {
    list,
    listAll,
  };
}

interface VersionListOptions {
  resourceAddress: string;
  continuationHandle: string | null;
  pageSize?: number;
}

const ResourceInfoSchema = fromSnakeCaseSchema(
  z.object({
    resourceInfo: ResourceInfoItemSchema,
  })
);

const VersionPageSchema = fromSnakeCaseSchema(
  z.object({
    items: z.array(ResourceInfoSchema),
    nextContinuationHandle: z.string().nullable(),
    versionsOrder: z.enum(["newest_first", "oldest_first", "by_key", "unspecified"]).optional(),
  })
);

export type VersionPage = z.infer<typeof VersionPageSchema>;
