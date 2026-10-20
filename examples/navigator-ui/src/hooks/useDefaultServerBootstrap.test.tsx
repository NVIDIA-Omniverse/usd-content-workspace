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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DiscoveredApi } from "@/hooks/useServiceApi";
import { useAppStore } from "@/state/appStore";
import { createTestQueryClient } from "@/test-utils/utils";
import { useDefaultServerBootstrap } from "./useDefaultServerBootstrap";

const mocks = vi.hoisted(() => ({
  defaultDiscovery: "",
  getApi: vi.fn((_name: string): Promise<unknown> => Promise.resolve(null)),
}));

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => ({ defaultDiscovery: mocks.defaultDiscovery }),
  };
});

vi.mock("@/hooks/useServiceApi", () => ({
  useServiceApi: () => ({
    getApi: (_discovery: string, name: string) => mocks.getApi(name),
    createServiceApi: vi.fn(),
  }),
}));

const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>
);

describe("useDefaultServerBootstrap", () => {
  beforeEach(() => {
    act(() => {
      useAppStore.setState(useAppStore.getInitialState());
    });
    mocks.defaultDiscovery = "";
    mocks.getApi.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("does nothing when defaultDiscovery is empty", () => {
    const { result } = renderHook(() => useDefaultServerBootstrap(), { wrapper });

    expect(result.current.fetchStatus).toBe("idle");
    expect(mocks.getApi).not.toHaveBeenCalled();
    expect(useAppStore.getState().endpoints.size).toBe(0);
  });

  it("syncs the storage endpoint from the configured discovery", async () => {
    mocks.defaultDiscovery = "https://stack.example/";
    const storageApi: DiscoveredApi = {
      url: "http://storage-1/v1alpha",
      baseUrl: "http://storage-1/",
      versionPath: "/v1alpha",
      version: "v1alpha",
    };
    mocks.getApi.mockResolvedValueOnce(storageApi);

    const { result } = renderHook(() => useDefaultServerBootstrap(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const state = useAppStore.getState();
    expect(state.connections["http://storage-1/"]).toBeDefined();
    expect(state.activeConnection).toBe("http://storage-1/");
    expect(mocks.getApi).toHaveBeenCalledWith("filefolder");
  });

  it("exposes the error and leaves the store empty when the discovery call fails", async () => {
    mocks.defaultDiscovery = "https://stack.example/";
    mocks.getApi.mockRejectedValueOnce(new Error("network down"));

    const { result } = renderHook(() => useDefaultServerBootstrap(), { wrapper });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.error).toBeInstanceOf(Error);
    expect(useAppStore.getState().endpoints.size).toBe(0);
  });
});
