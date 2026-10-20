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
import { fromSnakeCaseSchema } from "@/util/schemas";
import { ApiInstance, isHttpError } from "../api";

export function fileFolderService(api: ApiInstance) {
  return {
    async list(folderAddress: string, continuationHandle: string | null): Promise<ListResult> {
      const params = new URLSearchParams();
      params.set("max_page_size", "1000");
      if (continuationHandle) {
        params.set("continuation_handle", continuationHandle);
      }

      const response = await api.get<object>(`filefolder/list/${encodeURIComponent(folderAddress)}?${params}`);
      const responseObject = await response.json();
      return ListResultSchema.parse({
        ...responseObject,
        folder_address: folderAddress,
        is_top_level_address: false,
      });
    },
    async listStat(folderAddress: string, continuationHandle: string | null): Promise<ListStatResult> {
      const params = new URLSearchParams();
      params.set("max_page_size", "1000");
      if (continuationHandle) {
        params.set("continuation_handle", continuationHandle);
      }

      const response = await api.get<object>(`filefolder/liststat/${encodeURIComponent(folderAddress)}?${params}`);
      const responseObject = await response.json();
      return ListStatSchema.parse({
        ...responseObject,
        folder_address: folderAddress,
      });
    },
    async createFolder(folderAddress: string): Promise<void> {
      try {
        const response = await api.put(`filefolder/${encodeURIComponent(folderAddress)}`);
        if (response.status !== 204) {
          throw new Error(`Unexpected response status: ${response.status}`);
        }
      } catch (error) {
        if (isHttpError(error) && error.response.status === 409) {
          error.message = "resource already exists as a file";
        }
        throw error;
      }
    },
    async deleteFolder(folderAddress: string): Promise<void> {
      const response = await api.delete(`filefolder/list/${encodeURIComponent(folderAddress)}`);
      // 400 response is only possible when native folder support is enabled in storage api
      if (response.status === 400) {
        throw new Error("Failed to remove folder. Folder is not empty.");
      }
      if (response.status !== 204) {
        throw new Error(`Unexpected response status: ${response.status}`);
      }
    },
  };
}

export const MetadataSchema = fromSnakeCaseSchema(
  z.object({
    dataObjectSize: z.number(),
    lastModifiedTimestamp: z.string().optional(),
  })
);

export type Metadata = z.infer<typeof MetadataSchema>;

const ListItemSchema = fromSnakeCaseSchema(
  z.object({
    resourceAddress: z.string(),
    resourceIdentity: z.string(),
    metadata: MetadataSchema,
  })
);

export type ListItem = z.infer<typeof ListItemSchema>;

const ListResultSchema = fromSnakeCaseSchema(
  z.object({
    folderAddress: z.string(),
    isTopLevelAddress: z.boolean().default(false),
    subfolderAddresses: z.array(z.string()),
    subResourceAddresses: z.array(z.string()),
    nextContinuationHandle: z.string().nullable(),
  })
);

export type ListResult = z.infer<typeof ListResultSchema>;

const ListStatSchema = fromSnakeCaseSchema(
  z.object({
    folderAddress: z.string(),
    isRoot: z.boolean().default(false),
    subfolderAddresses: z.array(z.string()),
    entries: z.array(ListItemSchema),
    nextContinuationHandle: z.string().nullable(),
  })
);

export type ListStatResult = z.infer<typeof ListStatSchema>;

export const ResourceInfoItemSchema = fromSnakeCaseSchema(
  z.object({
    resourceIdentity: z.string(),
    metadata: MetadataSchema,
  })
);

export type ResourceInfo = z.infer<typeof ResourceInfoItemSchema>;
