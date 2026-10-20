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

import { useCallback, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Progress } from "ky";
import { FileWithPath } from "@mantine/dropzone";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useFileActionNotification } from "@/hooks/useFileActionNotification";
import { folderContentQueryKey } from "@/hooks/useFolderContentQuery";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { useActiveConnection } from "@/state/appStore";
import { PathType } from "@/state/path";
import { Batch } from "@/util/batch";
import { formatBytes } from "@/util/file";
import { getParentPath, join } from "@/util/url";

export function useFileUpload() {
  const queryClient = useQueryClient();
  const activeConnection = useActiveConnection();
  const createNotification = useFileActionNotification();
  const { createServiceApi } = useServiceApi();
  const { batchSize, requestTimeout } = useAppConfig();

  const upload = useCallback(
    async (entries: UploadEntry[]) => {
      if (!activeConnection) {
        throw new Error("No active connection available");
      }
      const options = getApiOptions(activeConnection, { requestTimeout });
      const api = await createServiceApi(activeConnection.info.discovery, "fileobject", options);
      if (!api) {
        throw new Error("No active connection available");
      }
      const { endpoint, currentFolder } = activeConnection;
      const uploadNotification = createNotification({ action: "Upload", data: entries });

      const completedItems = new Set<string>();
      const failedItems = new Set<string>();
      const totalBytes = entries.reduce((acc, entry) => {
        return entry.type === PathType.Folder ? acc : acc + entry.file.size;
      }, 0);
      let completedBytes = 0;
      let activeBytes = 0;
      let hasReportedProgress = false;

      const startTime = Date.now();
      const progressByEntry = new Map<FileEntry, number>();
      let progressUpdateTimeout: ReturnType<typeof setTimeout> | undefined;
      let lastProgressUpdateTime = 0;
      let pendingProgress: UploadProgressSnapshot | undefined;

      const publishProgress = ({ bytes, percent }: UploadProgressSnapshot) => {
        lastProgressUpdateTime = Date.now();
        hasReportedProgress = true;

        // Calculate average speed since the start of the upload
        const elapsedSeconds = (lastProgressUpdateTime - startTime) / 1000;
        const rate = elapsedSeconds > 0 ? bytes / elapsedSeconds : 0;

        uploadNotification.progress(
          `${formatBytes(bytes)} / ${formatBytes(totalBytes)}`,
          `${formatBytes(rate)}/s`,
          percent
        );
      };

      const flushProgress = () => {
        if (progressUpdateTimeout) {
          clearTimeout(progressUpdateTimeout);
          progressUpdateTimeout = undefined;
        }
        if (!pendingProgress) {
          return;
        }

        const progress = pendingProgress;
        pendingProgress = undefined;
        publishProgress(progress);
      };

      const scheduleProgress = () => {
        if (!pendingProgress) {
          return;
        }

        const now = Date.now();
        const elapsedSinceLastUpdate = now - lastProgressUpdateTime;
        if (lastProgressUpdateTime === 0 || elapsedSinceLastUpdate >= progressUpdateIntervalMs) {
          flushProgress();
          return;
        }

        if (!progressUpdateTimeout) {
          progressUpdateTimeout = setTimeout(flushProgress, progressUpdateIntervalMs - elapsedSinceLastUpdate);
        }
      };

      const updateProgress = (entry: FileEntry, { transferredBytes }: Progress) => {
        if (!Number.isFinite(transferredBytes)) {
          return;
        }

        const previousBytes = progressByEntry.get(entry) ?? 0;
        const nextBytes = Math.min(entry.file.size, Math.max(previousBytes, transferredBytes));
        if (progressByEntry.has(entry) && nextBytes === previousBytes) {
          return;
        }

        progressByEntry.set(entry, nextBytes);
        activeBytes += nextBytes - previousBytes;

        const bytes = completedBytes + activeBytes;
        if (bytes === 0 && !hasReportedProgress) {
          return;
        }

        pendingProgress = {
          bytes,
          percent: totalBytes === 0 ? 100 : Number((bytes / totalBytes).toFixed(2)) * 100,
        };
        scheduleProgress();
      };

      const batch = new Batch<UploadEntry, void, Error>(
        async (entry) => {
          if (entry.type === PathType.Folder) {
            // Folders are not uploaded explicitly.
            // Instead, they are created automatically when a file inside that folder is uploaded.
            return;
          }

          const { file, path } = entry;
          const resourceAddress = join(path, encodeURIComponent(file.name));
          await fileObjectService(api).upload(
            resourceAddress,
            file,
            (progressEvent: Progress) => updateProgress(entry, progressEvent),
            uploadNotification.signal
          );
        },
        {
          batchSize,
          signal: uploadNotification.signal,
          onItemSuccess: (entry) => {
            if (entry.type === PathType.File) {
              updateProgress(entry, {
                percent: 1,
                transferredBytes: entry.file.size,
                totalBytes: entry.file.size,
              });
              flushProgress();

              activeBytes -= progressByEntry.get(entry) ?? 0;
              progressByEntry.delete(entry);
              completedBytes += entry.file.size;
              completedItems.add(join(entry.path, entry.name));
            }
          },
          onItemError: (entry) => {
            failedItems.add(join(entry.path, entry.name));
          },
        }
      );

      await batch.add(entries);
      await batch.flush();

      const folderItems = entries
        .filter((entry) => entry.type === PathType.Folder)
        .map((entry) => join(entry.path, entry.name));

      flushProgress();
      if (uploadNotification.signal.aborted) {
        uploadNotification.cancel();
      } else if (failedItems.size === 0) {
        uploadNotification.complete([...completedItems, ...folderItems]);
      } else if (completedItems.size === 0) {
        uploadNotification.fail(Array.from(failedItems));
      } else {
        uploadNotification.completePartially([...completedItems, ...folderItems], Array.from(failedItems));
      }

      await Promise.all([
        ...Array.from(completedItems).flatMap((path) => [
          queryClient.invalidateQueries({
            queryKey: folderContentQueryKey(endpoint, getParentPath(path)),
          }),
          queryClient.invalidateQueries({
            queryKey: [endpoint, path, "version"],
          }),
        ]),
        queryClient.invalidateQueries({
          queryKey: folderContentQueryKey(endpoint, currentFolder),
        }),
      ]);
    },
    [activeConnection, createServiceApi, requestTimeout, createNotification, queryClient, batchSize]
  );

  const uploadFiles = useCallback(
    async (handles: FileSystemFileHandle[]) => {
      if (!activeConnection) {
        return;
      }

      const fileEntries = await Promise.all(
        handles.map(
          async (handle) =>
            ({
              name: handle.name,
              file: await handle.getFile(),
              path: activeConnection.currentFolder,
              type: PathType.File,
            }) satisfies FileEntry
        )
      );
      await upload(fileEntries);
    },
    [activeConnection, upload]
  );

  const uploadFolder = useCallback(
    async (dirHandle: FileSystemDirectoryHandle) => {
      if (!activeConnection) {
        throw new Error("No active connection available");
      }
      const folderPath = join(activeConnection.currentFolder, encodeURIComponent(dirHandle.name));
      const entries = await handleDirectoryEntry(dirHandle, folderPath);
      const currentFolderEntry: FolderEntry = {
        name: dirHandle.name,
        type: PathType.Folder,
        path: activeConnection.currentFolder,
      };
      await upload([...entries, currentFolderEntry]);
    },
    [activeConnection, upload]
  );

  const uploadFromDropzone = useCallback(
    async (files: FileWithPath[]): Promise<void> => {
      if (!activeConnection) {
        return;
      }

      const folderEntries = new Map<string, FolderEntry>();

      const fileEntries: FileEntry[] = files.map((file) => {
        // remove the ./ or / from the path
        const trimmedPath = file.path?.replace(/^(\.\/|\/)/, "");
        const parts = trimmedPath?.split("/").filter(Boolean);
        // remove the file name
        parts?.pop();

        let path = activeConnection.currentFolder;
        for (const part of parts ?? []) {
          const parentPath = path;
          path = join(parentPath, encodeURIComponent(part));
          folderEntries.set(path, {
            name: part,
            type: PathType.Folder,
            path: parentPath,
          });
        }

        return {
          file,
          name: file.name,
          path,
          type: PathType.File,
        };
      });

      await upload([...fileEntries, ...folderEntries.values()]);
    },
    [activeConnection, upload]
  );

  return useMemo(
    () => ({
      uploadFiles,
      uploadFolder,
      uploadFromDropzone,
    }),
    [uploadFiles, uploadFolder, uploadFromDropzone]
  );
}

