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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fileImage from "@/assets/file.png";
import folderImage from "@/assets/folder.png";
import serverImage from "@/assets/server.png";
import { useAppStore } from "@/state/appStore";
import { Connection } from "@/state/slices/connectionSlice";
import { createMockFile, createMockFolder, createMockTopLevelAddress } from "@/test-utils/mockPath";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { thumbnailResourceQueryKey, usePathThumbnail } from "./usePathThumbnail";

const mockAuth = createTestAuth();

// Keep real `create` so ky requests actually flow through MSW; stub `getApiOptions`
// to avoid the getState() → appStore → zustand init chain.
vi.mock("@/services/api", async () => {
  const actual = await vi.importActual<typeof import("@/services/api")>("@/services/api");
  return {
    ...actual,
    getApiOptions: () => ({
      discovery: "https://discovery.example.com",
      auth: {
        get: () => mockAuth,
        set: vi.fn(),
      },
    }),
  };
});

const THUMBNAIL_SERVICE_URL = "https://thumbnail-service.example.com";
const DISCOVERY_SERVICES_URL = "https://discovery.example.com/api/v1/services";
const DISCOVERY_V2_SERVICES_URL = "https://discovery.example.com/api/v2/services";
const THUMBNAILS_URL = "https://thumbnail-service.example.com/v1alpha/thumbnails";
const DOWNLOAD_URL = "https://test-endpoint.com/v1alpha/fileobject/by-identity/*";

// v2 discovery is unsupported by this deployment, so callers fall back to v1.
const server = setupServer(http.get(DISCOVERY_V2_SERVICES_URL, () => new HttpResponse(null, { status: 404 })));

const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>
);

const connection: Connection = {
  endpoint: "https://test-endpoint.com",
  currentFolder: "",
  selectedPaths: new Set(),
  info: {
    auth: createTestAuth(),
    discovery: "https://discovery.example.com",
  },
};

beforeEach(() => {
  server.listen();
  // Discovery documents are cached per connection for the session, so drop the cache between
  // tests to keep each case isolated to its own MSW handlers.
  useAppStore.getState().clearDiscoveryCache();
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:mock-object-url");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
});

afterEach(() => {
  vi.useRealTimers();
  server.resetHandlers();
  server.close();
  vi.clearAllMocks();
  vi.restoreAllMocks();
});

