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

import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, vi } from "vitest";
import { OpenId } from "@/pages/OpenId/OpenId";
import type { Authentication, AuthenticationState } from "@/services/auth";
import * as auth from "@/services/auth";
import { Connection, useAppStore } from "@/state/appStore";
import { render } from "@/test-utils";
import { createTestQueryClient } from "@/test-utils/utils";

const mocks = vi.hoisted(() => ({ getApi: vi.fn() }));

vi.mock("@/hooks/useAppConfig", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/hooks/useAppConfig")>();
  return {
    ...actual,
    useAppConfig: () => ({ defaultDiscovery: "" }),
  };
});

describe("OpenId", () => {
  const state: AuthenticationState = {
    urls: {
      discovery: "http://discovery",
    },
  };

  vi.mock("@/services/auth", async () => {
    const module = await vi.importActual<typeof import("@/services/auth")>("@/services/auth");
    return {
      decodeState: module.decodeState,
      encodeState: module.encodeState,
      exchangeCode: vi.fn((): Promise<Authentication> => {
        return Promise.resolve({
          accessToken: "test-access-token",
          idToken: "test-id-token",
          expiresAt: new Date(),
        });
      }),
      getAuthConfig: vi.fn(() => Promise.resolve({ openidConfiguration: "", clients: {} })),
    };
  });

  vi.mock("@/hooks/useServiceApi", () => {
    return {
      useServiceApi: () => ({
        getApi: mocks.getApi,
        createServiceApi: vi.fn(),
      }),
    };
  });

  beforeEach(() => {
    const now = new Date();
    vi.setSystemTime(now);
    mocks.getApi.mockResolvedValue({
      url: "http://storage/v1alpha",
      baseUrl: "http://storage",
      versionPath: "/v1alpha",
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("exchanges code", async () => {
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <MemoryRouter initialEntries={[`/openid?code=test&state=${auth.encodeState(state)}`]}>
          <Routes>
            <Route path="/openid" element={<OpenId />} />
            <Route path="/" element={<></>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    const { result } = renderHook(() => useAppStore());
    await waitFor(() =>
      expect(result.current.connections["http://storage"]).toEqual({
        endpoint: "http://storage",
        currentFolder: "",
        selectedPaths: new Set(),
        info: {
          auth: {
            accessToken: "test-access-token",
            idToken: "test-id-token",
            expiresAt: new Date(),
          },
          discovery: "http://discovery",
        },
      } satisfies Connection)
    );

    await waitFor(() => expect(result.current.activeConnection).toBe("http://storage"));
  });

  it("displays an error if auth config is not available", async () => {
    vi.spyOn(auth, "getAuthConfig").mockResolvedValueOnce(null);

    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <MemoryRouter initialEntries={[`/openid?code=test&state=${auth.encodeState(state)}`]}>
          <Routes>
            <Route path="/openid" element={<OpenId />} />
            <Route path="/" element={<></>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    const error = await screen.findByText(
      /Failed to finish authentication -- authentication configuration is not available/
    );
    expect(error).toBeVisible();
  });

  it("displays an error from the identity provider", async () => {
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <MemoryRouter initialEntries={[`/openid?error=invalid_client&error_description=Test%20external%20error`]}>
          <Routes>
            <Route path="/openid" element={<OpenId />} />
            <Route path="/" element={<></>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    const error = await screen.findByText(/invalid_client/);
    expect(error).toBeVisible();

    const errorDescription = await screen.findByText(/Test external error/);
    expect(errorDescription).toBeVisible();
  });

  it("displays an internal error", async () => {
    vi.spyOn(auth, "exchangeCode").mockImplementationOnce(() => {
      throw new Error("Test error.");
    });

    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <MemoryRouter initialEntries={[`/openid?code=test&state=${auth.encodeState(state)}`]}>
          <Routes>
            <Route path="/openid" element={<OpenId />} />
            <Route path="/" element={<></>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    const error = await screen.findByText(/Test error./);
    expect(error).toBeVisible();
  });

  it("displays an internal error if discovery does not have any available endpoints", async () => {
    mocks.getApi.mockResolvedValueOnce(null);

    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <MemoryRouter initialEntries={[`/openid?code=test&state=${auth.encodeState(state)}`]}>
          <Routes>
            <Route path="/openid" element={<OpenId />} />
            <Route path="/" element={<></>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );

    const error = await screen.findByText(/This deployment does not have any Storage API available for Navigator./);
    expect(error).toBeVisible();
  });
});
