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

import { ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { ListStatResult } from "@/services/storage/fileFolderService";
import { Connection, rootFolderPath } from "@/state/appStore";
import { PathType } from "@/state/path";
import { renderHook, waitFor } from "@/test-utils";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { formatDate } from "@/util/date";
import { FolderContent, useFolderContentQuery } from "./useFolderContentQuery";

const endpoint = "http://storage.local";
const testFolder = "file-storage://fileservice/test-folder/";

const mockAuth = createTestAuth();

const connection: Connection = {
  endpoint,
  currentFolder: testFolder,
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

vi.mock("@/state/appStore", () => ({
  useConnection: (endpoint: string) => (connection.endpoint === endpoint ? connection : undefined),
  rootFolderPath: "file-storage://fileservice",
}));

// Mock auth API options
vi.mock("@/services/api", async () => {
  const actual = await vi.importActual<typeof import("@/services/api")>("@/services/api");
  return {
    ...actual,
    getApiOptions: () => ({
      discovery: "",
      auth: {
        get: () => mockAuth,
        set: vi.fn(),
      },
    }),
  };
});

// Create storage service mocks
const mocks = vi.hoisted(() => ({
  mockStorageService: {
    fileFolder: {
      listStat: vi.fn(),
    },
    capabilities: {
      listTopLevelAddresses: vi.fn(),
    },
  },
}));

vi.mock("@/services/storage/capabilitiesService", () => ({
  capabilitiesService: () => mocks.mockStorageService.capabilities,
}));

vi.mock("@/services/storage/fileFolderService", () => ({
  fileFolderService: () => mocks.mockStorageService.fileFolder,
}));

vi.mock("@/hooks/useServiceApi", async () => {
  const { create } = await vi.importActual<typeof import("@/services/api")>("@/services/api");
  const getApi = (_name: string) =>
    Promise.resolve({
      url: "http://storage.local/v1alpha",
      baseUrl: "http://storage.local",
      versionPath: "/v1alpha",
    });
  return {
    useServiceApi: () => ({
      getApi: (_discovery: string, name: string) => getApi(name),
      createServiceApi: async (_discovery: string, name: string, options?: import("@/services/api").ApiOptions) => {
        const api = await getApi(name);
        return api ? create(api.url, options) : null;
      },
    }),
  };
});

describe("useFolderContentQuery", () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should fetch initial folder content", async () => {
    const mockResponse: ListStatResult = {
      folderAddress: testFolder,
      subfolderAddresses: [],
      isRoot: false,
      entries: [
        {
          resourceAddress: "file1.txt",
          resourceIdentity: "id1",
          metadata: {
            dataObjectSize: 1024,
            lastModifiedTimestamp: "2024-02-27T12:00:00Z",
          },
        },
      ],
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.fileFolder.listStat.mockResolvedValueOnce(mockResponse);

    const { result } = renderHook(() => useFolderContentQuery(endpoint, testFolder), { wrapper });

    // Should start in loading state
    expect(result.current.isLoading).toBe(true);

    // Wait for data to load
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    // Verify correct data is returned
    expect(result.current.data?.pages[0]).toEqual({
      isRoot: false,
      files: new Map([
        [
          "file1.txt",
          {
            name: "file1.txt",
            type: PathType.File,
            displayedType: "TXT",
            isFolderLike: false,
            size: "1 KiB",
            bytes: 1024,
            modified: formatDate("2024-02-27T12:00:00Z"),
            resourceAddress: "file1.txt",
            resourceIdentity: "id1",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-02-27T12:00:00Z",
            },
          },
        ],
      ]),
      folders: new Map(),
      nextContinuationHandle: null,
    } as FolderContent);
    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledWith(testFolder, null);
  });

  it("should fetch top-level addresses for root folder", async () => {
    const mockResponse: ListStatResult = {
      folderAddress: rootFolderPath,
      subfolderAddresses: ["address1", "address2"],
      isRoot: true,
      entries: [],
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.capabilities.listTopLevelAddresses.mockResolvedValueOnce(mockResponse);

    const { result } = renderHook(() => useFolderContentQuery(endpoint, rootFolderPath), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data?.pages[0]).toEqual({
      isRoot: true,
      folders: new Map([
        [
          "address1",
          {
            name: "address1",
            type: PathType.TopLevelAddress,
            displayedType: "Bucket",
            isFolderLike: true,
            resourceAddress: "address1",
          },
        ],
        [
          "address2",
          {
            name: "address2",
            type: PathType.TopLevelAddress,
            displayedType: "Bucket",
            isFolderLike: true,
            resourceAddress: "address2",
          },
        ],
      ]),
      files: new Map(),
      nextContinuationHandle: null,
    } as FolderContent);
    expect(mocks.mockStorageService.capabilities.listTopLevelAddresses).toHaveBeenCalled();
    expect(mocks.mockStorageService.fileFolder.listStat).not.toHaveBeenCalled();
  });

  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useFolderContentQuery(endpoint, testFolder, false), { wrapper });

    expect(result.current.isLoading).toBe(false);
    expect(mocks.mockStorageService.fileFolder.listStat).not.toHaveBeenCalled();
  });

  it("should automatically fetch next pages when available", async () => {
    // First page response
    const firstPage: ListStatResult = {
      folderAddress: testFolder,
      subfolderAddresses: [],
      isRoot: false,
      entries: [
        {
          resourceAddress: "file1.txt",
          resourceIdentity: "id1",
          metadata: {
            dataObjectSize: 1024,
            lastModifiedTimestamp: "2024-02-27T12:00:00Z",
          },
        },
      ],
      nextContinuationHandle: "page2",
    };

    // Second page response
    const secondPage: ListStatResult = {
      folderAddress: testFolder,
      subfolderAddresses: [],
      isRoot: false,
      entries: [
        {
          resourceAddress: "file2.txt",
          resourceIdentity: "id2",
          metadata: {
            dataObjectSize: 2048,
            lastModifiedTimestamp: "2024-02-27T13:00:00Z",
          },
        },
      ],
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.fileFolder.listStat.mockResolvedValueOnce(firstPage).mockResolvedValueOnce(secondPage);

    const { result } = renderHook(() => useFolderContentQuery(endpoint, testFolder), { wrapper });

    await waitFor(() => expect(result.current.data?.pages).toHaveLength(2));
    expect(result.current.data?.pages[1]).toEqual({
      isRoot: false,
      folders: new Map(),
      files: new Map([
        [
          "file2.txt",
          {
            name: "file2.txt",
            type: PathType.File,
            displayedType: "TXT",
            isFolderLike: false,
            size: "2 KiB",
            bytes: 2048,
            modified: formatDate("2024-02-27T13:00:00Z"),
            resourceAddress: "file2.txt",
            resourceIdentity: "id2",
            metadata: {
              dataObjectSize: 2048,
              lastModifiedTimestamp: "2024-02-27T13:00:00Z",
            },
          },
        ],
      ]),
      nextContinuationHandle: null,
    } as FolderContent);

    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledTimes(2);
    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledWith(testFolder, null);
    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledWith(testFolder, "page2");
  });

  it("should handle empty folders", async () => {
    const emptyResponse: ListStatResult = {
      isRoot: false,
      subfolderAddresses: [],
      entries: [],
      folderAddress: testFolder,
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.fileFolder.listStat.mockResolvedValueOnce(emptyResponse);

    const { result } = renderHook(() => useFolderContentQuery(endpoint, testFolder), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data?.pages[0].folders.size).toBe(0);
    expect(result.current.data?.pages[0].files.size).toBe(0);
    expect(result.current.hasNextPage).toBe(false);
  });

  it("should handle API errors", async () => {
    const error = new Error("API Error");
    mocks.mockStorageService.fileFolder.listStat.mockRejectedValueOnce(error);

    const { result } = renderHook(() => useFolderContentQuery(endpoint, testFolder), { wrapper });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.error).toBe(error);
  });

  it("should update query when folder path changes", async () => {
    const folder1 = "file-storage://fileservice/folder1";
    const folder2 = "file-storage://fileservice/folder2";

    const response1: ListStatResult = {
      folderAddress: folder1,
      subfolderAddresses: [],
      isRoot: false,
      entries: [
        {
          resourceAddress: "file1.txt",
          resourceIdentity: "id1",
          metadata: {
            dataObjectSize: 1024,
            lastModifiedTimestamp: "2024-02-27T12:00:00Z",
          },
        },
      ],
      nextContinuationHandle: null,
    };

    const response2: ListStatResult = {
      folderAddress: folder2,
      subfolderAddresses: [],
      isRoot: false,
      entries: [
        {
          resourceAddress: "file2.txt",
          resourceIdentity: "id2",
          metadata: {
            dataObjectSize: 2048,
            lastModifiedTimestamp: "2024-02-27T13:00:00Z",
          },
        },
      ],
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.fileFolder.listStat.mockResolvedValueOnce(response1).mockResolvedValueOnce(response2);

    const { result, rerender } = renderHook(({ folderPath }) => useFolderContentQuery(endpoint, folderPath), {
      wrapper,
      initialProps: { folderPath: folder1 },
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.pages[0]).toEqual({
      isRoot: false,
      folders: new Map(),
      files: new Map([
        [
          "file1.txt",
          {
            name: "file1.txt",
            type: PathType.File,
            displayedType: "TXT",
            isFolderLike: false,
            size: "1 KiB",
            bytes: 1024,
            modified: formatDate("2024-02-27T12:00:00Z"),
            resourceAddress: "file1.txt",
            resourceIdentity: "id1",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-02-27T12:00:00Z",
            },
          },
        ],
      ]),
      nextContinuationHandle: null,
    } as FolderContent);

    rerender({ folderPath: folder2 });

    await waitFor(() =>
      expect(result.current.data?.pages[0]).toEqual({
        isRoot: false,
        folders: new Map(),
        files: new Map([
          [
            "file2.txt",
            {
              name: "file2.txt",
              type: PathType.File,
              displayedType: "TXT",
              isFolderLike: false,
              size: "2 KiB",
              bytes: 2048,
              modified: formatDate("2024-02-27T13:00:00Z"),
              resourceAddress: "file2.txt",
              resourceIdentity: "id2",
              metadata: {
                dataObjectSize: 2048,
                lastModifiedTimestamp: "2024-02-27T13:00:00Z",
              },
            },
          ],
        ]),
        nextContinuationHandle: null,
      } as FolderContent)
    );
  });

  it("should refetch when navigating back to a previously visited folder", async () => {
    const folder1 = "file-storage://fileservice/folder1/";
    const folder2 = "file-storage://fileservice/folder2/";

    const response1: ListStatResult = {
      folderAddress: folder1,
      subfolderAddresses: [],
      isRoot: false,
      entries: [],
      nextContinuationHandle: null,
    };

    const response2: ListStatResult = {
      folderAddress: folder2,
      subfolderAddresses: [],
      isRoot: false,
      entries: [],
      nextContinuationHandle: null,
    };

    mocks.mockStorageService.fileFolder.listStat
      .mockResolvedValueOnce(response1)
      .mockResolvedValueOnce(response2)
      .mockResolvedValueOnce(response1);

    const { result, rerender } = renderHook(({ folderPath }) => useFolderContentQuery(endpoint, folderPath), {
      wrapper,
      initialProps: { folderPath: folder1 },
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledTimes(1);

    rerender({ folderPath: folder2 });
    await waitFor(() => expect(result.current.data?.pages[0].folders.size).toBe(0));
    expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledTimes(2);

    rerender({ folderPath: folder1 });
    await waitFor(() => expect(mocks.mockStorageService.fileFolder.listStat).toHaveBeenCalledTimes(3));
  });
});
