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
import { useFileActionNotification } from "@/hooks/useFileActionNotification";
import { folderContentQueryKey } from "@/hooks/useFolderContentQuery";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { fileFolderService, fileObjectService } from "@/services/storage";
import { Connection, useAppStore } from "@/state/appStore";
import { Path } from "@/state/path";
import { Batch } from "@/util/batch";
import { Semaphore } from "@/util/semaphore";

export function useDeletePathMutation(connection: Connection | undefined) {
  const createDeleteNotification = useFileActionNotification();
  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  const queryClient = useQueryClient();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  return useMutation({
    mutationFn: async (resources: Path[]) => {
      const concurrencyLimit = 15;
      const listSemaphore = new Semaphore(concurrencyLimit);
      if (!connection) {
        throw new Error("No connection available");
      }
      const options = getApiOptions(connection, { requestTimeout });
      const fileFolderApi = await createServiceApi(connection.info.discovery, "filefolder", options);
      const fileObjectApi = await createServiceApi(connection.info.discovery, "fileobject", options);
      if (!fileFolderApi || !fileObjectApi) {
        throw new Error("No connection available");
      }

      const currentFolder = connection.currentFolder;
      const deleteNotification = createDeleteNotification({ action: "Delete", data: resources });

      // Track folder hierarchy for proper deletion order
      const folderHierarchy = new Map<string, string[]>();

      // Track completed resources and resources that caused errors
      const completedResources = new Set<string>();
      const failedResources = new Set<string>();

      const deleteResource = async ({ resourceAddress, isFolder }: DeleteBatchItem) => {
        if (failedResources.has(resourceAddress)) {
          throw new Error("Resource has already failed to delete");
        }
        deleteNotification.update(resourceAddress);
        if (isFolder) {
          await fileFolderService(fileFolderApi).deleteFolder(resourceAddress);
        } else {
          await fileObjectService(fileObjectApi).delete(resourceAddress);
        }
      };

      const batch = new Batch<DeleteBatchItem>(deleteResource, {
        batchSize: concurrencyLimit,
        onItemSuccess: (item) => {
          completedResources.add(item.resourceAddress);
        },
        onItemError: (item) => {
          failedResources.add(item.resourceAddress);
        },
      });

      const processResource = async (resourceAddress: string, isFolder: boolean, parentFolder?: string) => {
        if (deleteNotification.signal.aborted) {
          return;
        }

        if (isFolder) {
          // Track parent-child relationships for folders
          if (parentFolder) {
            const children = folderHierarchy.get(parentFolder) || [];
            children.push(resourceAddress);
            folderHierarchy.set(parentFolder, children);
          }

          let continuationHandle: string | null = null;
          do {
            if (deleteNotification.signal.aborted) {
              return;
            }

            let response;
            try {
              response = await listSemaphore.run(() =>
                fileFolderService(fileFolderApi).list(resourceAddress, continuationHandle)
              );
            } catch {
              // Failed to list the folder further
              failedResources.add(resourceAddress);
              break;
            }

            await batch.add(
              response.subResourceAddresses.map((resourceAddress) => ({ resourceAddress, isFolder: false }))
            );

            await Promise.all(
              response.subfolderAddresses.map(async (subfolder) => {
                if (deleteNotification.signal.aborted) {
                  return;
                }
                try {
                  await processResource(subfolder, true, resourceAddress);
                } catch {
                  failedResources.add(subfolder);
                }
              })
            );

            continuationHandle = response.nextContinuationHandle;
          } while (continuationHandle != null);
        } else {
          await batch.add([{ resourceAddress, isFolder: false }]);
        }
      };

      // Process all resources selected by the user
      await Promise.all(resources.map((resource) => processResource(resource.resourceAddress, resource.isFolderLike)));

      // Delete folders in proper order (children before parents)
      const populateHierarchyList = async (folderAddresses: string[]) => {
        for (const resourceAddress of folderAddresses) {
          const children = folderHierarchy.get(resourceAddress) || [];
          await populateHierarchyList(children);
          await batch.add([{ resourceAddress, isFolder: true }]);
        }
      };

      // Start with top-level folders (those that were directly selected)
      const topLevelFolders = resources
        .filter((resource) => resource.isFolderLike)
        .map((resource) => resource.resourceAddress);

      await populateHierarchyList(topLevelFolders);

      if (deleteNotification.signal.aborted) {
        return;
      }

      // Delete folders (children first, then parents)
      await batch.flush();

      if (failedResources.size === 0) {
        deleteNotification.complete(Array.from(completedResources));
      } else if (completedResources.size === 0) {
        deleteNotification.fail(Array.from(failedResources));
      } else {
        deleteNotification.completePartially(Array.from(completedResources), Array.from(failedResources));
      }

      const selectedPaths = connection.selectedPaths;
      const deletedResourceAddresses = new Set(resources.map((r) => r.resourceAddress));
      const remainingSelectedPaths = Array.from(selectedPaths).filter(
        (path) => !deletedResourceAddresses.has(path.resourceAddress)
      );
      setSelectedPaths(connection.endpoint, remainingSelectedPaths);
      await queryClient.invalidateQueries({
        queryKey: folderContentQueryKey(connection.endpoint, currentFolder),
      });
    },
  });
}

interface DeleteBatchItem {
  resourceAddress: string;
  isFolder: boolean;
}
