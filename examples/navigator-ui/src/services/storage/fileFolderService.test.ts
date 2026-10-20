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

import { describe, expect, it, vi } from "vitest";
import type { ApiInstance } from "../api";
import { fileFolderService, ListStatResult } from "./fileFolderService";

describe("fileFolderService", () => {
  const mockGet = vi.fn();
  const mockDelete = vi.fn();
  const mockApi: ApiInstance = {
    get: mockGet,
    delete: mockDelete,
  } as unknown as ApiInstance;

  beforeEach(() => {
    mockGet.mockReset();
    mockDelete.mockReset();
  });

  describe("list", () => {
    it("should handle single page of results", async () => {
      const mockResponse = {
        subfolderAddresses: ["folder1", "folder2"],
        subResourceAddresses: ["resource1", "resource2"],
        nextContinuationHandle: null,
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = fileFolderService(mockApi);
      const result = await service.list("test-folder", null);

      expect(mockGet).toHaveBeenCalledWith("filefolder/list/test-folder?max_page_size=1000");
      expect(result).toEqual({
        folderAddress: "test-folder",
        isTopLevelAddress: false,
        subfolderAddresses: ["folder1", "folder2"],
        subResourceAddresses: ["resource1", "resource2"],
        nextContinuationHandle: null,
      });
    });

    it("should handle multiple pages of results", async () => {
      const mockResponses = [
        {
          subfolderAddresses: ["folder1"],
          subResourceAddresses: ["resource1"],
          nextContinuationHandle: "handle1",
        },
        {
          subfolderAddresses: ["folder2"],
          subResourceAddresses: ["resource2"],
          nextContinuationHandle: null,
        },
      ];

      mockGet
        .mockResolvedValueOnce({
          json: () => Promise.resolve(mockResponses[0]),
        })
        .mockResolvedValueOnce({
          json: () => Promise.resolve(mockResponses[1]),
        });

      const service = fileFolderService(mockApi);
      const firstPage = await service.list("test-folder", null);
      const secondPage = await service.list("test-folder", firstPage.nextContinuationHandle);

      expect(mockGet).toHaveBeenCalledTimes(2);
      expect(mockGet).toHaveBeenNthCalledWith(1, "filefolder/list/test-folder?max_page_size=1000");
      expect(mockGet).toHaveBeenNthCalledWith(
        2,
        "filefolder/list/test-folder?max_page_size=1000&continuation_handle=handle1"
      );

      expect(firstPage).toEqual({
        folderAddress: "test-folder",
        isTopLevelAddress: false,
        subfolderAddresses: ["folder1"],
        subResourceAddresses: ["resource1"],
        nextContinuationHandle: "handle1",
      });

      expect(secondPage).toEqual({
        folderAddress: "test-folder",
        isTopLevelAddress: false,
        subfolderAddresses: ["folder2"],
        subResourceAddresses: ["resource2"],
        nextContinuationHandle: null,
      });
    });

    it("should handle API errors", async () => {
      mockGet.mockRejectedValueOnce(new Error("API Error"));

      const service = fileFolderService(mockApi);
      await expect(service.list("test-folder", null)).rejects.toThrow("API Error");
    });
  });

  describe("listStat", () => {
    it("should handle single page of results", async () => {
      const mockResponse = {
        subfolderAddresses: ["folder1", "folder2"],
        entries: [
          {
            resourceAddress: "resource1",
            resourceIdentity: "id1",
            metadata: {
              dataObjectSize: 1000,
              lastModifiedTimestamp: "2024-02-27T12:00:00Z",
            },
          },
        ],
        nextContinuationHandle: null,
      } as ListStatResult;

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = fileFolderService(mockApi);
      const result = await service.listStat("test-folder", null);

      expect(result).toEqual({
        folderAddress: "test-folder",
        isRoot: false,
        subfolderAddresses: ["folder1", "folder2"],
        entries: [
          {
            resourceAddress: "resource1",
            resourceIdentity: "id1",
            metadata: {
              dataObjectSize: 1000,
              lastModifiedTimestamp: "2024-02-27T12:00:00Z",
            },
          },
        ],
        nextContinuationHandle: null,
      } as ListStatResult);

      expect(mockGet).toHaveBeenCalledWith("filefolder/liststat/test-folder?max_page_size=1000");
    });

    it("should handle multiple pages of results", async () => {
      const mockResponses = [
        {
          subfolderAddresses: ["folder1"],
          entries: [
            {
              resourceAddress: "resource1",
              resourceIdentity: "id1",
              metadata: {
                dataObjectSize: 1000,
                lastModifiedTimestamp: "2024-02-27T12:00:00Z",
              },
            },
          ],
          nextContinuationHandle: "handle1",
        },
        {
          subfolderAddresses: ["folder2"],
          entries: [
            {
              resourceAddress: "resource2",
              resourceIdentity: "id2",
              metadata: {
                dataObjectSize: 2000,
                lastModifiedTimestamp: "2024-02-27T13:00:00Z",
              },
            },
          ],
          nextContinuationHandle: null,
        },
      ] as ListStatResult[];

      mockGet
        .mockResolvedValueOnce({
          json: () => Promise.resolve(mockResponses[0]),
        })
        .mockResolvedValueOnce({
          json: () => Promise.resolve(mockResponses[1]),
        });

      const service = fileFolderService(mockApi);
      const firstPage = await service.listStat("test-folder", null);
      const secondPage = await service.listStat("test-folder", firstPage.nextContinuationHandle);

      expect(mockGet).toHaveBeenCalledTimes(2);
      expect(mockGet).toHaveBeenNthCalledWith(1, "filefolder/liststat/test-folder?max_page_size=1000");
      expect(mockGet).toHaveBeenNthCalledWith(
        2,
        "filefolder/liststat/test-folder?max_page_size=1000&continuation_handle=handle1"
      );

      expect(firstPage).toEqual({
        folderAddress: "test-folder",
        isRoot: false,
        subfolderAddresses: ["folder1"],
        entries: [
          {
            resourceAddress: "resource1",
            resourceIdentity: "id1",
            metadata: {
              dataObjectSize: 1000,
              lastModifiedTimestamp: "2024-02-27T12:00:00Z",
            },
          },
        ],
        nextContinuationHandle: "handle1",
      } as ListStatResult);

      expect(secondPage).toEqual({
        folderAddress: "test-folder",
        isRoot: false,
        subfolderAddresses: ["folder2"],
        entries: [
          {
            resourceAddress: "resource2",
            resourceIdentity: "id2",
            metadata: {
              dataObjectSize: 2000,
              lastModifiedTimestamp: "2024-02-27T13:00:00Z",
            },
          },
        ],
        nextContinuationHandle: null,
      } as ListStatResult);
    });

    it("should handle API errors", async () => {
      mockGet.mockRejectedValueOnce(new Error("API Error"));

      const service = fileFolderService(mockApi);
      await expect(service.listStat("test-folder", null)).rejects.toThrow("API Error");
    });
  });

  describe("deleteFolder", () => {
    it("should delete a folder", async () => {
      mockDelete.mockResolvedValueOnce({
        status: 204,
      });

      const service = fileFolderService(mockApi);
      await service.deleteFolder("test-folder/subfolder");

      expect(mockDelete).toHaveBeenCalledWith("filefolder/list/test-folder%2Fsubfolder");
    });

    it("should throw on unexpected status", async () => {
      mockDelete.mockResolvedValueOnce({
        status: 200,
      });

      const service = fileFolderService(mockApi);

      await expect(service.deleteFolder("test-folder")).rejects.toThrow("Unexpected response status: 200");
    });
  });
});
