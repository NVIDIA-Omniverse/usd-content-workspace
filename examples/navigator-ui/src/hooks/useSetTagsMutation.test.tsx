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
import { renderHook, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { Connection } from "@/state/slices/connectionSlice";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { useSetTagsMutation } from "./useSetTagsMutation";

const endpoint = "http://storage.local";
const mockAuth = createTestAuth();

const connection: Connection = {
  endpoint,
  currentFolder: "file-storage://fileservice/test-folder/",
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "",
  },
};

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

describe("useSetTagsMutation", () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>
  );

  const server = setupServer();

  beforeAll(() => server.listen());
  afterEach(() => {
    server.resetHandlers();
  });
  afterAll(() => server.close());

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("sets tags successfully with etag", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/metadata/*/tags`, async ({ request }) => {
        const body = await request.text();
        const tags = JSON.parse(body);
        const etag = request.headers.get("If-Match");

        // Verify the request includes the etag header
        expect(etag).toBe("test-etag-123");
        expect(tags).toEqual({ "test-key": "test-value", "another-key": "another-value" });

        return HttpResponse.json({});
      })
    );

    const { result } = renderHook(() => useSetTagsMutation(connection), { wrapper });

    const tags = { "test-key": "test-value", "another-key": "another-value" };
    const resourceAddress = "file-storage://fileservice/test-file.txt";
    const etag = "test-etag-123";

    await result.current.mutateAsync({ resourceAddress, metadata: tags, etag });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
  });

  it("handles API errors when setting tags", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/metadata/*/tags`, () => {
        return new HttpResponse(JSON.stringify({ error: "Server error" }), {
          status: 500,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    const { result } = renderHook(() => useSetTagsMutation(connection), { wrapper });

    const tags = { "test-key": "test-value" };
    const resourceAddress = "file-storage://fileservice/test-file.txt";
    const etag = "test-etag-123";

    await expect(result.current.mutateAsync({ resourceAddress, metadata: tags, etag })).rejects.toThrow();
  });

  it("handles 412 Precondition Failed when etag is outdated", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/metadata/*/tags`, () => {
        return new HttpResponse(JSON.stringify({ error: "Precondition Failed" }), {
          status: 412,
          headers: { "Content-Type": "application/json" },
        });
      })
    );

    const { result } = renderHook(() => useSetTagsMutation(connection), { wrapper });

    const tags = { "test-key": "test-value" };
    const resourceAddress = "file-storage://fileservice/test-file.txt";
    const etag = "outdated-etag";

    await expect(result.current.mutateAsync({ resourceAddress, metadata: tags, etag })).rejects.toThrow();
  });

  it("handles network errors", async () => {
    server.use(
      http.put(`${endpoint}/v1alpha/metadata/*/tags`, () => {
        return HttpResponse.error();
      })
    );

    const { result } = renderHook(() => useSetTagsMutation(connection), { wrapper });

    const tags = { "test-key": "test-value" };
    const resourceAddress = "file-storage://fileservice/test-file.txt";
    const etag = "test-etag-123";

    await expect(result.current.mutateAsync({ resourceAddress, metadata: tags, etag })).rejects.toThrow();
  });
});
