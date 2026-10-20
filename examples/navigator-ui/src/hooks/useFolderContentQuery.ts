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

import { useEffect, useMemo } from "react";
import { InfiniteData, QueryKey, useInfiniteQuery } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { CreateServiceApi, useServiceApi } from "@/hooks/useServiceApi";
import { ApiOptionsConfig, getApiOptions } from "@/services/api";
import { capabilitiesService, fileFolderService } from "@/services/storage";
import { Connection, rootFolderPath, useConnection } from "@/state/appStore";
import {
  createFileListItem,
  createFolderListItem,
  createTopLevelAddressListItem,
  FileListItem,
  FolderListItem,
  Path,
  TopLevelAddressItem,
} from "@/state/path";
import { ensureFolderPath } from "@/util/url";

/**
 * Hook that fetches and manages folder content with infinite scrolling capabilities.
 * When used for the root folder, it lists top-level addresses. For other folders, it lists their contents.
 *
 * @param endpoint - The endpoint where the folder path is located
 * @param resourceAddress - The path of the folder whose contents should be fetched
 * @param enabled - Whether the query should be enabled (defaults to true)
 * @returns An infinite query result object containing the folder contents, loading states, and pagination controls.
 *          The query will automatically fetch subsequent pages when available.
 */
export function useFolderContentQuery(endpoint: string, resourceAddress: string, enabled = true) {
  const connection = useConnection(endpoint);
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();
  const query = useInfiniteQuery<
    FolderContent,
    Error,
    InfiniteData<FolderContent, string | null>,
    QueryKey,
    string | null
  >({
    queryKey: folderContentQueryKey(endpoint, resourceAddress),
    queryFn: async ({ pageParam }) => {
      return fetchFolderContent(connection, resourceAddress, pageParam, { requestTimeout }, createServiceApi);
    },

    initialPageParam: null,
    getNextPageParam: (lastPage: FolderContent) => lastPage.nextContinuationHandle,
    enabled: enabled && !!connection,
    // Always treat cached data as stale so navigating to a previously visited folder triggers a background refetch
    staleTime: 0,
    refetchOnWindowFocus: false,
  });

  useEffect(() => {
    if (query.isSuccess && query.hasNextPage) {
      void query.fetchNextPage({ cancelRefetch: false });
    }
  }, [query]);

  return query;
}

export interface FolderContent {
  isRoot?: boolean;
  folders: Map<string, FolderListItem | TopLevelAddressItem>;
  files: Map<string, FileListItem>;
  nextContinuationHandle?: string | null;
}

export type FolderQuery = ReturnType<typeof useFolderContentQuery>;

export async function fetchFolderContent(
  connection: Connection,
  resourceAddress: string,
  page: string | null,
  config: ApiOptionsConfig,
  createServiceApi: CreateServiceApi
) {
  const options = getApiOptions(connection, config);
  const discovery = connection.info.discovery;

  const listStatResult = await (async () => {
    if (resourceAddress === rootFolderPath) {
      const capabilitiesApi = await createServiceApi(discovery, "capabilities", options);
      if (!capabilitiesApi) {
        throw new Error("Storage capabilities API is not available.");
      }
      return capabilitiesService(capabilitiesApi).listTopLevelAddresses();
    }

    const fileFolderApi = await createServiceApi(discovery, "filefolder", options);
    if (!fileFolderApi) {
      throw new Error("Storage filefolder API is not available.");
    }
    return fileFolderService(fileFolderApi).listStat(ensureFolderPath(resourceAddress), page);
  })();

  const folders = new Map<string, FolderListItem | TopLevelAddressItem>();
  listStatResult.subfolderAddresses.forEach((folder) => {
    const item = listStatResult.isRoot ? createTopLevelAddressListItem(folder) : createFolderListItem(folder);
    folders.set(item.resourceAddress, item);
  });

  const files = new Map<string, FileListItem>();
  listStatResult.entries.forEach((file) => {
    const item = createFileListItem(file);
    files.set(file.resourceAddress, item);
  });

  return {
    isRoot: listStatResult.isRoot,
    folders,
    files,
    nextContinuationHandle: listStatResult.nextContinuationHandle,
  };
}

export function useFolderContentData(folderContent: ReturnType<typeof useFolderContentQuery>): Path[] {
  return useMemo(() => {
    const folderMap = new Map<string, FolderListItem | TopLevelAddressItem>();
    const folders = folderContent.data?.pages.flatMap((page) => Array.from(page.folders.values())) || [];
    folders.forEach((folder) => {
      folderMap.set(folder.resourceAddress, folder);
    });

    const fileMap = new Map<string, FileListItem>();
    const files = folderContent.data?.pages.flatMap((page) => Array.from(page.files.values())) || [];
    files.forEach((file) => {
      fileMap.set(file.resourceAddress, file);
    });
    return ([] as Path[]).concat(Array.from(folderMap.values()), Array.from(fileMap.values()));
  }, [folderContent.data]);
}

export function folderContentQueryKey(endpoint: string, resourceAddress?: string) {
  return [endpoint, ensureFolderPath(resourceAddress ?? rootFolderPath), "content"] as const;
}