describe("usePathThumbnail", () => {
  it("returns the default file image when the thumbnail service is unavailable", async () => {
    server.use(http.get(DISCOVERY_SERVICES_URL, () => HttpResponse.json({ services: [] })));

    const path = createMockFile("file-id");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    // Initially placeholder before discovery resolves
    expect(result.current).toBe(fileImage);

    // Still placeholder after discovery returns no thumbnail service
    await waitFor(() => {
      expect(result.current).toBe(fileImage);
    });
  });

  it("returns the default folder image when path is a folder and service is unavailable", () => {
    server.use(http.get(DISCOVERY_SERVICES_URL, () => HttpResponse.json({ services: [] })));

    const path = createMockFolder("folder1");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    expect(result.current).toBe(folderImage);
  });

  it("returns the default server image when path is a top-level address", () => {
    // Top-level addresses never query the thumbnail service
    const path = createMockTopLevelAddress("bucket1");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    expect(result.current).toBe(serverImage);
  });

  it("debounces thumbnail discovery until the path is stable for 150ms", async () => {
    vi.useFakeTimers();
    let discoveryRequests = 0;

    server.use(
      http.get(DISCOVERY_SERVICES_URL, () => {
        discoveryRequests += 1;
        return HttpResponse.json({ services: [] });
      })
    );

    const firstPath = createMockFile("first.png");
    const secondPath = createMockFile("second.png");
    const { rerender } = renderHook(({ path }) => usePathThumbnail(path, connection), {
      initialProps: { path: firstPath },
      wrapper,
    });

    expect(discoveryRequests).toBe(0);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(149);
    });
    expect(discoveryRequests).toBe(0);

    rerender({ path: secondPath });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(discoveryRequests).toBe(0);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(149);
    });
    vi.useRealTimers();
    await waitFor(() => {
      expect(discoveryRequests).toBe(1);
    });
  });

  it("requests a 256x256 thumbnail and downloads the first returned resource identity", async () => {
    const thumbnailResourceId = "thumb-resource-id";
    let capturedWidth: string | null = null;
    let capturedHeight: string | null = null;

    server.use(
      http.get(DISCOVERY_SERVICES_URL, () =>
        HttpResponse.json({
          services: [
            { id: "thumbnail", name: "Thumbnail Service", type: "thumbnail", rest: THUMBNAIL_SERVICE_URL },
            { id: "storage", name: "Storage Service", type: "storage", rest: "https://test-endpoint.com" },
          ],
        })
      ),
      http.get(THUMBNAILS_URL, ({ request }) => {
        const searchParams = new URL(request.url).searchParams;
        capturedWidth = searchParams.get("width");
        capturedHeight = searchParams.get("height");
        return HttpResponse.json({
          thumbnails: [
            {
              thumbnail: { resource_address: "thumb-address", resource_identity: thumbnailResourceId },
            },
          ],
        });
      }),
      http.get(
        DOWNLOAD_URL,
        () => new Response("binary-data", { status: 200, headers: { "Content-Type": "image/png" } })
      )
    );

    vi.mocked(URL.createObjectURL).mockReturnValue("blob:mock-url");

    // Files use resourceAddress as the association_target_resource_address
    const path = createMockFile("file1.png");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    await waitFor(() => {
      expect(result.current).toBe("blob:mock-url");
    });
    expect(capturedWidth).toBe("256");
    expect(capturedHeight).toBe("256");
  });

  it("uses resourceAddress as associationTarget for folders", async () => {
    let capturedResourceAddress: string | null = null;

    server.use(
      http.get(DISCOVERY_SERVICES_URL, () =>
        HttpResponse.json({
          services: [
            { id: "thumbnail", name: "Thumbnail Service", type: "thumbnail", rest: THUMBNAIL_SERVICE_URL },
            { id: "storage", name: "Storage Service", type: "storage", rest: "https://test-endpoint.com" },
          ],
        })
      ),
      http.get(THUMBNAILS_URL, ({ request }) => {
        capturedResourceAddress = new URL(request.url).searchParams.get("association_target_resource_address");
        return HttpResponse.json({
          thumbnails: [
            {
              thumbnail: { resource_address: "thumb-address", resource_identity: "folder-thumb-id" },
              specification: { image_to_image: { width: 256, height: 256 } },
            },
          ],
        });
      }),
      http.get(
        DOWNLOAD_URL,
        () => new Response("binary-data", { status: 200, headers: { "Content-Type": "image/png" } })
      )
    );

    vi.mocked(URL.createObjectURL).mockReturnValue("blob:folder-url");

    const path = createMockFolder("my-folder/path");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    await waitFor(() => {
      expect(result.current).toBe("blob:folder-url");
    });

    expect(capturedResourceAddress).toBe("my-folder/path");
  });

  it("returns the placeholder image when the thumbnail service returns no identities", async () => {
    let thumbnailsHandlerCalled = false;

    server.use(
      http.get(DISCOVERY_SERVICES_URL, () =>
        HttpResponse.json({
          services: [
            { id: "thumbnail", name: "Thumbnail Service", type: "thumbnail", rest: THUMBNAIL_SERVICE_URL },
            { id: "storage", name: "Storage Service", type: "storage", rest: "https://test-endpoint.com" },
          ],
        })
      ),
      http.get(THUMBNAILS_URL, () => {
        thumbnailsHandlerCalled = true;
        return HttpResponse.json({ thumbnails: [] });
      })
    );

    const path = createMockFile("file-empty.png");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    await waitFor(() => {
      expect(thumbnailsHandlerCalled).toBe(true);
    });

    expect(result.current).toBe(fileImage);
  });

  it("returns the placeholder image when the thumbnail download fails", async () => {
    let downloadHandlerCalled = false;

    server.use(
      http.get(DISCOVERY_SERVICES_URL, () =>
        HttpResponse.json({
          services: [
            { id: "thumbnail", name: "Thumbnail Service", type: "thumbnail", rest: THUMBNAIL_SERVICE_URL },
            { id: "storage", name: "Storage Service", type: "storage", rest: "https://test-endpoint.com" },
          ],
        })
      ),
      http.get(THUMBNAILS_URL, () =>
        HttpResponse.json({
          thumbnails: [
            {
              thumbnail: { resource_address: "bad-address", resource_identity: "bad-resource" },
              specification: { image_to_image: { width: 256, height: 256 } },
            },
          ],
        })
      ),
      http.get(DOWNLOAD_URL, () => {
        downloadHandlerCalled = true;
        return new Response(null, { status: 500 });
      })
    );

    const path = createMockFile("file-fail.png");
    const { result } = renderHook(() => usePathThumbnail(path, connection), { wrapper });

    await waitFor(() => {
      expect(downloadHandlerCalled).toBe(true);
    });

    expect(result.current).toBe(fileImage);
  });

  it("decodes the resource address so encoded variants share one query key", () => {
    const fullyEncoded = "buckets/photos/image%20%280%29.png";
    const spacesOnlyEncoded = "buckets/photos/image%20(0).png";

    const fullyEncodedKey = thumbnailResourceQueryKey(connection.endpoint, fullyEncoded);
    const spacesOnlyEncodedKey = thumbnailResourceQueryKey(connection.endpoint, spacesOnlyEncoded);

    expect(fullyEncodedKey).toEqual([connection.endpoint, "buckets/photos/image (0).png", "thumbnail", "resource"]);
    expect(spacesOnlyEncodedKey).toEqual(fullyEncodedKey);
  });
});
