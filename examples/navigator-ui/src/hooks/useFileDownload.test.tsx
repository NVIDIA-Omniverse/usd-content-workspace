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
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { list } from "radash";
import { Connection } from "@/state/appStore";
import { createMockDirectoryHandle } from "@/test-utils/mockFileSystemAccessApi";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { useFileDownload } from "./useFileDownload";

const createMockNotification = () => {
  const controller = new AbortController();
  return {
    id: "test-notification-id",
    progress: vi.fn(),
    update: vi.fn(),
    complete: vi.fn(),
    completePartially: vi.fn(),
    cancel: vi.fn(),
    fail: vi.fn(),
    signal: controller.signal,
  };
};

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

const mockConnection: Connection = {
  endpoint: "https://test-endpoint.com",
  currentFolder: "/test-folder",
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "https://test-discovery.com",
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
  const queryClient = createTestQueryClient();
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
};

describe("useFileDownload", () => {
  it("should resolve the storage services through the given connection", async () => {
    const otherConnection: Connection = {
      ...mockConnection,
      endpoint: "https://other-endpoint.com",
      info: { auth: mockAuth, discovery: "https://other-discovery.com" },
    };
    const { result } = renderHook(() => useFileDownload(otherConnection), { wrapper });

    await act(async () => {
      await result.current.download([], createMockDirectoryHandle("test-dir", []));
    });

    expect(mockCreateServiceApiDiscovery).toHaveBeenCalledWith("https://other-discovery.com");
    expect(mockCreateServiceApiDiscovery).not.toHaveBeenCalledWith(mockConnection.info.discovery);
  });

  it("should throw without a connection", async () => {
    const { result } = renderHook(() => useFileDownload(undefined), { wrapper });

    await expect(result.current.download([], createMockDirectoryHandle("test-dir", []))).rejects.toThrow(
      "No connection available"
    );
  });

  it("should download files", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("x".repeat(1500)));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1500",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: FileSystemFileHandle[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);

    const files = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    await act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.complete).toHaveBeenCalledWith(["file1.txt", "file2.txt"]);
    });
    expect(entries.length).toBe(2);
    expect(entries[0].name).toBe("file1.txt");
    expect(entries[1].name).toBe("file2.txt");
  });

  it("should download a zero-byte file without reporting an error", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        return new HttpResponse(new ReadableStream({ start: (controller) => controller.close() }), {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "0",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: FileSystemFileHandle[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);

    const files = [{ ...createMockFile("file1.txt"), bytes: 0 }];
    await act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.complete).toHaveBeenCalledWith(["file1.txt"]);
    });
    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
    expect(entries.length).toBe(1);
    expect(entries[0].name).toBe("file1.txt");
  });

  it("should download folders with nested content", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/filefolder/listStat/test-folder", () => {
        return HttpResponse.json({
          subfolder_addresses: ["test-folder/subfolder"],
          entries: [
            {
              resource_address: "test-folder/file1.txt",
              resource_identity: "id1",
              metadata: {
                data_object_size: 1500,
              },
            },
            {
              resource_address: "test-folder/file2.txt",
              resource_identity: "id2",
              metadata: {
                data_object_size: 1500,
              },
            },
          ],
          sub_resource_addresses: ["test-folder/file1.txt", "test-folder/file2.txt"],
          next_continuation_handle: null,
        });
      }),

      http.get("https://test-endpoint.com/v1alpha/filefolder/listStat/test-folder%2Fsubfolder", () => {
        return HttpResponse.json({
          subfolder_addresses: [],
          entries: [
            {
              resource_address: "test-folder/subfolder/file3.txt",
              resource_identity: "id3",
              metadata: {
                data_object_size: 1500,
              },
            },
          ],
          sub_resource_addresses: ["test-folder/subfolder/file3.txt"],
          next_continuation_handle: null,
        });
      }),
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("x".repeat(1500)));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1500",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);

    await act(async () => {
      await result.current.download([createMockFolder("test-folder")], mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();

    await waitFor(() => {
      expect(testNotification.complete).toHaveBeenCalledWith([
        "test-folder/subfolder",
        "test-folder",
        "test-folder/file1.txt",
        "test-folder/file2.txt",
        "test-folder/subfolder/file3.txt",
      ]);
    });
    expect(entries.length).toBe(1);
    expect(entries[0].name).toBe("test-folder");
  });

  it("should display progress during download", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode(list(0, 200, () => "x").join("")));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1500",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: FileSystemFileHandle[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);
    const files = [createMockFile("file1.txt")];

    await act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(testNotification.progress).toHaveBeenCalledWith(
      "0 Bytes / 1.46 KiB",
      expect.any(String),
      expect.any(Number)
    );
  });

  it("should handle partial download success", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/file1.txt", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("x".repeat(1500)));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1500",
          },
        });
      }),
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/file2.txt", () => {
        return new HttpResponse(null, { status: 404 });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);
    const files = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    await act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.completePartially).toHaveBeenCalledWith(["file1.txt"], ["file2.txt"]);
    });
  });

  it("should fail download on file size mismatch", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    // mock listStat to return file with size 1500 bytes
    server.use(
      http.get("https://test-endpoint.com/v1alpha/filefolder/listStat/test-mismatch-folder", () => {
        return HttpResponse.json({
          subfolder_addresses: [],
          entries: [
            {
              resource_address: "test-mismatch-folder/file1.txt",
              resource_identity: "id1",
              metadata: {
                data_object_size: 1500,
              },
            },
          ],
          sub_resource_addresses: ["test-mismatch-folder/file1.txt"],
          next_continuation_handle: null,
        });
      }),

      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("x".repeat(1000)));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 200,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1000",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);

    await act(async () => {
      await result.current.download([createMockFolder("test-mismatch-folder")], mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.completePartially).toHaveBeenCalledWith(
        ["test-mismatch-folder"],
        ["test-mismatch-folder/file1.txt"]
      );
    });
  });

  it("should handle download failure", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint.com/v1alpha/fileobject/by-address/*", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("x".repeat(1500)));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          status: 500,
          headers: {
            "Content-Type": "application/octet-stream",
            "Content-Length": "1500",
          },
        });
      })
    );

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);
    const files = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    await act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.fail).toHaveBeenCalledWith(["file1.txt", "file2.txt"]);
    });
  });

  it("should handle abort signal correctly", async () => {
    const controller = new AbortController();
    const abortedNotification = {
      id: "test-notification-id",
      progress: vi.fn(),
      update: vi.fn(),
      complete: vi.fn(),
      completePartially: vi.fn(),
      cancel: vi.fn(),
      fail: vi.fn(),
      signal: controller.signal,
    };
    mockFileActionNotification.mockReturnValue(abortedNotification);

    const { result } = renderHook(() => useFileDownload(mockConnection), { wrapper });

    const entries: FileSystemFileHandle[] = [];
    const mockDirHandle = createMockDirectoryHandle("test-dir", entries);
    const files = [createMockFile("file1.txt"), createMockFile("file2.txt")];

    const downloadPromise = act(async () => {
      await result.current.download(files, mockDirHandle);
    });

    controller.abort();
    await downloadPromise;

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(abortedNotification.cancel).toHaveBeenCalled();
    expect(entries.length).toBe(0); // No files should be created
  });
});
