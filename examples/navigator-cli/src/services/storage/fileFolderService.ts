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
import { isHttpError, type ApiInstance } from "@/services/api";
import { noopLogger, type Logger } from "@/output";
import { fromSnakeCaseSchema } from "@/util/schemas";

export const MetadataSchema = fromSnakeCaseSchema(
  z.object({
    dataObjectSize: z.number().default(0),
    lastModifiedTimestamp: z.string().optional(),
  }),
);
export type Metadata = z.infer<typeof MetadataSchema>;

const ListItemSchema = fromSnakeCaseSchema(
  z.object({
    resourceAddress: z.string(),
    resourceIdentity: z.string(),
    metadata: MetadataSchema,
  }),
);

const ListResultSchema = fromSnakeCaseSchema(
  z.object({
    folderAddress: z.string(),
    isTopLevelAddress: z.boolean().default(false),
    subfolderAddresses: z.array(z.string()),
    subResourceAddresses: z.array(z.string()),
    nextContinuationHandle: z.string().nullable(),
  }),
);
type ListResult = z.infer<typeof ListResultSchema>;

const ListStatSchema = fromSnakeCaseSchema(
  z.object({
    folderAddress: z.string(),
    isRoot: z.boolean().default(false),
    subfolderAddresses: z.array(z.string()),
    entries: z.array(ListItemSchema),
    nextContinuationHandle: z.string().nullable(),
  }),
);
export type ListStatResult = z.infer<typeof ListStatSchema>;

export const ResourceInfoItemSchema = fromSnakeCaseSchema(
  z.object({
    resourceIdentity: z.string(),
    metadata: MetadataSchema,
  }),
);
export type ResourceInfo = z.infer<typeof ResourceInfoItemSchema>;

export function fileFolderService(
  api: ApiInstance,
  log: Logger = noopLogger(),
) {
  return {
    async list(
      folderAddress: string,
      continuationHandle: string | null,
    ): Promise<ListResult> {
      const params = new URLSearchParams({ max_page_size: "1000" });
      if (continuationHandle) {
        params.set("continuation_handle", continuationHandle);
      }
      const response = await api.get<object>(
        `v1alpha/filefolder/list/${encodeURIComponent(folderAddress)}?${params.toString()}`,
      );
      return ListResultSchema.parse({
        ...(await response.json()),
        folder_address: folderAddress,
        is_top_level_address: false,
      });
    },

    async listStat(
      folderAddress: string,
      continuationHandle: string | null,
    ): Promise<ListStatResult> {
      const params = new URLSearchParams({ max_page_size: "1000" });
      if (continuationHandle) {
        params.set("continuation_handle", continuationHandle);
      }
      const response = await api.get<object>(
        `v1alpha/filefolder/liststat/${encodeURIComponent(folderAddress)}?${params.toString()}`,
      );
      return ListStatSchema.parse({
        ...(await response.json()),
        folder_address: folderAddress,
      });
    },

    async *listPages(folderAddress: string): AsyncGenerator<ListResult> {
      let handle: string | null = null;
      let pageNum = 0;
      do {
        pageNum++;
        log(
          "ls",
          pageNum === 1
            ? `page 1 — ${folderAddress}`
            : `page ${pageNum} — ${folderAddress} (continuation)`,
        );
        const page = await this.list(folderAddress, handle);
        yield page;
        handle = page.nextContinuationHandle;
      } while (handle !== null);
    },

    async *listStatPages(
      folderAddress: string,
    ): AsyncGenerator<ListStatResult> {
      let handle: string | null = null;
      let pageNum = 0;
      do {
        pageNum++;
        log(
          "ls",
          pageNum === 1
            ? `page 1 — ${folderAddress}`
            : `page ${pageNum} — ${folderAddress} (continuation)`,
        );
        const page = await this.listStat(folderAddress, handle);
        yield page;
        handle = page.nextContinuationHandle;
      } while (handle !== null);
    },

    async listAll(folderAddress: string): Promise<ListResult> {
      let handle: string | null = null;
      const subfolderAddresses: string[] = [];
      const subResourceAddresses: string[] = [];
      do {
        const page = await this.list(folderAddress, handle);
        subfolderAddresses.push(...page.subfolderAddresses);
        subResourceAddresses.push(...page.subResourceAddresses);
        handle = page.nextContinuationHandle;
      } while (handle !== null);
      return {
        folderAddress,
        isTopLevelAddress: false,
        subfolderAddresses,
        subResourceAddresses,
        nextContinuationHandle: null,
      };
    },

    async createFolder(folderAddress: string): Promise<void> {
      try {
        const response = await api.put(
          `v1alpha/filefolder/${encodeURIComponent(folderAddress)}`,
        );
        if (response.status !== 204) {
          throw new Error(`Unexpected status: ${response.status}`);
        }
      } catch (error) {
        if (isHttpError(error) && error.response.status === 409) {
          throw new Error("resource already exists as a file");
        }
        throw error;
      }
    },

    async deleteFolder(folderAddress: string): Promise<void> {
      const response = await api.delete(
        `v1alpha/filefolder/list/${encodeURIComponent(folderAddress)}`,
        {
          throwHttpErrors: false,
        },
      );
      if (![204, 404].includes(response.status)) {
        throw new Error(`Failed to delete folder — HTTP${response.status}`);
      }
    },
  };
}
