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

import { useQuery } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { CreateServiceApi, useServiceApi } from "@/hooks/useServiceApi";
import { ApiOptionsConfig, getApiOptions } from "@/services/api";
import { metadataService, PathMetadata } from "@/services/storage/metadataService";
import { Connection } from "@/state/slices/connectionSlice";

export function usePathMetadata(resourceAddress: string, connection: Connection, enabled = true) {
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();
  return useQuery({
    queryKey: contentMetadataQueryKey(connection.endpoint, resourceAddress),
    queryFn: async () => {
      return await getPathMetadata(resourceAddress, connection, { requestTimeout }, createServiceApi);
    },
    enabled: enabled && !!resourceAddress,
  });
}

export async function getPathMetadata(
  resourceAddress: string,
  connection: Connection,
  config: ApiOptionsConfig,
  createServiceApi: CreateServiceApi
): Promise<PathMetadata> {
  try {
    const options = getApiOptions(connection, config);
    const api = await createServiceApi(connection.info.discovery, "metadata", options);
    if (!api) {
      throw new Error("Storage metadata API is not available.");
    }
    const service = metadataService(api);
    return await service.get(resourceAddress);
  } catch (error) {
    // If metadata can't be received, return default metadata.
    return {
      acl: {
        // Assume that the current user has full permissions for the specified path.
        // If permissions are actually missing, the API will handle this and return an HTTP403 error.
        value: ["read", "write"],
        etag: "",
      },
      tags: {
        value: {},
        etag: "",
      },
      thumbnails: [],
    };
  }
}

export function contentMetadataQueryKey(endpoint: string, resourceAddress?: string) {
  return [endpoint, resourceAddress, "metadata"] as const;
}
