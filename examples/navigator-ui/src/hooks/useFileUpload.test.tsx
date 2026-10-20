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
import { Progress } from "ky";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { FileWithPath } from "@mantine/dropzone";
import * as fileObjectServiceModule from "@/services/storage/fileObjectService";
import * as appStore from "@/state/appStore";
import { Connection } from "@/state/appStore";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { useFileUpload } from "./useFileUpload";

const createMockNotification = () => {
  const abortController = new AbortController();
  return {
    id: "test-notification-id",
    progress: vi.fn(),
    update: vi.fn(),
    complete: vi.fn(),
    completePartially: vi.fn(),
    fail: vi.fn(),
    signal: abortController.signal,
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

const mockActiveConnection: Connection = {
  endpoint: "https://test-endpoint.com",
  currentFolder: "/test-folder/",
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

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
      createServiceApi: async (_discovery: string, name: string, options?: import("@/services/api").ApiOptions) => {
        const api = await getApi(name);
        return api ? create(api.url, options) : null;
      },
    }),
  };
});

function createMockFileHandle(name: string, content: string, type = "text/plain"): FileSystemFileHandle {
  const file = new File([content], name, { type });
  return {
    name,
    kind: "file",
    getFile: vi.fn().mockResolvedValue(file),
  } as unknown as FileSystemFileHandle;
}

function createMockFileWithPath(name: string, content: string, path?: string): FileWithPath {
  const file = new File([content], name, { type: "text/plain" });
  // The real Dropzone component augments a File instance with an extra `path` property.
  const fileWithPath = file as unknown as FileWithPath & { path?: string };
  fileWithPath.path = path ?? `./${name}`;
  return fileWithPath as FileWithPath;
}

function createMockDirectoryHandle(
  name: string,
  entries: (FileSystemFileHandle | FileSystemDirectoryHandle)[]
): FileSystemDirectoryHandle {
  return {
    name,
    kind: "directory",
    values: vi.fn().mockImplementation(function* () {
      for (const entry of entries) {
        yield entry;
      }
    }),
    getDirectoryHandle: vi.fn().mockImplementation((name: string) => {
      const entry = entries.find((e) => e.name === name);
      if (entry?.kind === "directory") {
        return entry;
      }
      throw new Error("Directory not found");
    }),
  } as unknown as FileSystemDirectoryHandle;
}

const wrapper = ({ children }: { children: ReactNode }) => {
  const queryClient = createTestQueryClient();
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
};