type UploadEntry = FileEntry | FolderEntry;

interface UploadProgressSnapshot {
  bytes: number;
  percent: number;
}

const progressUpdateIntervalMs = 250;

export interface FileEntry {
  name: string;
  type: PathType.File;
  file: File;
  path: string;
}

export interface FolderEntry {
  name: string;
  type: PathType.Folder;
  path: string;
}

/**
 * Recursively processes a directory handle and collects all file entries
 * @param dirHandle - The directory handle to process
 * @param path - The current path being processed
 * @returns Promise resolving to an array of FileEntry objects
 **/
async function handleDirectoryEntry(dirHandle: FileSystemDirectoryHandle, path: string): Promise<UploadEntry[]> {
  const entries: UploadEntry[] = [];
  for await (const entry of dirHandle.values()) {
    if (entry.kind === "file") {
      entries.push({ name: entry.name, file: await entry.getFile(), path, type: PathType.File });
    }
    if (entry.kind === "directory") {
      const newHandle = await dirHandle.getDirectoryHandle(entry.name, { create: false });

      const folderEntry: FolderEntry = {
        name: entry.name,
        path,
        type: PathType.Folder,
      };
      const fileEntries = await handleDirectoryEntry(newHandle, join(path, encodeURIComponent(entry.name)));
      entries.push(...fileEntries, folderEntry);
    }
  }
  return entries;
}
