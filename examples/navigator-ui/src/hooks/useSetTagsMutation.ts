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

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { metadataService } from "@/services/storage/metadataService";
import { Connection } from "@/state/slices/connectionSlice";
import { contentMetadataQueryKey } from "./usePathMetadata";

interface SetTagsParams {
  resourceAddress: string;
  metadata: Record<string, string>;
  etag: string;
}

export function useSetTagsMutation(connection: Connection) {
  const queryClient = useQueryClient();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  return useMutation({
    mutationFn: async ({ resourceAddress, metadata, etag }: SetTagsParams) => {
      const options = getApiOptions(connection, { requestTimeout });
      const api = await createServiceApi(connection.info.discovery, "metadata", options);
      if (!api) {
        throw new Error("Storage metadata API is not available.");
      }
      const service = metadataService(api);

      await service.setTags(resourceAddress, metadata, etag);
    },
    onSuccess: (_, { resourceAddress }) => {
      void queryClient.invalidateQueries({
        queryKey: contentMetadataQueryKey(connection.endpoint, resourceAddress),
      });
    },
  });
}
