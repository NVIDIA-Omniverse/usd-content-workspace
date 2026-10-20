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
import { formatDistanceToNowStrict } from "date-fns";
import { Progress } from "ky";
import { throttle } from "radash";
import { getApiOptions } from "@/services/api";
import { fileFolderService } from "@/services/storage/fileFolderService";
import { fileObjectService } from "@/services/storage/fileObjectService";
import { Connection } from "@/state/appStore";
import { Path, PathType } from "@/state/path";
import { Batch } from "@/util/batch";
import { formatBytes } from "@/util/file";
import { sleep } from "@/util/sleep";
import { getPathName, sanitizePathName } from "@/util/url";
import { useAppConfig } from "./useAppConfig";
import { useFileActionNotification } from "./useFileActionNotification";
import { useServiceApi } from "./useServiceApi";

export function useFileDownload(connection: Connection | undefined) {
  const createDownloadNotification = useFileActionNotification();
  const { createServiceApi } = useServiceApi();
  const { batchSize, requestTimeout } = useAppConfig();

  const download = useCallback(
    async (files: Path[], selectedFolderHandle: FileSystemDirectoryHandle) => {
      if (!connection) {
        throw new Error("No connection available");
      }
      const options = getApiOptions(connection, { requestTimeout });
      const fileFolderApi = await createServiceApi(connection.info.discovery, "filefolder", options);
      const fileObjectApi = await createServiceApi(connection.info.discovery, "fileobject", options);
      if (!fileFolderApi || !fileObjectApi) {
        throw new Error("No connection available");
      }
      const downloadNotification = createDownloadNotification({ action: "Download", data: files });

      const completedItems: string[] = [];
      const failedItems: string[] = [];
      const downloadStats: DownloadStats = {
        startTime: Date.now(),
        completedFiles: 0,
        completedBytes: 0,
        totalBytes: 0,
      };

      const startTime = Date.now();

      const batch = new Batch<DownloadBatchItem, number>(downloadFile, {
        batchSize,
        signal: downloadNotification.signal,
        onBatchComplete: () => {
          // Add a delay to prevent the UI from becoming unresponsive while waiting for batch processing to complete.
          // This option is required in the case where a large number of 0 byte files are downloaded.
          return sleep(10);
        },
        onItemSuccess: (item, result) => {
          completedItems.push(item.resourceAddress);
          downloadStats.completedBytes += result;
          downloadStats.completedFiles++;
        },
        onItemError: (item) => {
          failedItems.push(item.resourceAddress);
        },
      });

      const processFolder = async (folderAddress: string, parentFolderHandle: FileSystemDirectoryHandle) => {
        if (downloadNotification.signal.aborted) {
          return;
        }

        let folderHandle: FileSystemDirectoryHandle | null = null;
        try {
          // create the local directory first before adding files for download
          const localFolderName = sanitizePathName(getPathName(folderAddress));
          folderHandle = await parentFolderHandle.getDirectoryHandle(
            // paths containing // are allowed in S3
            localFolderName.length > 0 ? localFolderName : "no name",
            { create: true }
          );
        } catch {
          // folder handle will remain null if an error occurs
        }

        if (!folderHandle) {
          failedItems.push(folderAddress);
          return;
        }

        let continuationHandle: string | null = null;
        do {
          try {
            const listStatResult = await fileFolderService(fileFolderApi).listStat(folderAddress, continuationHandle);

            await batch.add(
              listStatResult.entries.map((item) => ({
                resourceAddress: item.resourceAddress,
                fileSize: item.metadata.dataObjectSize,
                parentFolderHandle: folderHandle,
              }))
            );
            await Promise.all(listStatResult.subfolderAddresses.map((folder) => processFolder(folder, folderHandle)));

            continuationHandle = listStatResult.nextContinuationHandle;
          } catch (error) {
            failedItems.push(folderAddress);
            break;
          }
        } while (continuationHandle != null);

        completedItems.push(folderAddress);
      };

      const progressMap = new Map<string, number>();

      const updateProgressWithTotal = throttle(
        { interval: 250 },
        (resourceAddress: string, { transferredBytes }: Progress) => {
          progressMap.set(resourceAddress, transferredBytes);

          const transferredBytesSum = Array.from(progressMap.values()).reduce((acc, curr) => acc + curr, 0);
          const bytes = downloadStats.completedBytes + transferredBytesSum;

          const remainingTime = calculateRemainingTime({
            ...downloadStats,
            completedBytes: bytes,
          });

          const remainingTimeString =
            remainingTime !== Infinity
              ? remainingTime < 2
                ? "a few seconds left"
                : `${formatDistanceToNowStrict(new Date(Date.now() + remainingTime * 1000), {
                    roundingMethod: "ceil",
                  })} left`
              : "";

          const elapsedSeconds = (Date.now() - startTime) / 1000;
          const speed = elapsedSeconds > 0 ? bytes / elapsedSeconds : 0;

          const primaryMessage = `${formatBytes(bytes)} / ${formatBytes(downloadStats.totalBytes)}`;
          const secondaryMessage = `${formatBytes(speed)}/s, ${remainingTimeString}`;
          const percent =
            downloadStats.totalBytes === 0 ? 100 : Number((bytes / downloadStats.totalBytes).toFixed(2)) * 100;
          downloadNotification.progress(primaryMessage, secondaryMessage, percent);
        }
      );

      const updateProgressWithTransferred = throttle(
        { interval: 250 },
        (resourceAddress: string, { transferredBytes }: Progress) => {
          progressMap.set(resourceAddress, transferredBytes);
          const transferredBytesSum = Array.from(progressMap.values()).reduce((acc, curr) => acc + curr, 0);
          const bytes = downloadStats.completedBytes + transferredBytesSum;
          const elapsedSeconds = (Date.now() - startTime) / 1000;
          const rate = elapsedSeconds > 0 ? bytes / elapsedSeconds : 0;

          let filesMessage = "";
          if (downloadStats.completedFiles > 1) {
            filesMessage = `${downloadStats.completedFiles} files, `;
          } else if (downloadStats.completedFiles === 1) {
            filesMessage = "1 file, ";
          }

          const primaryMessage = `${filesMessage}${formatBytes(bytes)} transferred`;
          const secondaryMessage = `${formatBytes(rate)}/s`;

          downloadNotification.progress(primaryMessage, secondaryMessage);
        }
      );

      // total size is known only when downloading files. If folders are passed, the size is not available.
      const updateProgress = files.some((item) => item.isFolderLike)
        ? updateProgressWithTransferred
        : updateProgressWithTotal;

      async function downloadFile({
        resourceAddress,
        parentFolderHandle,
        fileSize,
      }: DownloadBatchItem): Promise<number> {
        const response = await fileObjectService(fileObjectApi!).download(resourceAddress, "address", {
          onProgress: (progressEvent: Progress) => updateProgress(resourceAddress, progressEvent),
          signal: downloadNotification.signal,
        });

        if (!response?.body) {
          throw new Error("No response body");
        }

        // write the file to the local directory
        const fileName = sanitizePathName(getPathName(resourceAddress));
        const fileHandle = await parentFolderHandle.getFileHandle(fileName, { create: true });
        const writeable = await fileHandle.createWritable();
        await response.body.pipeTo(writeable);

        progressMap.delete(resourceAddress);
        const sizeOnDisk = (await fileHandle.getFile()).size;

        if (fileSize !== sizeOnDisk) {
          await writeable.abort();
          await parentFolderHandle.removeEntry(fileName);
          throw new Error(`File size mismatch: server size: ${fileSize}, local size: ${sizeOnDisk}`);
        }

        return sizeOnDisk;
      }

      try {
        await Promise.all(
          files
            .filter((item) => item.type === PathType.File)
            .map((item) => {
              downloadStats.totalBytes += item.bytes;
              return batch.add([
                {
                  resourceAddress: item.resourceAddress,
                  parentFolderHandle: selectedFolderHandle,
                  fileSize: item.bytes,
                },
              ]);
            })
        );

        await Promise.all(
          files
            .filter((item) => item.isFolderLike)
            .map((item) => processFolder(item.resourceAddress, selectedFolderHandle))
        );

        await batch.flush();
      } catch {
        failedItems.push("unknown error");
      }

      if (downloadNotification.signal.aborted) {
        downloadNotification.cancel();
      } else if (failedItems.length === 0) {
        downloadNotification.complete(completedItems);
      } else if (completedItems.length === 0) {
        downloadNotification.fail(failedItems);
      } else {
        downloadNotification.completePartially(completedItems, failedItems);
      }
    },
    [connection, createServiceApi, requestTimeout, createDownloadNotification, batchSize]
  );

  return useMemo(
    () => ({
      download,
    }),
    [download]
  );
}

interface DownloadBatchItem {
  resourceAddress: string;
  parentFolderHandle: FileSystemDirectoryHandle;
  fileSize?: number;
}

interface DownloadStats {
  startTime: number;
  completedFiles: number;
  completedBytes: number;
  totalBytes: number;
}

/**
 * Calculates an estimate of the time remaining (in seconds) for the active download.
 */
function calculateRemainingTime(stats: DownloadStats) {
  if (stats.completedBytes === 0) {
    return Infinity;
  }
  const elapsedTime = Date.now() - stats.startTime;
  const remainingBytes = stats.totalBytes - stats.completedBytes;
  const remainingTime = (remainingBytes / stats.completedBytes) * elapsedTime;
  return remainingTime / 1000;
}
