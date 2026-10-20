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
import { afterEach, beforeEach, vi } from "vitest";
import { CopyResponse, ListStatResult, MoveResponse } from "@/services/storage";
import { FileListItem, FolderListItem } from "@/state/path";
import { ClipboardOperation } from "@/state/slices/clipboardSlice";
import { Connection } from "@/state/slices/connectionSlice";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { usePathClipboard } from "./usePathClipboard";

const endpoint = "https://test-endpoint/";
const destination = "/test-destination/";

const mockAuth = createTestAuth();

const connection: Connection = {
  endpoint,
  currentFolder: "/test-folder",
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

vi.mock("@/state/appStore", async () => {
  const actual = await vi.importActual("@/state/appStore");
  return {
    ...actual,
    useConnection: (endpoint: string) => (connection.endpoint === endpoint ? connection : undefined),
  };
});

const createMockNotification = () => ({
  id: "test-notification-id",
  update: vi.fn(),
  complete: vi.fn(),
  completePartially: vi.fn(),
  fail: vi.fn(),
  signal: { aborted: false },
});

const mocks = vi.hoisted(() => {
  const fileActionNotification = vi.fn(() => {
    return createMockNotification();
  });

  const notificationShow = vi.fn();

  return {
    fileActionNotification,
    useFileActionNotification: vi.fn(() => fileActionNotification),
    notificationShow,
    useNotification: vi.fn(() => ({
      show: notificationShow,
    })),
  };
});

vi.mock("./useFileActionNotification", () => ({
  useFileActionNotification: mocks.useFileActionNotification,
}));

vi.mock("./useNotification", () => ({
  useNotification: mocks.useNotification,
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
      url: "https://test-endpoint/v1alpha",
      baseUrl: "https://test-endpoint/",
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

const wrapper = ({ children }: { children: ReactNode }) => {
  return <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>;
};

describe("usePathClipboard", () => {
  const server = setupServer(
    http.post<{ resource_identity: string }, { destination_resource_address: string }>(
      "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
      async ({ request }) => {
        const { destination_resource_address } = await request.json();
        return HttpResponse.json(
          {
            resourceIdentity: `${destination_resource_address}-identity`,
          } as CopyResponse,
          { status: 200 }
        );
      }
    ),
    http.post<{ resource_address: string }, { destination_resource_address: string }>(
      "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
      async ({ request }) => {
        const { destination_resource_address } = await request.json();
        return HttpResponse.json(
          {
            resourceIdentity: `${destination_resource_address}-identity`,
          } as MoveResponse,
          { status: 201 }
        );
      }
    ),
    http.delete("https://test-endpoint/v1alpha/fileobject/by-address/:resource_identity", () => {
      return HttpResponse.json({}, { status: 204 });
    }),
    http.put("https://test-endpoint/v1alpha/filefolder/:folder_address", () => {
      return new HttpResponse(null, { status: 204 });
    })
  );

  beforeEach(() => {
    server.listen();
    vi.clearAllMocks();
  });

  afterEach(() => {
    server.resetHandlers();
    server.close();
  });

  test("copies files", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    expect(result.current.items).toEqual([]);
    expect(result.current.empty()).toBe(true);

    const copiedFiles = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.copy(copiedFiles);
    });
    expect(result.current.items).toBe(copiedFiles);
    expect(result.current.empty()).toBe(false);

    const newFiles = [createMockFile("file3.txt")];
    act(() => {
      result.current.copy(newFiles);
    });
    expect(result.current.items).toBe(newFiles);
    expect(result.current.empty()).toBe(false);

    act(() => {
      result.current.clear();
    });
    expect(result.current.items).toEqual([]);
    expect(result.current.empty()).toBe(true);
  });

  test("pastes files", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const copyRequestSpy = vi.fn();
    const moveRequestSpy = vi.fn();

    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        async ({ request }) => {
          copyRequestSpy();
          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as CopyResponse,
            { status: 200 }
          );
        }
      ),
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request }) => {
          moveRequestSpy();
          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(mocks.fileActionNotification).toBeCalledWith({ action: "Copy", data: source });
    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "/test-destination/file1.txt",
          resourceIdentity: "/test-destination/file1.txt-identity",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/file2.txt",
          resourceIdentity: "/test-destination/file2.txt-identity",
        } as Partial<FileListItem>),
      ]),
      []
    );

    expect(copyRequestSpy).toHaveBeenCalledTimes(2);
    expect(moveRequestSpy).not.toHaveBeenCalled();
  });

  test("cuts files", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const moveRequestSpy = vi.fn();
    const deleteRequestSpy = vi.fn();

    server.use(
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request }) => {
          moveRequestSpy();
          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      ),
      http.delete("https://test-endpoint/v1alpha/fileobject/by-address/:resource_identity", () => {
        deleteRequestSpy();
        return HttpResponse.json({}, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.cut(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(mocks.fileActionNotification).toBeCalledWith({ action: "Cut", data: source });
    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "/test-destination/file1.txt",
          resourceIdentity: "/test-destination/file1.txt-identity",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/file2.txt",
          resourceIdentity: "/test-destination/file2.txt-identity",
        } as Partial<FileListItem>),
      ]),
      []
    );

    // Verify move was called and delete was NOT called
    expect(moveRequestSpy).toHaveBeenCalledTimes(2);
    expect(deleteRequestSpy).not.toHaveBeenCalled();
  });

  test("renames a file", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt")];
    const newName = "file2.txt";
    act(() => {
      result.current.cut(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, newName });
    });

    expect(mocks.fileActionNotification).toBeCalledWith({ action: "Rename", data: source });
    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "/test-destination/file2.txt",
          resourceIdentity: "/test-destination/file2.txt-identity",
        } as Partial<FileListItem>),
      ]),
      []
    );
  });

  test("invalidates the version cache for the destination when copying files", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const queryClient = createTestQueryClient();
    const invalidateQueriesSpy = vi.spyOn(queryClient, "invalidateQueries");
    const customWrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper: customWrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: [endpoint, "/test-destination/file1.txt", "version"],
    });
    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: [endpoint, "/test-destination/file2.txt", "version"],
    });
  });

  test("invalidates the version cache for the destination when moving files", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const queryClient = createTestQueryClient();
    const invalidateQueriesSpy = vi.spyOn(queryClient, "invalidateQueries");
    const customWrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper: customWrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.cut(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: [endpoint, "/test-destination/file1.txt", "version"],
    });
    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: [endpoint, "/test-destination/file2.txt", "version"],
    });
  });

  test("encodes new name with special characters when renaming on paste", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const moveRequestBodies: { destination_resource_address: string }[] = [];
    server.use(
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request }) => {
          const body = await request.json();
          moveRequestBodies.push(body as { destination_resource_address: string });
          return HttpResponse.json(
            {
              resourceIdentity: `${(body as { destination_resource_address: string }).destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt")];
    act(() => {
      result.current.cut(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, newName: "renamed file.txt" });
    });

    expect(moveRequestBodies).toHaveLength(1);
    expect(moveRequestBodies[0].destination_resource_address).toBe("/test-destination/renamed%20file.txt");
    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "/test-destination/renamed%20file.txt",
          resourceIdentity: "/test-destination/renamed%20file.txt-identity",
        } as Partial<FileListItem>),
      ]),
      []
    );
  });

  test("encodes file name with unicode and spaces when copying on paste", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const copyRequestBodies: { destination_resource_address: string }[] = [];
    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        async ({ request }) => {
          const body = await request.json();
          copyRequestBodies.push(body as { destination_resource_address: string });
          return HttpResponse.json(
            {
              resourceIdentity: `${(body as { destination_resource_address: string }).destination_resource_address}-identity`,
            } as CopyResponse,
            { status: 200 }
          );
        }
      )
    );

    const fileName = "9728-pekannöt [w1200 x h400]mm bump_noise.png";
    const expectedAddress = `/test-destination/${encodeURIComponent(fileName)}`;

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.copy([createMockFile(fileName)]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(copyRequestBodies).toHaveLength(1);
    expect(copyRequestBodies[0].destination_resource_address).toBe(expectedAddress);
    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: expectedAddress,
          resourceIdentity: `${expectedAddress}-identity`,
        } as Partial<FileListItem>),
      ]),
      []
    );
  });

  test("throws an error if more than one file is renamed", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.cut(source);
    });

    await expect(
      result.current.paste.mutateAsync({
        destination,
        newName: "file3.txt",
      })
    ).rejects.toThrow("Cannot rename more than one path at once.");
  });

  test("pastes files partially", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        async ({ request, params }) => {
          if (params.resource_identity === "file1.txt-identity") {
            const { destination_resource_address } = await request.json();
            return HttpResponse.json(
              {
                resourceIdentity: `${destination_resource_address}-identity`,
              } as CopyResponse,
              { status: 200 }
            );
          }
          return new HttpResponse(null, { status: 500 });
        }
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt"), createMockFile("file3.txt")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.completePartially).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/file1.txt",
          resourceIdentity: "/test-destination/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      [
        expect.objectContaining({
          resourceAddress: "file2.txt",
          resourceIdentity: "file2.txt-identity",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "file3.txt",
          resourceIdentity: "file3.txt-identity",
        } as Partial<FileListItem>),
      ],
      []
    );
  });

  test("pastes files with some skipped when overwrite is disabled for a copy", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string; fail_if_exists: boolean }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        async ({ request, params }) => {
          const { destination_resource_address, fail_if_exists } = await request.json();
          expect(fail_if_exists).toBe(true);
          if (params.resource_identity === "file1.txt-identity") {
            return HttpResponse.json(
              {
                resourceIdentity: `${destination_resource_address}-identity`,
              } as CopyResponse,
              { status: 200 }
            );
          }
          return new HttpResponse(null, { status: 409 });
        }
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, overwrite: false });
    });

    expect(testNotification.complete).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/file1.txt",
          resourceIdentity: "/test-destination/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      ["file2.txt"]
    );
  });

  test("treats a 409 as a failure, not a skip, when overwrite was not disabled", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        () => new HttpResponse(null, { status: 409 })
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.fail).toBeCalledWith([
      expect.objectContaining({
        resourceAddress: "file1.txt",
        resourceIdentity: "file1.txt-identity",
      } as Partial<FileListItem>),
    ]);
  });

  test("pastes files with some skipped when overwrite is disabled for a move", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.post<{ resource_address: string }, { destination_resource_address: string; fail_if_exists: boolean }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request, params }) => {
          const { destination_resource_address, fail_if_exists } = await request.json();
          expect(fail_if_exists).toBe(true);
          if (params.resource_address === "file1.txt") {
            return HttpResponse.json(
              {
                resourceIdentity: `${destination_resource_address}-identity`,
              } as MoveResponse,
              { status: 201 }
            );
          }
          return new HttpResponse(null, { status: 409 });
        }
      )
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt"), createMockFile("file2.txt")];
    act(() => {
      result.current.cut(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, overwrite: false });
    });

    expect(testNotification.complete).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/file1.txt",
          resourceIdentity: "/test-destination/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      ["file2.txt"]
    );
    expect(result.current.items).toEqual([source[1]]);
    expect(result.current.operation).toBe(ClipboardOperation.Cut);
  });

  test("pastes folders", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: ["folder1/folder2"],
              entries: [
                {
                  resourceAddress: "folder1/file1.txt",
                  resourceIdentity: "folder1/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: "page2",
            } as ListStatResult,
            { status: 200 }
          );
        } else if (
          request.url ===
          "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000&continuation_handle=page2"
        ) {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder1/file2.txt",
                  resourceIdentity: "folder1/file2.txt-identity",
                  metadata: {
                    dataObjectSize: 111,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        } else if (
          request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1%2Ffolder2?max_page_size=1000"
        ) {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1/folder2",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder1/folder2/file3.txt",
                  resourceIdentity: "folder1/folder2/file3.txt-identity",
                  metadata: {
                    dataObjectSize: 100,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }
        return HttpResponse.json(
          {
            isRoot: false,
            entries: [],
            subfolderAddresses: [],
            nextContinuationHandle: null,
            folderAddress: "",
          } as ListStatResult,
          { status: 200 }
        );
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("folder1")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.complete).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/folder2",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/file1.txt",
          resourceIdentity: "/test-destination/folder1/file1.txt-identity",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/file2.txt",
          resourceIdentity: "/test-destination/folder1/file2.txt-identity",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/folder2/file3.txt",
          resourceIdentity: "/test-destination/folder1/folder2/file3.txt-identity",
        } as Partial<FileListItem>),
      ]),
      []
    );
  });

  test("cuts folders and deletes the emptied source folders last", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const events: string[] = [];

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: ["folder1/folder2"],
              entries: [
                {
                  resourceAddress: "folder1/file1.txt",
                  resourceIdentity: "folder1/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1%2Ffolder2?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1/folder2",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder1/folder2/file2.txt",
                  resourceIdentity: "folder1/folder2/file2.txt-identity",
                  metadata: {
                    dataObjectSize: 100,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        return new HttpResponse(null, { status: 404 });
      }),
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request, params }) => {
          events.push(`move:${params.resource_address}`);
          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      ),
      http.delete("https://test-endpoint/v1alpha/filefolder/list/*", ({ request }) => {
        events.push(`delete:${decodeURIComponent(new URL(request.url).pathname.split("/").pop() ?? "")}`);
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.cut([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(events).toEqual([
      "move:folder1/file1.txt",
      "move:folder1/folder2/file2.txt",
      "delete:folder1/folder2",
      "delete:folder1",
    ]);
    expect(testNotification.complete).toBeCalled();
  });

  test("does not delete cut folders when a descendant move fails", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const events: string[] = [];

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: ["folder1/folder2"],
              entries: [
                {
                  resourceAddress: "folder1/file1.txt",
                  resourceIdentity: "folder1/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1%2Ffolder2?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1/folder2",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder1/folder2/file2.txt",
                  resourceIdentity: "folder1/folder2/file2.txt-identity",
                  metadata: {
                    dataObjectSize: 100,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        return new HttpResponse(null, { status: 404 });
      }),
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request, params }) => {
          events.push(`move:${params.resource_address}`);
          if (params.resource_address === "folder1/folder2/file2.txt") {
            return new HttpResponse(null, { status: 500 });
          }

          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      ),
      http.delete("https://test-endpoint/v1alpha/filefolder/list/*", ({ request }) => {
        events.push(`delete:${decodeURIComponent(new URL(request.url).pathname.split("/").pop() ?? "")}`);
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.cut([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(events).toEqual(["move:folder1/file1.txt", "move:folder1/folder2/file2.txt"]);
    expect(testNotification.completePartially).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/file1.txt",
          resourceIdentity: "/test-destination/folder1/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "folder1/folder2/file2.txt",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "folder1/folder2",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "folder1",
        } as Partial<FolderListItem>),
      ]),
      []
    );
  });

  test("does not report folders as failed or delete them when a descendant move is skipped", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const events: string[] = [];

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: ["folder1/folder2"],
              entries: [
                {
                  resourceAddress: "folder1/file1.txt",
                  resourceIdentity: "folder1/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1%2Ffolder2?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1/folder2",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder1/folder2/file.txt",
                  resourceIdentity: "folder1/folder2/file.txt-identity",
                  metadata: {
                    dataObjectSize: 100,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        return new HttpResponse(null, { status: 404 });
      }),
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request, params }) => {
          events.push(`move:${params.resource_address}`);
          if (params.resource_address === "folder1/folder2/file.txt") {
            return new HttpResponse(null, { status: 409 });
          }

          const { destination_resource_address } = await request.json();
          return HttpResponse.json(
            {
              resourceIdentity: `${destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      ),
      http.delete("https://test-endpoint/v1alpha/filefolder/list/*", ({ request }) => {
        events.push(`delete:${decodeURIComponent(new URL(request.url).pathname.split("/").pop() ?? "")}`);
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.cut([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, overwrite: false });
    });

    expect(events).toEqual(["move:folder1/file1.txt", "move:folder1/folder2/file.txt"]);
    expect(testNotification.complete).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/file1.txt",
          resourceIdentity: "/test-destination/folder1/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      ["folder1/folder2/file.txt"]
    );
    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  test("creates destination folder when copying and cutting an empty folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const events: string[] = [];

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: [],
              entries: [],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }

        return new HttpResponse(null, { status: 404 });
      }),
      http.put("https://test-endpoint/v1alpha/filefolder/:folder_address", ({ params }) => {
        events.push(`createFolder:${decodeURIComponent(params.folder_address as string)}`);
        return new HttpResponse(null, { status: 204 });
      }),
      http.delete("https://test-endpoint/v1alpha/filefolder/list/*", ({ request }) => {
        events.push(`delete:${decodeURIComponent(new URL(request.url).pathname.split("/").pop() ?? "")}`);
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.copy([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(events).toEqual(["createFolder:/test-destination/folder1"]);

    act(() => {
      result.current.cut([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(events).toEqual([
      "createFolder:/test-destination/folder1",
      "createFolder:/test-destination/folder1",
      "delete:folder1",
    ]);
    expect(testNotification.complete).toHaveBeenCalledTimes(2);
  });

  test("skips an existing empty destination folder when overwrite is disabled", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return HttpResponse.json(
          {
            isRoot: false,
            folderAddress: "folder1",
            subfolderAddresses: [],
            entries: [],
            nextContinuationHandle: null,
          } as ListStatResult,
          { status: 200 }
        );
      }),
      http.put("https://test-endpoint/v1alpha/filefolder/:folder_address", () => {
        return new HttpResponse(null, { status: 409 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.copy([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, overwrite: false });
    });

    expect(testNotification.complete).toHaveBeenCalledWith([], ["folder1"]);
    expect(testNotification.fail).not.toHaveBeenCalled();
    expect(testNotification.completePartially).not.toHaveBeenCalled();
  });

  test("renames folders and deletes the emptied source folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const deleteRequestSpy = vi.fn();
    const moveDestinations: string[] = [];

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return HttpResponse.json(
          {
            isRoot: false,
            folderAddress: "folder1",
            subfolderAddresses: [],
            entries: [
              {
                resourceAddress: "folder1/file1.txt",
                resourceIdentity: "folder1/file1.txt-identity",
                metadata: {
                  dataObjectSize: 256,
                  lastModifiedTimestamp: new Date().toISOString(),
                },
              },
            ],
            nextContinuationHandle: null,
          } as ListStatResult,
          { status: 200 }
        );
      }),
      http.post<{ resource_address: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-address/:resource_address/move",
        async ({ request }) => {
          const body = (await request.json()) as { destination_resource_address: string };
          moveDestinations.push(body.destination_resource_address);
          return HttpResponse.json(
            {
              resourceIdentity: `${body.destination_resource_address}-identity`,
            } as MoveResponse,
            { status: 201 }
          );
        }
      ),
      http.delete("https://test-endpoint/v1alpha/filefolder/list/*", () => {
        deleteRequestSpy();
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.cut([createMockFolder("folder1")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination, newName: "renamed-folder" });
    });

    expect(moveDestinations).toEqual(["/test-destination/renamed-folder/file1.txt"]);
    expect(deleteRequestSpy).toHaveBeenCalledTimes(1);
    expect(testNotification.complete).toBeCalled();
  });

  test("pastes folders partially if an error occurs in a child folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder1?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: ["folder1/folder2"],
              entries: [
                {
                  resourceAddress: "folder1/file1.txt",
                  resourceIdentity: "folder1/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: null,
            } as ListStatResult,
            { status: 200 }
          );
        }
        return new HttpResponse(null, { status: 500 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("folder1")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.completePartially).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/folder1/file1.txt",
          resourceIdentity: "/test-destination/folder1/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      [
        expect.objectContaining({
          resourceAddress: "folder1/folder2",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "folder1",
        } as Partial<FolderListItem>),
      ],
      []
    );
  });

  test("pastes folders partially if an error occurs during pagination", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", ({ request }) => {
        if (request.url === "https://test-endpoint/v1alpha/filefolder/liststat/folder?max_page_size=1000") {
          return HttpResponse.json(
            {
              isRoot: false,
              folderAddress: "folder1",
              subfolderAddresses: [],
              entries: [
                {
                  resourceAddress: "folder/file1.txt",
                  resourceIdentity: "folder/file1.txt-identity",
                  metadata: {
                    dataObjectSize: 256,
                    lastModifiedTimestamp: new Date().toISOString(),
                  },
                },
              ],
              nextContinuationHandle: "page2",
            } as ListStatResult,
            { status: 200 }
          );
        }
        return new HttpResponse(null, { status: 500 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("folder")];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.completePartially).toBeCalledWith(
      [
        expect.objectContaining({
          resourceAddress: "/test-destination/folder/file1.txt",
          resourceIdentity: "/test-destination/folder/file1.txt-identity",
        } as Partial<FileListItem>),
      ],
      [
        expect.objectContaining({
          resourceAddress: "folder",
        } as Partial<FolderListItem>),
      ],
      []
    );
  });

  test("displays an error", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.post<{ resource_identity: string }, { destination_resource_address: string }>(
        "https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy",
        () => {
          return new HttpResponse(null, { status: 500 });
        }
      ),
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return new HttpResponse(null, { status: 500 });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [
      createMockFolder("folder1"),
      createMockFolder("folder2"),
      createMockFile("file1.txt"),
      createMockFile("file2.txt"),
      createMockFile("file3.txt"),
    ];
    act(() => {
      result.current.copy(source);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.fail).toBeCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          resourceAddress: "folder1",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "folder2",
        } as Partial<FolderListItem>),
        expect.objectContaining({
          resourceAddress: "file1.txt",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "file2.txt",
        } as Partial<FileListItem>),
        expect.objectContaining({
          resourceAddress: "file3.txt",
        } as Partial<FileListItem>),
      ])
    );
  });

  test("throws an error when pasting a folder into its own child path", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/parent/folder")];
    act(() => {
      result.current.copy(source);
    });

    await expect(
      result.current.paste.mutateAsync({
        destination: "/parent/folder/child",
      })
    ).rejects.toThrow("Cannot paste folder into its own child path: folder");

    expect(mocks.notificationShow).toHaveBeenCalledWith({
      type: "error",
      title: "Invalid Operation",
      message: 'Cannot paste folder "folder" into its own child path.',
    });
  });

  test("throws an error when pasting a folder into itself", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/parent/folder/")];
    act(() => {
      result.current.copy(source);
    });

    await expect(
      result.current.paste.mutateAsync({
        destination: "/parent/folder",
      })
    ).rejects.toThrow("Cannot paste folder into its own child path: folder");

    expect(mocks.notificationShow).toHaveBeenCalledWith({
      type: "error",
      title: "Invalid Operation",
      message: 'Cannot paste folder "folder" into its own child path.',
    });
  });

  test("allows pasting a folder into a parent folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return HttpResponse.json(
          {
            isRoot: false,
            folderAddress: "/parent/child/folder",
            subfolderAddresses: [],
            entries: [],
            nextContinuationHandle: null,
          } as ListStatResult,
          { status: 200 }
        );
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/parent/child/folder")];
    act(() => {
      result.current.copy(source);
    });

    await act(async () => {
      await result.current.paste.mutateAsync({
        destination: "/parent",
      });
    });

    expect(mocks.notificationShow).not.toHaveBeenCalledWith(
      expect.objectContaining({
        type: "error",
      })
    );
    expect(testNotification.complete).toHaveBeenCalled();
  });

  test("allows pasting a folder into a sibling folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return HttpResponse.json(
          {
            isRoot: false,
            folderAddress: "/parent/folder1",
            subfolderAddresses: [],
            entries: [],
            nextContinuationHandle: null,
          } as ListStatResult,
          { status: 200 }
        );
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/parent/folder1")];
    act(() => {
      result.current.copy(source);
    });

    await act(async () => {
      await result.current.paste.mutateAsync({
        destination: "/parent/folder2",
      });
    });

    expect(mocks.notificationShow).not.toHaveBeenCalledWith(
      expect.objectContaining({
        type: "error",
      })
    );
    expect(testNotification.complete).toHaveBeenCalled();
  });

  test("canPaste returns false when clipboard is empty", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    expect(result.current.canPaste(destination)).toBe(false);
  });

  test("canPaste returns false when pasting file onto itself", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    // Create files with full path matching the current folder
    const source = [createMockFile("/test-folder/file1.txt")];
    act(() => {
      result.current.copy(source);
    });

    // Trying to paste into the same folder where it already exists should be blocked
    expect(result.current.canPaste("/test-folder")).toBe(false);
  });

  test("canPaste returns true when pasting different files into the same folder", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    // Create files that are NOT in the test-folder
    const source = [createMockFile("/other-folder/file1.txt"), createMockFile("/other-folder/file2.txt")];
    act(() => {
      result.current.copy(source);
    });

    // sourcePath is "/test-folder" but files are from "/other-folder", so should be allowed
    expect(result.current.canPaste("/test-folder")).toBe(true);
  });

  test("canPaste returns true when pasting into a different folder", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("/test-folder/file1.txt"), createMockFile("/test-folder/file2.txt")];
    act(() => {
      result.current.copy(source);
    });

    // sourcePath is "/test-folder", so pasting into a different folder should be allowed
    expect(result.current.canPaste("/different-folder")).toBe(true);
  });

  test("canPaste returns false when any item in clipboard would conflict", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    // Mix of files - one that would conflict and one that wouldn't
    const source = [createMockFile("/test-folder/file1.txt"), createMockFile("/other-folder/file2.txt")];
    act(() => {
      result.current.copy(source);
    });

    // Even though file2 would be ok, file1 would conflict, so paste should be blocked
    expect(result.current.canPaste("/test-folder")).toBe(false);
  });

  test("sourcePath is correctly exposed from the hook", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("file1.txt")];
    act(() => {
      result.current.copy(source);
    });

    // sourcePath should match connection.currentFolder
    expect(result.current.sourcePath).toBe("/test-folder");
  });

  test("canPaste returns false when folder would overwrite itself", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/test-folder/subfolder")];
    act(() => {
      result.current.copy(source);
    });

    // Trying to paste the folder back into the same location
    expect(result.current.canPaste("/test-folder")).toBe(false);
  });

  test("canPaste returns false when folder with trailing slash would overwrite itself", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/test-folder/subfolder/")];
    act(() => {
      result.current.copy(source);
    });

    // Should handle trailing slashes correctly
    expect(result.current.canPaste("/test-folder")).toBe(false);
  });

  test("canPaste returns true when pasting different folder into the same parent", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/other-folder/subfolder")];
    act(() => {
      result.current.copy(source);
    });

    // Pasting a different folder into test-folder should be allowed
    expect(result.current.canPaste("/test-folder")).toBe(true);
  });

  test("canPaste returns false when mixing files and folders with conflicts", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFile("/test-folder/file.txt"), createMockFolder("/test-folder/subfolder")];
    act(() => {
      result.current.copy(source);
    });

    // Both items would conflict with their original locations
    expect(result.current.canPaste("/test-folder")).toBe(false);
  });

  test("canPaste returns true when pasting folders into different location", () => {
    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    const source = [createMockFolder("/test-folder/subfolder")];
    act(() => {
      result.current.copy(source);
    });

    // Pasting into a different folder should be allowed
    expect(result.current.canPaste("/different-folder")).toBe(true);
  });

  test("limits concurrent copy requests when pasting a folder", async () => {
    const testNotification = createMockNotification();
    mocks.fileActionNotification.mockReturnValue(testNotification);

    const fileCount = 200;
    const entries = Array.from({ length: fileCount }, (_, i) => ({
      resourceAddress: `folder/file${i}.txt`,
      resourceIdentity: `folder/file${i}.txt-identity`,
      metadata: {
        dataObjectSize: 100,
        lastModifiedTimestamp: new Date().toISOString(),
      },
    }));

    server.use(
      http.get("https://test-endpoint/v1alpha/filefolder/liststat/*", () => {
        return HttpResponse.json(
          {
            isRoot: false,
            folderAddress: "folder",
            subfolderAddresses: [],
            entries,
            nextContinuationHandle: null,
          } as ListStatResult,
          { status: 200 }
        );
      })
    );

    let concurrentRequests = 0;
    let maxConcurrentRequests = 0;

    server.use(
      http.post("https://test-endpoint/v1alpha/fileobject/by-identity/:resource_identity/copy", async ({ request }) => {
        concurrentRequests++;
        maxConcurrentRequests = Math.max(maxConcurrentRequests, concurrentRequests);
        // Simulate network latency so concurrent requests overlap
        await new Promise((resolve) => setTimeout(resolve, 10));
        concurrentRequests--;

        const { destination_resource_address } = (await request.json()) as {
          destination_resource_address: string;
        };
        return HttpResponse.json({ resourceIdentity: `${destination_resource_address}-identity` } as CopyResponse, {
          status: 200,
        });
      })
    );

    const { result } = renderHook(() => usePathClipboard(endpoint), { wrapper });
    act(() => {
      result.current.copy([createMockFolder("folder")]);
    });
    await act(async () => {
      await result.current.paste.mutateAsync({ destination });
    });

    expect(testNotification.complete).toHaveBeenCalled();
    expect(maxConcurrentRequests).toBeLessThanOrEqual(15);
  });
});