describe("useFileUpload", () => {
  beforeEach(() => {
    vi.spyOn(appStore, "useActiveConnection").mockReturnValue(mockActiveConnection);
  });

  function setupMockUploadCompleteHandler(pathSpec: string) {
    server.use(
      http.put(pathSpec, () => {
        return HttpResponse.json(
          {
            redirect: {
              redirect_target_url: "https://upload-target.com/upload",
              additional_headers: [{ name: "x-ms-blob-type", value: "BlockBlob" }],
              method: "PUT",
              completion_header_names: [],
            },
          },
          { status: 300 }
        );
      }),
      http.put("https://upload-target.com/upload", async ({ request }) => {
        if (request.headers.get("x-ms-blob-type") !== "BlockBlob") {
          return HttpResponse.json({ error: "Invalid x-ms-blob-type" }, { status: 400 });
        }
        const reader = request.body?.getReader();
        if (!reader) {
          return HttpResponse.json({ error: "No body" }, { status: 400 });
        }

        while (true) {
          const { done } = await reader.read();
          if (done) {
            break;
          }
        }

        return HttpResponse.json({}, { status: 201 });
      }),
      http.post(
        "https://test-endpoint.com/v1alpha/fileobject/by-address/*/redirect/complete",
        () => new HttpResponse(null, { status: 200 })
      )
    );
  }

  it("should upload files successfully", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    setupMockUploadCompleteHandler("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ffile*");

    const { result } = renderHook(() => useFileUpload(), { wrapper });

    const files = [
      createMockFileHandle("file1.txt", "____16 bytes____"),
      createMockFileHandle("file2.txt", "____16 bytes____"),
    ];

    await act(async () => {
      await result.current.uploadFiles(files);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(testNotification.complete).toHaveBeenCalledWith(["/test-folder/file1.txt", "/test-folder/file2.txt"]);
  });

  it("should encode file names with special characters in the upload request", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    // File name "my file.txt" is encoded as "my%20file.txt" in the resource address
    setupMockUploadCompleteHandler(
      "https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Fmy%2520file.txt"
    );

    const { result } = renderHook(() => useFileUpload(), { wrapper });

    await act(async () => {
      await result.current.uploadFiles([createMockFileHandle("my file.txt", "____16 bytes____")]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(testNotification.complete).toHaveBeenCalledWith(["/test-folder/my file.txt"]);
  });

  it("should display progress when uploading files", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.put("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ffile1.txt", () => {
        return HttpResponse.json(
          {
            redirect: {
              redirect_target_url: "https://upload-target.com/upload",
              additional_headers: [{ name: "x-ms-blob-type", value: "BlockBlob" }],
              method: "PUT",
              completion_header_names: [],
            },
          },
          { status: 300 }
        );
      }),
      http.put("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ffile2.txt", () => {
        return new HttpResponse(null, { status: 400 });
      }),
      http.put("https://upload-target.com/upload", ({ request }) => {
        if (request.headers.get("x-ms-blob-type") !== "BlockBlob") {
          return HttpResponse.json({ error: "Invalid x-ms-blob-type" }, { status: 400 });
        }
        const reader = request.body?.getReader();
        if (!reader) {
          return HttpResponse.json({ error: "No body" }, { status: 400 });
        }

        return HttpResponse.json({}, { status: 201 });
      }),
      http.post(
        "https://test-endpoint.com/v1alpha/fileobject/by-address/*/redirect/complete",
        () => new HttpResponse(null, { status: 200 })
      )
    );

    const { result } = renderHook(() => useFileUpload(), { wrapper });

    await act(async () => {
      await result.current.uploadFiles([
        createMockFileHandle("file1.txt", "____16 bytes____"),
        createMockFileHandle("file2.txt", "____16 bytes____"),
      ]);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.progress).toHaveBeenCalledWith(
        "13 Bytes / 32 Bytes",
        expect.any(String),
        expect.any(Number)
      );
    });
  });

  it("should aggregate concurrent upload progress without decreasing", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-17T12:00:00Z"));

    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);
    const serviceSpy = vi.spyOn(fileObjectServiceModule, "fileObjectService").mockReturnValue({
      upload: vi.fn(
        async (resourceAddress: string, _file: File, onProgress: (progress: Progress) => void): Promise<void> => {
          if (resourceAddress.endsWith("file1.txt")) {
            onProgress({ percent: 0.75, transferredBytes: 12, totalBytes: 16 });
            await new Promise((resolve) => setTimeout(resolve, 600));
            return;
          }

          await new Promise((resolve) => setTimeout(resolve, 300));
          onProgress({ percent: 0.25, transferredBytes: 4, totalBytes: 16 });
          await new Promise((resolve) => setTimeout(resolve, 300));
        }
      ),
    } as unknown as ReturnType<typeof fileObjectServiceModule.fileObjectService>);

    try {
      const { result } = renderHook(() => useFileUpload(), { wrapper });
      let uploadPromise: Promise<void>;

      await act(async () => {
        uploadPromise = result.current.uploadFiles([
          createMockFileHandle("file1.txt", "____16 bytes____"),
          createMockFileHandle("file2.txt", "____16 bytes____"),
        ]);
        await vi.advanceTimersByTimeAsync(0);
      });

      await act(async () => {
        await vi.advanceTimersByTimeAsync(300);
      });

      expect(testNotification.progress).toHaveBeenNthCalledWith(1, "12 Bytes / 32 Bytes", expect.any(String), 38);
      expect(testNotification.progress).toHaveBeenNthCalledWith(2, "16 Bytes / 32 Bytes", expect.any(String), 50);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(300);
        await uploadPromise;
      });
    } finally {
      await vi.runAllTimersAsync();
      vi.useRealTimers();
      serviceSpy.mockRestore();
    }
  });

  it("should publish the latest upload progress before completion", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);
    const serviceSpy = vi.spyOn(fileObjectServiceModule, "fileObjectService").mockReturnValue({
      upload: vi.fn((_resourceAddress: string, _file: File, onProgress: (progress: Progress) => void) => {
        onProgress({ percent: 0.25, transferredBytes: 4, totalBytes: 16 });
        onProgress({ percent: 0.5, transferredBytes: 8, totalBytes: 16 });
        onProgress({ percent: 1, transferredBytes: 16, totalBytes: 16 });
        return Promise.resolve();
      }),
    } as unknown as ReturnType<typeof fileObjectServiceModule.fileObjectService>);

    try {
      const { result } = renderHook(() => useFileUpload(), { wrapper });

      await act(async () => {
        await result.current.uploadFiles([createMockFileHandle("file1.txt", "____16 bytes____")]);
      });

      expect(testNotification.progress).toHaveBeenCalledWith("16 Bytes / 16 Bytes", expect.any(String), 100);
      expect(testNotification.progress.mock.invocationCallOrder.at(-1)).toBeLessThan(
        testNotification.complete.mock.invocationCallOrder[0]
      );
    } finally {
      serviceSpy.mockRestore();
    }
  });

  it("should not report a non-finite percentage for empty files", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);
    const serviceSpy = vi.spyOn(fileObjectServiceModule, "fileObjectService").mockReturnValue({
      upload: vi.fn((_resourceAddress: string, _file: File, onProgress: (progress: Progress) => void) => {
        onProgress({ percent: 1, transferredBytes: 0, totalBytes: 0 });
        return Promise.resolve();
      }),
    } as unknown as ReturnType<typeof fileObjectServiceModule.fileObjectService>);

    try {
      const { result } = renderHook(() => useFileUpload(), { wrapper });

      await act(async () => {
        await result.current.uploadFiles([createMockFileHandle("empty.txt", "")]);
      });

      expect(testNotification.progress.mock.calls.every(([, , percent]) => Number.isFinite(percent))).toBe(true);
      expect(testNotification.complete).toHaveBeenCalledOnce();
    } finally {
      serviceSpy.mockRestore();
    }
  });

  it("should upload a directory with nested files and subdirectories", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    setupMockUploadCompleteHandler("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2F*");

    const { result } = renderHook(() => useFileUpload(), { wrapper });

    const nestedDirHandle = createMockDirectoryHandle("test-dir2", [
      createMockFileHandle("test1.txt", "____16 bytes____"),
      createMockFileHandle("test2.txt", "____16 bytes____"),
    ]);

    const dirHandle = createMockDirectoryHandle("test-dir", [
      createMockFileHandle("test0.txt", "____16 bytes____"),
      nestedDirHandle,
    ]);

    await act(async () => {
      await result.current.uploadFolder(dirHandle);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.complete).toHaveBeenCalledWith([
        "/test-folder/test-dir/test0.txt",
        "/test-folder/test-dir/test-dir2/test1.txt",
        "/test-folder/test-dir/test-dir2/test2.txt",
        "/test-folder/test-dir/test-dir2",
        "/test-folder/test-dir",
      ]);
    });
  });

  it("should encode folder names with special characters in directory upload requests", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    // Folder names are encoded in the resource address before the full address is encoded as a route parameter.
    setupMockUploadCompleteHandler(
      "https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Fmy%2520folder%2Fnested%2520folder%2Ffile.txt"
    );

    const { result } = renderHook(() => useFileUpload(), { wrapper });
    const nestedDirHandle = createMockDirectoryHandle("nested folder", [
      createMockFileHandle("file.txt", "____16 bytes____"),
    ]);
    const dirHandle = createMockDirectoryHandle("my folder", [nestedDirHandle]);

    await act(async () => {
      await result.current.uploadFolder(dirHandle);
    });

    expect(testNotification.complete).toHaveBeenCalledOnce();
  });

  it("should upload files from dropzone successfully", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    setupMockUploadCompleteHandler("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2F*");

    const { result } = renderHook(() => useFileUpload(), { wrapper });

    const files = [
      createMockFileWithPath("file1.txt", "____16 bytes____"),
      createMockFileWithPath("file2.txt", "____16 bytes____"),
      createMockFileWithPath("file3.txt", "____16 bytes____", "./test/file3.txt"),
    ];

    await act(async () => {
      await result.current.uploadFromDropzone(files);
    });

    expect(mockFileActionNotification).toHaveBeenCalled();
    expect(testNotification.complete).toHaveBeenCalledWith([
      "/test-folder/file1.txt",
      "/test-folder/file2.txt",
      "/test-folder/test/file3.txt",
      "/test-folder/test",
    ]);
  });

  it("should encode folder names with special characters in dropzone upload requests", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    setupMockUploadCompleteHandler(
      "https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Fmy%2520folder%2Ffile.txt"
    );

    const { result } = renderHook(() => useFileUpload(), { wrapper });
    const files = [createMockFileWithPath("file.txt", "____16 bytes____", "./my folder/file.txt")];

    await act(async () => {
      await result.current.uploadFromDropzone(files);
    });

    expect(testNotification.complete).toHaveBeenCalledOnce();
  });

  it("should handle partial upload failures", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.put("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ftest*", ({ request }) => {
        if (request.url.includes("test2.txt")) {
          return HttpResponse.json({ error: "Upload failed" }, { status: 500 });
        }
        return HttpResponse.json(
          {
            redirect: {
              redirect_target_url: "https://upload-target.com/upload",
              additional_headers: [{ name: "x-ms-blob-type", value: "BlockBlob" }],
              method: "PUT",
              completion_header_names: [],
            },
          },
          { status: 300 }
        );
      }),
      http.put("https://upload-target.com/upload", ({ request }) => {
        if (request.headers.get("x-ms-blob-type") !== "BlockBlob") {
          return HttpResponse.json({ error: "Invalid x-ms-blob-type" }, { status: 400 });
        }
        const reader = request.body?.getReader();
        if (!reader) {
          return HttpResponse.json({ error: "No body" }, { status: 400 });
        }

        return HttpResponse.json({}, { status: 201 });
      }),
      http.post(
        "https://test-endpoint.com/v1alpha/fileobject/by-address/*/redirect/complete",
        () => new HttpResponse(null, { status: 200 })
      )
    );
    const { result } = renderHook(() => useFileUpload(), { wrapper });

    const dirHandle = createMockDirectoryHandle("test-dir", [
      createMockFileHandle("test1.txt", "____16 bytes____"),
      createMockFileHandle("test2.txt", "____16 bytes____"),
    ]);

    // The failing upload retries with backoff before giving up; fake timers let the
    // test observe that outcome without waiting on the real backoff delays.
    vi.useFakeTimers();
    try {
      await act(async () => {
        const uploadPromise = result.current.uploadFolder(dirHandle);
        await vi.runAllTimersAsync();
        await uploadPromise;
      });
    } finally {
      vi.useRealTimers();
    }

    expect(mockFileActionNotification).toHaveBeenCalled();
    await waitFor(() => {
      expect(testNotification.completePartially).toHaveBeenCalledWith(
        ["/test-folder/test-dir/test1.txt", "/test-folder/test-dir"],
        ["/test-folder/test-dir/test2.txt"]
      );
    });
  });

  it("should handle complete upload failure", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.put("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ftest*", ({ request }) => {
        if (request.url.includes("test")) {
          return HttpResponse.json({ error: "Upload failed" }, { status: 500 });
        }
        return HttpResponse.json(
          {
            redirect: {
              redirect_target_url: "https://upload-target.com/upload",
              additional_headers: [{ name: "x-ms-blob-type", value: "BlockBlob" }],
              method: "PUT",
              completion_header_names: [],
            },
          },
          { status: 300 }
        );
      }),
      http.put("https://upload-target.com/upload", ({ request }) => {
        if (request.headers.get("x-ms-blob-type") !== "BlockBlob") {
          return HttpResponse.json({ error: "Invalid x-ms-blob-type" }, { status: 400 });
        }
        const reader = request.body?.getReader();
        if (!reader) {
          return HttpResponse.json({ error: "No body" }, { status: 400 });
        }

        return HttpResponse.json({}, { status: 201 });
      })
    );
    const { result } = renderHook(() => useFileUpload(), { wrapper });

    const dirHandle = createMockDirectoryHandle("test-dir", [
      createMockFileHandle("test1.txt", "____16 bytes____"),
      createMockFileHandle("test2.txt", "____16 bytes____"),
    ]);

    // Both uploads retry with backoff before giving up; fake timers let the test
    // observe that outcome without waiting on the real backoff delays.
    vi.useFakeTimers();
    try {
      await act(async () => {
        const uploadPromise = result.current.uploadFolder(dirHandle);
        await vi.runAllTimersAsync();
        await uploadPromise;
      });
    } finally {
      vi.useRealTimers();
    }

    expect(mockFileActionNotification).toHaveBeenCalled();
    // Both files retry independently with backoff, so the order in which they
    // finish failing (and land in the reported list) isn't guaranteed.
    await waitFor(() => {
      expect(testNotification.fail).toHaveBeenCalledTimes(1);
    });
    const failedPaths = testNotification.fail.mock.calls[0][0] as string[];
    expect(failedPaths).toHaveLength(2);
    expect(failedPaths).toEqual(
      expect.arrayContaining(["/test-folder/test-dir/test1.txt", "/test-folder/test-dir/test2.txt"])
    );
  });

  it("invalidates the version cache for each successfully uploaded file", async () => {
    const testNotification = createMockNotification();
    mockFileActionNotification.mockReturnValue(testNotification);

    setupMockUploadCompleteHandler("https://test-endpoint.com/v1alpha/fileobject/by-address/%2Ftest-folder%2Ffile*");

    const queryClient = createTestQueryClient();
    const invalidateQueriesSpy = vi.spyOn(queryClient, "invalidateQueries");
    const customWrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useFileUpload(), { wrapper: customWrapper });

    await act(async () => {
      await result.current.uploadFiles([
        createMockFileHandle("file1.txt", "____16 bytes____"),
        createMockFileHandle("file2.txt", "____16 bytes____"),
      ]);
    });

    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: [mockActiveConnection.endpoint, "/test-folder/file1.txt", "version"],
    });
  });
});
