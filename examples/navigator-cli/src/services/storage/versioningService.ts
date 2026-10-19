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
import { type ApiInstance } from "@/services/api";
import { ResourceInfoItemSchema } from "@/services/storage/fileFolderService";
import { fromSnakeCaseSchema } from "@/util/schemas";

const VersionItemSchema = fromSnakeCaseSchema(
  z.object({
    resourceInfo: ResourceInfoItemSchema,
  }),
);

const VersionPageSchema = fromSnakeCaseSchema(
  z.object({
    items: z.array(VersionItemSchema),
    nextContinuationHandle: z.string().nullable(),
  }),
);

export type VersionPage = z.infer<typeof VersionPageSchema>;
export type VersionItem = z.infer<typeof VersionItemSchema>;

export function versioningService(api: ApiInstance) {
  return {
    async list(
      resourceAddress: string,
      continuationHandle: string | null,
    ): Promise<VersionPage> {
      const params = new URLSearchParams({ max_page_size: "100" });
      if (continuationHandle) {
        params.set("continuation_handle", continuationHandle);
      }
      const response = await api.get(
        `v1alpha/versioning/${encodeURIComponent(resourceAddress)}/versions?${params.toString()}`,
      );
      return VersionPageSchema.parse(await response.json());
    },

    async listAll(resourceAddress: string): Promise<VersionItem[]> {
      const items: VersionItem[] = [];
      let handle: string | null = null;
      do {
        const page = await this.list(resourceAddress, handle);
        items.push(...page.items);
        handle = page.nextContinuationHandle;
      } while (handle !== null);
      return items;
    },
  };
}
