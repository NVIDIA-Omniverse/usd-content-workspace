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

import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useFileActionNotification } from "@/hooks/useFileActionNotification";
import { folderContentQueryKey } from "@/hooks/useFolderContentQuery";
import { useNotification } from "@/hooks/useNotification";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions, isHttpError } from "@/services/api";
import { fileFolderService, fileObjectService } from "@/services/storage";
import { useAppStore, useConnection } from "@/state/appStore";
import { createFileListItem, createFolderListItem, FileListItem, Path, PathType } from "@/state/path";
import { ClipboardOperation } from "@/state/slices/clipboardSlice";
import { Batch } from "@/util/batch";
import { Semaphore } from "@/util/semaphore";
import { ensureFolderPath, getParentPath, join } from "@/util/url";

export function usePathClipboard(endpoint: string) {
  const connection = useConnection(endpoint);
  const queryClient = useQueryClient();
  const sourcePath = useAppStore((state) => state.clipboard.sourcePath);
  const clipboard = useAppStore((state) => state.clipboard.items.get(endpoint) ?? emptyClipboard);
  const clipboardOperation = useAppStore((state) => state.clipboardOperation);
  const createClipboardNotification = useFileActionNotification();
  const notifications = useNotification();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  const empty = useCallback((): boolean => {
    return !clipboard.length;
  }, [clipboard]);

  const canPaste = useCallback(
    (destination: string): boolean => {
      // Cannot paste if clipboard is empty
      if (!clipboard.length) {
        return false;
      }
      // Cannot paste if any item would overwrite itself (same name at same location)
      // This happens when pasting into the source folder without renaming
      const normalizedDestination = ensureFolderPath(destination);
      const normalizedSource = ensureFolderPath(sourcePath);

      if (normalizedDestination === normalizedSource) {
        // Pasting into the same folder - check if any items would conflict
        for (const item of clipboard) {
          const targetPath = join(destination, encodeURIComponent(item.name));
          // Normalize both paths for comparison to handle folders with/without trailing slashes
          const normalizedTarget = item.type === PathType.Folder ? ensureFolderPath(targetPath) : targetPath;
          const normalizedItemAddress =
            item.type === PathType.Folder ? ensureFolderPath(item.resourceAddress) : item.resourceAddress;

          if (normalizedTarget === normalizedItemAddress) {
            // This item (file or folder) would overwrite itself, so paste is not allowed
            return false;
          }
        }
      }
      return true;
    },
    [clipboard, sourcePath]
  );

  const copyToClipboard = useAppStore((state) => state.copyToClipboard);
  const copy = useCallback(
    (resources: Path[]) => {
      copyToClipboard(endpoint, connection?.currentFolder ?? "", resources);
    },
    [copyToClipboard, endpoint, connection?.currentFolder]
  );

  const cut = useCallback(
    (resources: Path[]) => {
      copyToClipboard(endpoint, connection?.currentFolder ?? "", resources, ClipboardOperation.Cut);
    },
    [copyToClipboard, endpoint, connection?.currentFolder]
  );

  const clearClipboard = useAppStore((state) => state.clearClipboard);
  const clear = useCallback(() => {
    clearClipboard(endpoint);
  }, [clearClipboard, endpoint]);

  const paste = useMutation({
    mutationFn: async ({ destination, newName, overwrite }: PasteArguments) => {
      const concurrencyLimit = 15;
      const listSemaphore = new Semaphore(concurrencyLimit);
      const options = getApiOptions(connection, { requestTimeout });
      const fileFolderApi = await createServiceApi(connection.info.discovery, "filefolder", options);
      const fileObjectApi = await createServiceApi(connection.info.discovery, "fileobject", options);
      if (!fileFolderApi || !fileObjectApi) {
        throw new Error("Storage API is not available.");
      }

      if (clipboard.length === 0) {
        throw new Error("Clipboard is empty.");
      }

      if (newName && clipboard.length > 1) {
        throw new Error("Cannot rename more than one path at once.");
      }

      // Check if any folder in the clipboard is being pasted into its own child path
      const normalizedDestination = ensureFolderPath(destination);
      for (const resource of clipboard) {
        if (resource.type === PathType.Folder) {
          const normalizedSource = ensureFolderPath(resource.resourceAddress);
          if (normalizedDestination.startsWith(normalizedSource)) {
            notifications.show({
              type: "error",
              title: "Invalid Operation",
              message: `Cannot paste folder "${resource.name}" into its own child path.`,
            });
            throw new Error(`Cannot paste folder into its own child path: ${resource.name}`);
          }
        }
      }

      const notification = createClipboardNotification({
        action: newName ? "Rename" : clipboardOperation,
        data: clipboard,
      });
      const completedFiles = new Set<FileListItem>();
      const completedFolders = new Set<CompletedFolder>();
      const completedCutSourceAddresses = new Set<string>();
      const failedResources: Path[] = [];
      // Source resource addresses of files skipped because fail_if_exists blocked the copy or move.
      const skippedSourceAddresses: string[] = [];

      // The server responds with 409 Conflict when fail_if_exists blocked a copy or move because a
      // resource already exists at the destination. Treat that outcome as a skip rather than
      // a failure so the user can distinguish "already there" from an actual error.
      const shouldSkip = (error: unknown): boolean =>
        overwrite === false && isHttpError(error) && error.response.status === 409;

      /**
       * Process a single resource to be pasted, either by copying a file or recursively
       * copying a folder and its contents.
       *
       * @param resource - The resource to be processed.
       * @param destination - The destination address to paste the resource.
       * @returns Whether the resource completed, was skipped, or failed.
       */
      const processResource = async (resource: Path, destination: string): Promise<PasteResult> => {
        if (notification.signal.aborted) {
          return PasteResult.Failed;
        }

        if (resource.type === PathType.Folder || resource.type === PathType.TopLevelAddress) {
          let folders: string[] = [];
          let continuationHandle: string | null = null;
          const childFileResults: Promise<PasteResult>[] = [];

          let failed = false;
          let skipped = false;
          do {
            if (notification.signal.aborted) {
              break;
            }

            let response;
            try {
              response = await listSemaphore.run(() =>
                fileFolderService(fileFolderApi).listStat(resource.resourceAddress, continuationHandle)
              );
            } catch {
              failed = true;
              break;
            }

            await batch.add(
              response.entries.map((entry) => {
                const fileListItem = createFileListItem(entry);
                const childFileResult = Promise.withResolvers<PasteResult>();
                childFileResults.push(childFileResult.promise);
                return {
                  resource: fileListItem,
                  destination: join(destination, encodeURIComponent(fileListItem.name)),
                  onSettled: childFileResult.resolve,
                };
              })
            );

            folders = folders.concat(response.subfolderAddresses);
            continuationHandle = response.nextContinuationHandle;
          } while (continuationHandle != null);

          const isEmpty = folders.length === 0 && childFileResults.length === 0;
          if (isEmpty) {
            try {
              await fileFolderService(fileFolderApi).createFolder(destination);
            } catch (error) {
              if (shouldSkip(error)) {
                skippedSourceAddresses.push(resource.resourceAddress);
                skipped = true;
              } else {
                failed = true;
              }
            }
          }

          const childFolderResults = await Promise.all(
            folders.map(async (folder) => {
              const folderListItem = createFolderListItem(folder);
              return processResource(folderListItem, join(destination, encodeURIComponent(folderListItem.name)));
            })
          );

          for (const result of childFolderResults) {
            failed ||= result === PasteResult.Failed;
            skipped ||= result === PasteResult.Skipped;
          }
          for (const result of await Promise.all(childFileResults)) {
            failed ||= result === PasteResult.Failed;
            skipped ||= result === PasteResult.Skipped;
          }

          if (failed) {
            failedResources.push(resource);
            return PasteResult.Failed;
          }
          if (skipped) {
            return PasteResult.Skipped;
          }

          await queryClient.invalidateQueries({ queryKey: folderContentQueryKey(endpoint, destination) });
          if (clipboardOperation === ClipboardOperation.Cut) {
            queryClient.removeQueries({ queryKey: folderContentQueryKey(endpoint, resource.resourceAddress) });
            await queryClient.invalidateQueries({
              queryKey: folderContentQueryKey(endpoint, getParentPath(resource.resourceAddress)),
            });
          }

          notification.update(destination);
          completedFolders.add({ source: resource, destination });
          return PasteResult.Completed;
        }

        let result = PasteResult.Failed;
        await batch.add([
          {
            resource,
            destination,
            onSettled: (settledResult) => {
              result = settledResult;
            },
          },
        ]);
        await batch.flush();
        return result;
      };

      const copyResource = async ({ resource, destination }: CopyBatchItem): Promise<FileListItem> => {
        let resourceIdentity: string;

        if (clipboardOperation === ClipboardOperation.Cut) {
          const moveResult = await fileObjectService(fileObjectApi).move(
            resource.resourceAddress,
            destination,
            overwrite === undefined ? undefined : !overwrite
          );
          resourceIdentity = moveResult.resourceIdentity;

          // Invalidate source folder queries since the file has been moved
          queryClient.removeQueries({ queryKey: folderContentQueryKey(endpoint, resource.resourceAddress) });
          await queryClient.invalidateQueries({
            queryKey: folderContentQueryKey(endpoint, getParentPath(resource.resourceAddress)),
          });
        } else {
          const copyResult = await fileObjectService(fileObjectApi).copy(
            resource.resourceIdentity,
            destination,
            overwrite === undefined ? undefined : !overwrite
          );
          resourceIdentity = copyResult.resourceIdentity;
        }

        // Invalidate destination folder queries
        await queryClient.invalidateQueries({ queryKey: folderContentQueryKey(endpoint, destination) });
        // Invalidate version queries for the destination file, which may now point to
        // a different underlying object (and therefore a different version history).
        await queryClient.invalidateQueries({ queryKey: [endpoint, destination, "version"] });

        return createFileListItem({
          resourceIdentity,
          resourceAddress: destination,
          metadata: {
            ...resource.metadata,
          },
        });
      };

      const batch = new Batch<CopyBatchItem, FileListItem>(copyResource, {
        batchSize: concurrencyLimit,
        signal: notification.signal,
        onItemSuccess: (item, fileListItem) => {
          notification.update(item.destination);
          completedFiles.add(fileListItem);
          if (clipboardOperation === ClipboardOperation.Cut) {
            completedCutSourceAddresses.add(item.resource.resourceAddress);
          }
          item.onSettled?.(PasteResult.Completed);
        },
        onItemError: (item, error) => {
          if (shouldSkip(error)) {
            skippedSourceAddresses.push(item.resource.resourceAddress);
            item.onSettled?.(PasteResult.Skipped);
          } else {
            failedResources.push(item.resource);
            item.onSettled?.(PasteResult.Failed);
          }
        },
      });

      await Promise.all(
        clipboard.map((resource) =>
          processResource(resource, join(destination, encodeURIComponent(newName ?? resource.name)))
        )
      );
      await batch.flush();

      if (clipboardOperation === ClipboardOperation.Cut) {
        const sortedCompletedFolders = Array.from(completedFolders).sort(
          (a, b) => b.source.resourceAddress.length - a.source.resourceAddress.length
        );

        for (const { source } of sortedCompletedFolders) {
          try {
            await fileFolderService(fileFolderApi).deleteFolder(source.resourceAddress);
            completedCutSourceAddresses.add(source.resourceAddress);
          } catch {
            failedResources.push(source);
          }
        }
      }

      const completedResources = [
        ...completedFiles,
        ...Array.from(completedFolders, ({ destination }) => createFolderListItem(destination)),
      ];

      // A cut resource that was skipped (rather than moved) still exists at its original address, so
      // re-seat the clipboard on whatever wasn't actually moved instead of clearing it.
      const hasSkippedCutItems = clipboardOperation === ClipboardOperation.Cut && skippedSourceAddresses.length > 0;

      if (notification.signal.aborted) {
        notification.cancel();
      } else {
        if (hasSkippedCutItems) {
          copyToClipboard(
            endpoint,
            sourcePath,
            clipboard.filter((resource) => !completedCutSourceAddresses.has(resource.resourceAddress)),
            ClipboardOperation.Cut
          );
        }

        if (failedResources.length === 0) {
          // Skipped items reflect an intentional, user-requested outcome (keep the existing resource),
          // not an error, so this is still a success -- just report the skip count alongside it.
          if (!hasSkippedCutItems) {
            clearClipboard(endpoint);
          }
          notification.complete(completedResources, skippedSourceAddresses);
        } else if (completedResources.length === 0 && skippedSourceAddresses.length === 0) {
          notification.fail(failedResources);
        } else {
          notification.completePartially(completedResources, failedResources, skippedSourceAddresses);
        }
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: folderContentQueryKey(endpoint, destination) }),
        queryClient.invalidateQueries({ queryKey: folderContentQueryKey(endpoint, sourcePath) }),
      ]);
    },
  });

  return {
    empty,
    canPaste,
    items: clipboard,
    operation: clipboardOperation,
    sourcePath,
    clear,
    copy,
    cut,
    paste,
  };
}

export interface PasteArguments {
  destination: string;
  newName?: string;
  overwrite?: boolean;
}

const emptyClipboard: Path[] = [];

interface CopyBatchItem {
  resource: FileListItem;
  destination: string;
  onSettled?: (result: PasteResult) => void;
}

const enum PasteResult {
  Completed,
  Skipped,
  Failed,
}

interface CompletedFolder {
  source: Path;
  destination: string;
}
