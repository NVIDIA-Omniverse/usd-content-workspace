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
import { act, renderHook } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as appStore from "@/state/appStore";
import { Connection } from "@/state/appStore";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { useDeletePathMutation } from "./useDeletePathMutation";

const createMockNotification = () => ({
  id: "test-notification-id",
  update: vi.fn(),
  complete: vi.fn(),
  completePartially: vi.fn(),
  fail: vi.fn(),
  signal: { aborted: false },
});

const mockFileActionNotification = vi.fn(() => {
  return createMockNotification();
});

vi.mock("./useFileActionNotification", () => ({
  useFileActionNotification: () => () => mockFileActionNotification(),
}));

const server = setupServer();
beforeEach(() => {
  server.listen();
  vi.clearAllMocks();
});

afterEach(() => {
  server.resetHandlers();
  server.close();
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

const { mockCreateServiceApiDiscovery } = vi.hoisted(() => ({
  mockCreateServiceApiDiscovery: vi.fn<(discovery: string) => void>(),
}));

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
      createServiceApi: async (discovery: string, name: string, options?: import("@/services/api").ApiOptions) => {
        mockCreateServiceApiDiscovery(discovery);
        const api = await getApi(name);
        return api ? create(api.url, options) : null;
      },
    }),
  };
});

const wrapper = ({ children }: { children: ReactNode }) => {
  return <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>;
};

