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
import { renderHook } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { Connection } from "../state/appStore";
import { folderContentQueryKey } from "./useFolderContentQuery";
import { useNewFolderMutation } from "./useNewFolderMutation";

const endpoint = "http://storage.local";
const currentFolder = "file-storage://fileservice/test-folder/";

const mockAuth = createTestAuth();

const activeConnection: Connection = {
  endpoint,
  currentFolder,
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

const mockUseActiveConnection = vi.fn().mockReturnValue(activeConnection);

vi.mock("@/state/appStore", () => ({
  useActiveConnection: () => mockUseActiveConnection() as Connection | undefined,
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

const mockShow = vi.fn();
vi.mock("./useNotification", () => ({
  useNotification: () => ({
    show: mockShow,
  }),
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

describe("useNewFolderMutation", () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>
  );

  const server = setupServer();

  beforeAll(() => server.listen());
  afterEach(() => {
    server.resetHandlers();
    mockShow.mockClear();
    mockUseActiveConnection.mockReturnValue(activeConnection);
  });
  afterAll(() => server.close());

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates a new folder", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, () => {
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "new-test-folder";
    const response = await result.current.mutateAsync(folderName);
    expect(response).toBeUndefined();

    expect(mockShow).toHaveBeenCalledWith({
      type: "success",
      message: `New folder '${folderName}' has been created.`,
    });
  });

  it("handles API errors when creating a folder", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, () => {
        return new HttpResponse(JSON.stringify({ error: "Server error" }), {
          status: 500,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "error-folder";
    await expect(result.current.mutateAsync(folderName)).rejects.toThrow();

    expect(mockShow).not.toHaveBeenCalled();
  });

  it("handles 409 conflict error when folder already exists as a file", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, () => {
        return new HttpResponse(JSON.stringify({ error: "Conflict" }), {
          status: 409,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "conflict-folder";
    await expect(result.current.mutateAsync(folderName)).rejects.toThrow("resource already exists as a file");

    expect(mockShow).not.toHaveBeenCalled();
  });

  it("creates a folder with the correct path", async () => {
    let capturedUrl = "";

    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, ({ request }) => {
        capturedUrl = request.url;
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "path-test-folder";
    await result.current.mutateAsync(folderName);

    const expectedPath = encodeURIComponent(`${currentFolder}${folderName}`);
    expect(capturedUrl).toContain(expectedPath);
  });

  it("encodes folder name with special characters in the request", async () => {
    let capturedUrl = "";

    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, ({ request }) => {
        capturedUrl = request.url;
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "my folder";
    await result.current.mutateAsync(folderName);

    // Hook uses encodeURIComponent(name); service then encodes the full address
    expect(capturedUrl).toContain("my%2520folder");
  });

  it("throws an error when no active connection is available", async () => {
    mockUseActiveConnection.mockReturnValueOnce(undefined);

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper });

    const folderName = "no-connection-folder";
    await expect(result.current.mutateAsync(folderName)).rejects.toThrow(
      "No active connection is available for creating a new folder."
    );
  });

  it("invalidates queries after successful folder creation", async () => {
    const queryClient = createTestQueryClient();
    const invalidateQueriesSpy = vi.spyOn(queryClient, "invalidateQueries");

    const customWrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    server.use(
      http.put(`${endpoint}/v1alpha/filefolder/*`, () => {
        return new HttpResponse(null, { status: 204 });
      })
    );

    const { result } = renderHook(() => useNewFolderMutation(), { wrapper: customWrapper });

    await result.current.mutateAsync("invalidation-test-folder");

    expect(invalidateQueriesSpy).toHaveBeenCalledWith({
      queryKey: folderContentQueryKey(endpoint, currentFolder),
    });
  });
});
