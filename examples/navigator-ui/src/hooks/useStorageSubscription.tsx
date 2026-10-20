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

import { useCallback, useEffect } from "react";
import { InfiniteData, useQueryClient } from "@tanstack/react-query";
import { debounce } from "radash";
import { useAppConfig } from "@/hooks/useAppConfig";
import { FolderContent, folderContentQueryKey } from "@/hooks/useFolderContentQuery";
import { useNotification } from "@/hooks/useNotification";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { OmniEvent, StorageEvent } from "@/services/notificationService";
import { fileObjectService } from "@/services/storage";
import { Connection } from "@/state/appStore";
import { createFileListItem, createFolderListItem } from "@/state/path";
import { Batch } from "@/util/batch";
import { decodePathName, getPathName } from "@/util/url";
import { PathSubscription } from "./usePathSubscription";

/**
 * Hook that handles storage-related events and updates the cache accordingly.
 * It processes events in batches to improve performance and handles various storage operations
 * like file creation, deletion, and folder operations.
 */
export function useStorageSubscription(connection: Connection, pathSubscription: PathSubscription) {
  const notification = useNotification();
  const queryClient = useQueryClient();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  type FolderContentQueryKey = ReturnType<typeof folderContentQueryKey>;

  const handleFileCreated = useCallback(
    async (event: StorageEvent, queryKey: FolderContentQueryKey, connection: Connection) => {
      const apiOptions = getApiOptions(connection, { requestTimeout });
      const api = await createServiceApi(connection.info.discovery, "fileobject", apiOptions);
      if (!api) {
        return;
      }
      const { metadata, resourceIdentity } = await fileObjectService(api).stat(event.message.resourceAddress);

      if (metadata) {
        queryClient.setQueryData(queryKey, (oldData: InfiniteData<FolderContent, string | null>) => {
          if (!oldData) {
            return oldData;
          }

          const rawAddress = event.message.resourceAddress;
          const decodedAddress = decodePathName(rawAddress);
          const updatedPages = [...oldData.pages];

          for (let i = 0; i < updatedPages.length; i++) {
            const existingKey = findCacheKey(updatedPages[i].files, rawAddress);

            if (existingKey !== null) {
              const file = createFileListItem({
                resourceAddress: existingKey,
                resourceIdentity: resourceIdentity ?? "",
                metadata,
              });
              const updatedFiles = new Map(updatedPages[i].files);
              updatedFiles.delete(existingKey);
              updatedFiles.set(existingKey, file);
              updatedPages[i] = { ...updatedPages[i], files: updatedFiles };
              return { ...oldData, pages: updatedPages };
            }
          }

          const file = createFileListItem({
            resourceAddress: decodedAddress,
            resourceIdentity: resourceIdentity ?? "",
            metadata,
          });
          if (updatedPages.length > 0) {
            const lastIndex = updatedPages.length - 1;
            updatedPages[lastIndex] = {
              ...updatedPages[lastIndex],
              files: new Map(updatedPages[lastIndex].files).set(decodedAddress, file),
            };
          }

          return { ...oldData, pages: updatedPages };
        });
      }
    },
    [queryClient, requestTimeout, createServiceApi]
  );

  const handleFileDeleted = useCallback(
    (event: StorageEvent, queryKey: FolderContentQueryKey) => {
      queryClient.setQueryData(queryKey, (oldData: InfiniteData<FolderContent, string | null>) => {
        if (!oldData) {
          return oldData;
        }

        const updatedPages = [...oldData.pages];
        const rawAddress = event.message.resourceAddress;

        for (let i = 0; i < updatedPages.length; i++) {
          const existingKey = findCacheKey(updatedPages[i].files, rawAddress);

          if (existingKey !== null) {
            const updatedFiles = new Map(updatedPages[i].files);
            updatedFiles.delete(existingKey);
            updatedPages[i] = { ...updatedPages[i], files: updatedFiles };
            break;
          }
        }

        return { ...oldData, pages: updatedPages };
      });
    },
    [queryClient]
  );

  const handleFolderCreated = useCallback(
    (event: StorageEvent, queryKey: FolderContentQueryKey) => {
      queryClient.setQueryData(queryKey, (oldData: InfiniteData<FolderContent, string | null>) => {
        if (!oldData) {
          return oldData;
        }

        const rawAddress = event.message.resourceAddress;
        const decodedAddress = decodePathName(rawAddress);
        const updatedPages = [...oldData.pages];

        for (let i = 0; i < updatedPages.length; i++) {
          const existingKey = findCacheKey(updatedPages[i].folders, rawAddress);

          if (existingKey !== null) {
            const folder = createFolderListItem(existingKey);
            const updatedFolders = new Map(updatedPages[i].folders);
            updatedFolders.delete(existingKey);
            updatedFolders.set(existingKey, folder);
            updatedPages[i] = { ...updatedPages[i], folders: updatedFolders };
            return { ...oldData, pages: updatedPages };
          }
        }

        const folder = createFolderListItem(decodedAddress);
        if (updatedPages.length > 0) {
          const lastIndex = updatedPages.length - 1;
          updatedPages[lastIndex] = {
            ...updatedPages[lastIndex],
            folders: new Map(updatedPages[lastIndex].folders).set(decodedAddress, folder),
          };
        }

        return { ...oldData, pages: updatedPages };
      });
    },
    [queryClient]
  );

  const handleFolderDeleted = useCallback(
    (event: StorageEvent, queryKey: FolderContentQueryKey) => {
      queryClient.setQueryData(queryKey, (oldData: InfiniteData<FolderContent, string | null>) => {
        if (!oldData) {
          return oldData;
        }

        const updatedPages = [...oldData.pages];
        const rawAddress = event.message.resourceAddress;

        for (let i = 0; i < updatedPages.length; i++) {
          const existingKey = findCacheKey(updatedPages[i].folders, rawAddress);

          if (existingKey !== null) {
            const updatedFolders = new Map(updatedPages[i].folders);
            updatedFolders.delete(existingKey);
            updatedPages[i] = { ...updatedPages[i], folders: updatedFolders };
            break;
          }
        }

        return { ...oldData, pages: updatedPages };
      });
    },
    [queryClient]
  );

  const updateCache = useCallback(
    async (event: StorageEvent) => {
      const queryKey = folderContentQueryKey(connection.endpoint, connection.currentFolder);

      switch (event.eventType) {
        case "omni.storage.created": {
          await handleFileCreated(event, queryKey, connection);
          break;
        }
        case "omni.storage.deleted": {
          handleFileDeleted(event, queryKey);
          break;
        }
        case "omni.storage.dir_created": {
          handleFolderCreated(event, queryKey);
          break;
        }
        case "omni.storage.dir_deleted": {
          handleFolderDeleted(event, queryKey);
          break;
        }
      }
    },
    [connection, handleFileCreated, handleFileDeleted, handleFolderCreated, handleFolderDeleted]
  );

  useEffect(() => {
    const batch = new Batch<StorageEvent, void, Error>(
      async (event) => {
        await updateCache(event);
      },
      {
        batchSize: 100,
        onItemError: (item) => {
          notification.show({
            type: "error",
            title: `Error handling ${item.eventType} event for "${getPathName(getEventResourceAddress(item))}"`,
            message: "Path content may be out of date. Please refresh the page.",
          });
        },
      }
    );

    const flush = debounce({ delay: 250 }, () => {
      void batch.flush();
    });

    const unsubscribe = pathSubscription.onEvent((event) => {
      if (isStorageSubscriptionEvent(event)) {
        void batch.add([event]);
        flush();
      }
    });

    return () => {
      unsubscribe();
      flush.cancel();
      void batch.flush();
    };
  }, [pathSubscription, notification, updateCache]);
}

function isStorageSubscriptionEvent(event: OmniEvent): event is StorageEvent {
  return event.eventType.startsWith("omni.storage.");
}

function getEventResourceAddress(event: StorageEvent) {
  return event.message.resourceAddress;
}

function findCacheKey<T>(map: Map<string, T>, rawAddress: string): string | null {
  if (map.has(rawAddress)) {
    return rawAddress;
  }
  const decoded = decodePathName(rawAddress);
  if (map.has(decoded)) {
    return decoded;
  }
  for (const key of map.keys()) {
    if (decodePathName(key) === decoded) {
      return key;
    }
  }
  return null;
}
