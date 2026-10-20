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
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OmniEvent } from "@/services/notificationService";
import * as appStore from "@/state/appStore";
import { Connection } from "@/state/appStore";
import { FileListItem, FolderListItem } from "@/state/path";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { FolderContent } from "./useFolderContentQuery";
import { useStorageSubscription } from "./useStorageSubscription";

const mockShow = vi.fn();
vi.mock("./useNotification", () => ({
  useNotification: () => ({
    show: mockShow,
  }),
}));

const mockPathSubscription = {
  onEvent: vi.fn((_onNotificationEvent: (event: OmniEvent) => void) => vi.fn()),
};

const mockSetQueryData = vi.fn();
const mockQueryClient = {
  setQueryData: mockSetQueryData,
};

vi.mock("@tanstack/react-query", async () => {
  const actual = await vi.importActual("@tanstack/react-query");
  return {
    ...actual,
    useQueryClient: () => mockQueryClient,
  };
});

const mockStat = vi.fn();
vi.mock("@/services/storage", () => ({
  fileObjectService: () => ({
    stat: mockStat,
  }),
}));

vi.mock("@/hooks/useServiceApi", async () => {
  const { create } = await vi.importActual<typeof import("@/services/api")>("@/services/api");
  const getApi = (_name: string) =>
    Promise.resolve({
      url: "https://test-endpoint.com/v1alpha",
      baseUrl: "https://test-endpoint.com",
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

beforeEach(() => {
  vi.clearAllMocks();
  mockStat.mockResolvedValue({
    metadata: {
      dataObjectSize: 1024,
      lastModifiedTimestamp: "2024-03-20T12:00:00Z",
    },
    resourceIdentity: "test-resource-id",
  });
});

const mockAuth = createTestAuth();

const mockActiveConnection: Connection = {
  endpoint: "https://test-endpoint.com",
  currentFolder: "/test-folder",
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

const wrapper = ({ children }: { children: ReactNode }) => {
  return <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>;
};

const createStorageEvent = (
  eventType: "omni.storage.created" | "omni.storage.deleted" | "omni.storage.dir_created" | "omni.storage.dir_deleted",
  resourceAddress: string,
  resourceIdentity: string
): OmniEvent => ({
  eventType,
  principalIdentity: "test-user",
  occurredAt: "2023-01-01T00:00:00Z",
  publishedAt: "2023-01-01T00:00:00Z",
  message: {
    resourceIdentity,
    resourceAddress,
  },
});

const createEmptyCacheData = () => ({
  pages: [
    {
      files: new Map<string, FileListItem>(),
      folders: new Map<string, FolderListItem>(),
      isRoot: false,
      nextContinuationHandle: null,
    },
  ],
});

const createTestCacheData = () => ({
  pages: [
    {
      files: new Map([["test-resource-address", { resourceAddress: "test-resource-address" } as FileListItem]]),
      folders: new Map([["test-resource-address", { resourceAddress: "test-resource-address" } as FolderListItem]]),
      isRoot: false,
      nextContinuationHandle: "page2",
    },
    {
      files: new Map([["test-resource-address2", { resourceAddress: "test-resource-address2" } as FileListItem]]),
      folders: new Map([["test-resource-address2", { resourceAddress: "test-resource-address2" } as FolderListItem]]),
      isRoot: false,
      nextContinuationHandle: null,
    },
  ],
});

describe("useStorageSubscription", () => {
  let cacheData: { pages: FolderContent[] };

  beforeEach(() => {
    vi.clearAllMocks();
    cacheData = createTestCacheData();
    vi.spyOn(appStore, "useActiveConnection").mockReturnValue(mockActiveConnection);
  });

  describe("omni.storage.created events", () => {
    it("should add a new file to the last page of the cache", async () => {
      const emptyCacheData = createEmptyCacheData();
      const createFileEvent = createStorageEvent("omni.storage.created", "new-file-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(emptyCacheData);
        const lastPage = newData.pages[newData.pages.length - 1];
        expect(lastPage.files.has("new-file-address")).toBe(true);
        expect(lastPage.files.get("new-file-address")).toEqual(
          expect.objectContaining({
            resourceAddress: "new-file-address",
            resourceIdentity: "test-resource-id",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-03-20T12:00:00Z",
            },
          })
        );
      });
    });

    it("should return undefined when oldData is undefined", async () => {
      const createFileEvent = createStorageEvent("omni.storage.created", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] } | undefined) => {
          pages: FolderContent[];
        };
        const newData = updateFn(undefined);
        expect(newData).toBeUndefined();
      });
    });

    it("should update an existing file in any page of the cache", async () => {
      const existingFile = {
        resourceAddress: "test-resource-address",
        resourceIdentity: "old-resource-id",
        metadata: {
          dataObjectSize: 512,
          lastModifiedTimestamp: "2023-01-01T00:00:00Z",
        },
      };

      cacheData.pages[0].files.set("test-resource-address", existingFile as FileListItem);

      const createFileEvent = createStorageEvent("omni.storage.created", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cacheData);
        const page0 = newData.pages[0];
        const page1 = newData.pages[1];
        expect(page0.files.size).toBe(1);
        expect(page1.files.size).toBe(1);
        expect(page0.files.get("test-resource-address")).toEqual(
          expect.objectContaining({
            resourceAddress: "test-resource-address",
            resourceIdentity: "test-resource-id",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-03-20T12:00:00Z",
            },
          })
        );
      });
    });

    it("should update an existing file when the notification address matches the cache key exactly", async () => {
      const encodedAddress = "https://test-endpoint.com/test-folder/foreign%20(1)";
      const existingFile = {
        resourceAddress: encodedAddress,
        resourceIdentity: "old-resource-id",
        metadata: {
          dataObjectSize: 512,
          lastModifiedTimestamp: "2023-01-01T00:00:00Z",
        },
      };

      const cache = createEmptyCacheData();
      cache.pages[0].files.set(encodedAddress, existingFile as FileListItem);

      const createFileEvent = createStorageEvent("omni.storage.created", encodedAddress, "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cache);
        const page0 = newData.pages[0];

        expect(page0.files.size).toBe(1);
        expect(page0.files.get(encodedAddress)).toEqual(
          expect.objectContaining({
            resourceAddress: encodedAddress,
            resourceIdentity: "test-resource-id",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-03-20T12:00:00Z",
            },
          })
        );
      });
    });

    it("should update an existing file when the notification address is encoded but the cache key is decoded", async () => {
      const encodedAddress = "https://test-endpoint.com/test-folder/image%20(1).png";
      const decodedAddress = "https://test-endpoint.com/test-folder/image (1).png";
      const existingFile = {
        resourceAddress: decodedAddress,
        resourceIdentity: "old-resource-id",
        metadata: {
          dataObjectSize: 512,
          lastModifiedTimestamp: "2023-01-01T00:00:00Z",
        },
      };

      const cache = createEmptyCacheData();
      cache.pages[0].files.set(decodedAddress, existingFile as FileListItem);

      // Notification arrives with encoded address; cache was populated from listStat with decoded address.
      const createFileEvent = createStorageEvent("omni.storage.created", encodedAddress, "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cache);
        const page0 = newData.pages[0];

        // Should update in-place, not append a duplicate.
        expect(page0.files.size).toBe(1);
        expect(page0.files.get(decodedAddress)).toEqual(
          expect.objectContaining({
            resourceAddress: decodedAddress,
            resourceIdentity: "test-resource-id",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-03-20T12:00:00Z",
            },
          })
        );
      });
    });

    it("should update an existing file when the notification parentheses are not encoded but the cache key is", async () => {
      const encodedAddress = "https://test-endpoint.com/test-folder/image%20%281%29.png";
      const notificationAddress = "https://test-endpoint.com/test-folder/image (1).png";
      const existingFile = {
        resourceAddress: encodedAddress,
        resourceIdentity: "old-resource-id",
        metadata: {
          dataObjectSize: 512,
          lastModifiedTimestamp: "2023-01-01T00:00:00Z",
        },
      };

      const cache = createEmptyCacheData();
      cache.pages[0].files.set(encodedAddress, existingFile as FileListItem);

      // Notification arrives with literal parentheses; cache was populated from listStat with encoded parentheses.
      const createFileEvent = createStorageEvent("omni.storage.created", notificationAddress, "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFileEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cache);
        const page0 = newData.pages[0];

        // Should update in-place, not append a duplicate under the decoded key.
        expect(page0.files.size).toBe(1);
        expect(page0.files.get(encodedAddress)).toEqual(
          expect.objectContaining({
            resourceAddress: encodedAddress,
            resourceIdentity: "test-resource-id",
            metadata: {
              dataObjectSize: 1024,
              lastModifiedTimestamp: "2024-03-20T12:00:00Z",
            },
          })
        );
      });
    });
  });

  describe("omni.storage.deleted events", () => {
    it("should remove a file from the cache when deleted", async () => {
      const deleteEvent = createStorageEvent("omni.storage.deleted", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(deleteEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cacheData);
        expect(newData.pages[0].files.has("test-resource-address")).toBe(false);
      });
    });

    it("should return undefined when oldData is undefined", async () => {
      const deleteEvent = createStorageEvent("omni.storage.deleted", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(deleteEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] } | undefined) => {
          pages: FolderContent[];
        };
        const newData = updateFn(undefined);
        expect(newData).toBeUndefined();
      });
    });
  });

  describe("omni.storage.dir_created events", () => {
    it("should update an existing folder in the cache", async () => {
      const createFolderEvent = createStorageEvent(
        "omni.storage.dir_created",
        "test-resource-address",
        "test-resource-id"
      );

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFolderEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cacheData);
        expect(newData.pages[0].folders.get("test-resource-address")).toEqual(
          expect.objectContaining({
            resourceAddress: "test-resource-address",
            type: "Folder",
            isFolderLike: true,
          })
        );
      });
    });

    it("should return undefined when oldData is undefined", async () => {
      const createFolderEvent = createStorageEvent(
        "omni.storage.dir_created",
        "test-resource-address",
        "test-resource-id"
      );

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFolderEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] } | undefined) => {
          pages: FolderContent[];
        };
        const newData = updateFn(undefined);
        expect(newData).toBeUndefined();
      });
    });

    it("should add a new folder to the last page of the cache", async () => {
      const emptyCacheData = createEmptyCacheData();
      const createFolderEvent = createStorageEvent(
        "omni.storage.dir_created",
        "new-folder-address",
        "test-resource-id"
      );

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createFolderEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(emptyCacheData);
        const lastPage = newData.pages[newData.pages.length - 1];
        expect(lastPage.folders.has("new-folder-address")).toBe(true);
        expect(lastPage.folders.get("new-folder-address")).toEqual(
          expect.objectContaining({
            resourceAddress: "new-folder-address",
            type: "Folder",
            isFolderLike: true,
          })
        );
      });
    });
  });

  describe("omni.storage.dir_deleted events", () => {
    it("should remove a folder from the cache when deleted", async () => {
      const deleteFolderEvent = createStorageEvent(
        "omni.storage.dir_deleted",
        "test-resource-address",
        "test-resource-id"
      );

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(deleteFolderEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] }) => {
          pages: FolderContent[];
        };
        const newData = updateFn(cacheData);
        expect(newData.pages[0].folders.has("test-resource-address")).toBe(false);
      });
    });

    it("should return undefined when oldData is undefined", async () => {
      const deleteFolderEvent = createStorageEvent(
        "omni.storage.dir_deleted",
        "test-resource-address",
        "test-resource-id"
      );

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(deleteFolderEvent);
      });

      await waitFor(() => {
        const updateFn = mockSetQueryData.mock.calls[0][1] as (oldData: { pages: FolderContent[] } | undefined) => {
          pages: FolderContent[];
        };
        const newData = updateFn(undefined);
        expect(newData).toBeUndefined();
      });
    });
  });

  describe("non-storage events", () => {
    it("should ignore thumbnail association events", () => {
      const thumbnailEvent: OmniEvent = {
        eventType: "omni.thumbnail.associated",
        principalIdentity: "test-user",
        occurredAt: "2023-01-01T00:00:00Z",
        publishedAt: "2023-01-01T00:00:00Z",
        message: {
          associationTarget: {
            resourceAddress: "test-resource-address",
            resourceIdentity: "test-resource-address?version-id=target",
          },
          targetType: "FILE",
          thumbnail: {
            resourceAddress: "thumbnail-address",
            resourceIdentity: "thumbnail-resource-id",
          },
        },
      };

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(thumbnailEvent);
      });

      expect(mockSetQueryData).not.toHaveBeenCalled();
      expect(mockStat).not.toHaveBeenCalled();
    });
  });

  describe("error handling", () => {
    it("should display an error notification when file stats cannot be retrieved", async () => {
      const error = new Error("Failed to get file stats");
      mockStat.mockRejectedValueOnce(error);

      const createEvent = createStorageEvent("omni.storage.created", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createEvent);
      });

      await waitFor(() => {
        expect(mockShow).toHaveBeenCalledWith({
          type: "error",
          title: 'Error handling omni.storage.created event for "test-resource-address"',
          message: "Path content may be out of date. Please refresh the page.",
        });
      });
    });

    it("should display HTTP error details in the notification", async () => {
      const httpError = new Error("Not Found") as Error & { name: string; response: { status: number } };
      httpError.name = "HTTPError";
      httpError.response = { status: 404 };
      mockStat.mockRejectedValueOnce(httpError);

      const createEvent = createStorageEvent("omni.storage.created", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createEvent);
      });

      await waitFor(() => {
        expect(mockShow).toHaveBeenCalledWith({
          type: "error",
          title: 'Error handling omni.storage.created event for "test-resource-address"',
          message: "Path content may be out of date. Please refresh the page.",
        });
      });
    });

    it("should handle non-Error objects in error notification", async () => {
      mockStat.mockRejectedValueOnce("string error");

      const createEvent = createStorageEvent("omni.storage.created", "test-resource-address", "test-resource-id");

      renderHook(() => useStorageSubscription(mockActiveConnection, mockPathSubscription), { wrapper });

      const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
      act(() => {
        eventHandler(createEvent);
      });

      await waitFor(() => {
        expect(mockShow).toHaveBeenCalledWith({
          type: "error",
          title: 'Error handling omni.storage.created event for "test-resource-address"',
          message: "Path content may be out of date. Please refresh the page.",
        });
      });
    });
  });
});
