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
import { rootFolderPath } from "@/state/appStore";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { ensureFolderPath } from "@/util/url";
import { ListStatResult } from "./fileFolderService";

export function capabilitiesService(api: ApiInstance) {
  return {
    listTopLevelAddresses: async (): Promise<ListStatResult> => {
      const response = await api.get<object>("capabilities/top-level-addresses");
      return TopLevelAddressesResultSchema.parse(await response.json());
    },
  };
}

const TopLevelAddressEntrySchema = fromSnakeCaseSchema(
  z.object({
    topLevelAddress: z.string(),
  })
);

export type TopLevelAddressEntry = z.infer<typeof TopLevelAddressEntrySchema>;

const TopLevelAddressesResultSchema = fromSnakeCaseSchema(
  z
    .object({
      items: z.array(TopLevelAddressEntrySchema),
    })
    .transform(({ items }) => ({
      folderAddress: rootFolderPath,
      isRoot: true,
      subfolderAddresses: items.map((item) => ensureFolderPath(item.topLevelAddress)),
      entries: [],
      nextContinuationHandle: null,
    }))
);

export type TopLevelAddresses = z.infer<typeof TopLevelAddressesResultSchema>;