describe("useDeletePathMutation", () => {
  beforeEach(() => {
    const addConnection = appStore.useAppStore.getState().addConnection;
    const setActiveConnection = appStore.useAppStore.getState().setActiveConnection;

    act(() => {
      addConnection("https://test-endpoint.com", mockActiveConnection.info);
      setActiveConnection("https://test-endpoint.com");
    });
  });

  it("should delete a single file successfully", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-file.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const testFile = createMockFile("test-file.txt");

    await act(async () => {
      await result.current.mutateAsync([testFile]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(testNotification.update).toHaveBeenCalledWith("test-file.txt");
    expect(testNotification.complete).toHaveBeenCalledWith(["test-file.txt"]);
    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should delete a folder with nested content", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/filefolder/list/test-folder", () => {
        return HttpResponse.json({
          sub_resource_addresses: ["test-folder/file1.txt", "test-folder/file2.txt"],
          subfolder_addresses: ["test-folder/subfolder"],
          next_continuation_handle: null,
        });
      }),

      http.get("https://test-endpoint.com/v1alpha/filefolder/list/test-folder%2Fsubfolder", () => {
        return HttpResponse.json({
          sub_resource_addresses: ["test-folder/subfolder/file3.txt"],
          subfolder_addresses: [],
          next_continuation_handle: null,
        });
      }),

      // File deletions
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-folder%2Ffile1.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-folder%2Ffile2.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-folder%2Fsubfolder%2Ffile3.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),

      // Folder deletions
      http.delete("https://test-endpoint.com/v1alpha/filefolder/list/test-folder%2Fsubfolder", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/filefolder/list/test-folder", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const testFolder = createMockFolder("test-folder");

    await act(async () => {
      await result.current.mutateAsync([testFolder]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();

    // Verify update was called for each resource being deleted
    expect(testNotification.update).toHaveBeenCalledWith("test-folder/file1.txt");
    expect(testNotification.update).toHaveBeenCalledWith("test-folder/file2.txt");
    expect(testNotification.update).toHaveBeenCalledWith("test-folder/subfolder/file3.txt");
    expect(testNotification.update).toHaveBeenCalledWith("test-folder/subfolder");
    expect(testNotification.update).toHaveBeenCalledWith("test-folder");

    // Verify complete was called with all the completed resources
    expect(testNotification.complete).toHaveBeenCalledWith([
      "test-folder/file1.txt",
      "test-folder/file2.txt",
      "test-folder/subfolder/file3.txt",
      "test-folder/subfolder",
      "test-folder",
    ]);

    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should handle partial failures when some resources fail to delete", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/file1.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/file2.txt", () => {
        return HttpResponse.json({}, { status: 500 }); // This file will fail to delete
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const file1 = createMockFile("file1.txt");
    const file2 = createMockFile("file2.txt");

    await act(async () => {
      await result.current.mutateAsync([file1, file2]);
    });

    // Verify update was called for each resource
    expect(testNotification.update).toHaveBeenCalledWith("file1.txt");
    expect(testNotification.update).toHaveBeenCalledWith("file2.txt");

    // Verify completePartially was called with correct arguments
    expect(testNotification.completePartially).toHaveBeenCalledWith(
      ["file1.txt"], // completed resources
      ["file2.txt"] // failed resources
    );

    expect(testNotification.complete).not.toHaveBeenCalled();
    expect(testNotification.fail).not.toHaveBeenCalled();
  });

  it("should handle complete failure when all resources fail to delete", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/file1.txt", () => {
        return HttpResponse.json({}, { status: 500 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/file2.txt", () => {
        return HttpResponse.json({}, { status: 500 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/file3.txt", () => {
        return HttpResponse.json({}, { status: 500 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const file1 = createMockFile("file1.txt");
    const file2 = createMockFile("file2.txt");
    const file3 = createMockFile("file3.txt");

    await act(async () => {
      await result.current.mutateAsync([file1, file2, file3]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();

    // Verify update was called for each resource
    expect(testNotification.update).toHaveBeenCalledWith("file1.txt");
    expect(testNotification.update).toHaveBeenCalledWith("file2.txt");
    expect(testNotification.update).toHaveBeenCalledWith("file3.txt");

    // Verify fail was called with the array of failed resources
    expect(testNotification.fail).toHaveBeenCalledWith(["file1.txt", "file2.txt", "file3.txt"]);

    expect(testNotification.complete).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should handle cancellation during deletion process", async () => {
    // Create a custom notification with aborted signal
    const abortedNotification = {
      id: "test-notification-id",
      update: vi.fn(),
      complete: vi.fn(),
      completePartially: vi.fn(),
      fail: vi.fn(),
      signal: { aborted: true }, // Pre-aborted
    };
    mockFileActionNotification.mockReturnValue(abortedNotification);

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-file.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const testFile = createMockFile("test-file.txt");

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    // Execute the mutation with the aborted signal already set
    await act(async () => {
      await result.current.mutateAsync([testFile]);
    });

    expect(abortedNotification.complete).not.toHaveBeenCalled();
    expect(abortedNotification.fail).not.toHaveBeenCalled();
    expect(abortedNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should handle folder listing errors", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/filefolder/list/error-folder", () => {
        return new HttpResponse(null, { status: 500 });
      }),
      // Make sure the folder deletion attempt also fails if it happens
      http.delete("https://test-endpoint.com/v1alpha/filefolder/list/error-folder", () => {
        return new HttpResponse(null, { status: 500 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const errorFolder = createMockFolder("error-folder");

    await act(async () => {
      await result.current.mutateAsync([errorFolder]);
    });

    expect(testNotification.fail).toHaveBeenCalledWith(["error-folder"]);
    expect(testNotification.complete).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should handle pagination when listing folder contents", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      // First page of folder listing
      http.get("https://test-endpoint.com/v1alpha/filefolder/list/paginated-folder", ({ request }) => {
        const url = new URL(request.url);
        const continuationHandle = url.searchParams.get("continuation_handle");

        if (!continuationHandle) {
          return HttpResponse.json({
            sub_resource_addresses: ["paginated-folder/file1.txt"],
            subfolder_addresses: [],
            next_continuation_handle: "page2",
          });
        }

        return HttpResponse.json({
          sub_resource_addresses: ["paginated-folder/file2.txt"],
          subfolder_addresses: [],
          next_continuation_handle: null,
        });
      }),

      // File deletions
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/paginated-folder%2Ffile1.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/paginated-folder%2Ffile2.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),

      // Folder deletion
      http.delete("https://test-endpoint.com/v1alpha/filefolder/list/paginated-folder", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    const paginatedFolder = createMockFolder("paginated-folder");

    await act(async () => {
      await result.current.mutateAsync([paginatedFolder]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();

    // Verify update was called for each resource
    expect(testNotification.update).toHaveBeenCalledWith("paginated-folder/file1.txt");
    expect(testNotification.update).toHaveBeenCalledWith("paginated-folder/file2.txt");
    expect(testNotification.update).toHaveBeenCalledWith("paginated-folder");

    // Verify complete was called with all the completed resources
    expect(testNotification.complete).toHaveBeenCalledWith([
      "paginated-folder/file1.txt",
      "paginated-folder/file2.txt",
      "paginated-folder",
    ]);

    // Verify other notification methods weren't called
    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  it("should limit concurrent delete requests when deleting a folder", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    const fileCount = 200;
    const fileAddresses = Array.from({ length: fileCount }, (_, i) => `large-folder/file${i}.txt`);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/filefolder/list/large-folder", () => {
        return HttpResponse.json({
          sub_resource_addresses: fileAddresses,
          subfolder_addresses: [],
          next_continuation_handle: null,
        });
      }),

      http.delete("https://test-endpoint.com/v1alpha/filefolder/list/large-folder", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    let concurrentRequests = 0;
    let maxConcurrentRequests = 0;

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/:address", async () => {
        concurrentRequests++;
        maxConcurrentRequests = Math.max(maxConcurrentRequests, concurrentRequests);
        await new Promise((resolve) => setTimeout(resolve, 10));
        concurrentRequests--;
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });
    const folder = createMockFolder("large-folder");

    await act(async () => {
      await result.current.mutateAsync([folder]);
    });

    expect(testNotification.complete).toHaveBeenCalled();
    expect(maxConcurrentRequests).toBeLessThanOrEqual(15);
  });

  it("should remove deleted resources from selectedPaths", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    const testFile1 = createMockFile("test-file1.txt");
    const testFile2 = createMockFile("test-file2.txt");
    const testFile3 = createMockFile("test-file3.txt");
    const setSelectedPaths = appStore.useAppStore.getState().setSelectedPaths;

    act(() => {
      setSelectedPaths("https://test-endpoint.com", [testFile1, testFile2, testFile3]);
    });

    const initialState = appStore.useAppStore.getState();
    expect(initialState.connections["https://test-endpoint.com"]?.selectedPaths).toEqual(
      new Set([testFile1, testFile2, testFile3])
    );

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-file1.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      }),
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-file2.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection("https://test-endpoint.com")), {
      wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync([testFile1, testFile2]);
    });

    // Verify that only the deleted resources were removed from selectedPaths
    const finalState = appStore.useAppStore.getState();
    expect(finalState.connections["https://test-endpoint.com"]?.selectedPaths).toEqual(new Set([testFile3]));
  });

  it("should delete through the given connection rather than the active one", async () => {
    const otherEndpoint = "https://other-endpoint.com";
    const testFile = createMockFile("test-file.txt");
    const otherFile = createMockFile("other-file.txt");

    act(() => {
      appStore.useAppStore
        .getState()
        .addConnection(otherEndpoint, { auth: mockAuth, discovery: "https://other-discovery" });
      appStore.useAppStore.getState().setSelectedPaths(otherEndpoint, [testFile, otherFile]);
      appStore.useAppStore.getState().setSelectedPaths("https://test-endpoint.com", [testFile]);
    });

    server.use(
      http.delete("https://test-endpoint.com/v1alpha/fileobject/by-address/test-file.txt", () => {
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => useDeletePathMutation(appStore.useConnection(otherEndpoint)), { wrapper });

    await act(async () => {
      await result.current.mutateAsync([testFile]);
    });

    expect(mockCreateServiceApiDiscovery).toHaveBeenCalledWith("https://other-discovery");
    expect(mockCreateServiceApiDiscovery).not.toHaveBeenCalledWith(mockActiveConnection.info.discovery);

    const state = appStore.useAppStore.getState();
    expect(state.connections[otherEndpoint]?.selectedPaths).toEqual(new Set([otherFile]));
    expect(state.connections["https://test-endpoint.com"]?.selectedPaths).toEqual(new Set([testFile]));
  });

  it("should fail without a connection", async () => {
    const { result } = renderHook(() => useDeletePathMutation(undefined), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync([createMockFile("test-file.txt")])).rejects.toThrow(
        "No connection available"
      );
    });
  });
});
