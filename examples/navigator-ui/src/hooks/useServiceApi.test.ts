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

import { renderHook } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { DiscoveryStackInfo } from "@/services/discoveryService";
import { useAppStore } from "@/state/appStore";
import { useServiceApi } from "./useServiceApi";

describe("useServiceApi", () => {
  const hostname = "discovery";

  const mockStackInfo: DiscoveryStackInfo = {
    services: [
      {
        id: "service1",
        name: "Storage Service",
        type: "storage",
        grpc: "http://grpc-endpoint",
        rest: "http://rest-endpoint",
      },
      {
        id: "service2",
        name: "Permission Service",
        type: "permission",
        grpc: "http://permission-grpc",
      },
    ],
  };

  const mockV2Info = {
    "schema-version": 2,
    services: [
      {
        name: "storage-service",
        rest: {
          url: "https://storage-v2",
          endpoints: [
            { name: "capabilities", versions: { v1alpha: { path: "/v1alpha" } } },
            { name: "filefolder", versions: { v1alpha: { path: "/v1alpha" } } },
            // The storage service dual-serves metadata.
            { name: "metadata", versions: { v1alpha: { path: "/v1alpha" }, v1beta: { path: "/v1beta" } } },
          ],
        },
      },
      {
        name: "event-consumer-service",
        rest: {
          url: "https://events-v2",
          endpoints: [{ name: "notifications-consumer", versions: { v1beta: { path: "/api/v1beta" } } }],
        },
      },
      { name: "live-edit-service" },
    ],
  };

  const server = setupServer(
    http.get(`https://${hostname}/api/v1/services`, () => {
      return HttpResponse.json(mockStackInfo);
    }),
    // v2 unsupported by default; individual tests opt in.
    http.get(`https://${hostname}/api/v2/services`, () => {
      return new HttpResponse(null, { status: 404 });
    })
  );

  const getApi = () => renderHook(() => useServiceApi()).result.current.getApi;

  beforeAll(() => {
    server.listen();
  });

  afterAll(() => {
    server.close();
  });

  beforeEach(() => {
    useAppStore.getState().clearDiscoveryCache();
  });

  afterEach(() => {
    server.resetHandlers();
    vi.resetAllMocks();
  });

  it("should fall back to v1 and append the hardcoded path when v2 is unavailable", async () => {
    const result = await getApi()(hostname, "filefolder");
    expect(result).toEqual({
      baseUrl: "http://rest-endpoint",
      versionPath: "/v1alpha",
      version: "v1alpha",
      url: "http://rest-endpoint/v1alpha",
    });
  });

  it("should return null on the v1 fallback when the service has no rest endpoint", async () => {
    const result = await getApi()(hostname, "permission");
    expect(result).toBeNull();
  });

  it("should resolve from v2 by matching rest.endpoints[].name and version path", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json(mockV2Info);
      })
    );

    const result = await getApi()(hostname, "filefolder");
    expect(result).toEqual({
      baseUrl: "https://storage-v2",
      versionPath: "/v1alpha",
      version: "v1alpha",
      url: "https://storage-v2/v1alpha",
    });
  });

  it("should resolve a non-storage v2 API using its own base URL", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json(mockV2Info);
      })
    );

    const result = await getApi()(hostname, "notifications-consumer");
    expect(result).toEqual({
      baseUrl: "https://events-v2",
      versionPath: "/api/v1beta",
      version: "v1beta",
      url: "https://events-v2/api/v1beta",
    });
  });

  it("should return null when v2 is supported but does not advertise the API", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json(mockV2Info);
      })
    );

    const result = await getApi()(hostname, "thumbnail");
    expect(result).toBeNull();
  });

  it("should prefer the v1beta metadata API when the deployment advertises both versions", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json(mockV2Info);
      })
    );

    const result = await getApi()(hostname, "metadata");
    expect(result).toEqual({
      baseUrl: "https://storage-v2",
      versionPath: "/v1beta",
      version: "v1beta",
      url: "https://storage-v2/v1beta",
    });
  });

  it("should fall back to the v1alpha metadata API when v1beta is not advertised", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json({
          ...mockV2Info,
          services: [
            {
              name: "storage-service",
              rest: {
                url: "https://storage-v2",
                endpoints: [{ name: "metadata", versions: { v1alpha: { path: "/v1alpha" } } }],
              },
            },
          ],
        });
      })
    );

    const result = await getApi()(hostname, "metadata");
    expect(result).toEqual({
      baseUrl: "https://storage-v2",
      versionPath: "/v1alpha",
      version: "v1alpha",
      url: "https://storage-v2/v1alpha",
    });
  });

  it("should resolve the metadata API as v1alpha on the v1 discovery fallback", async () => {
    const result = await getApi()(hostname, "metadata");
    expect(result).toEqual({
      baseUrl: "http://rest-endpoint",
      versionPath: "/v1alpha",
      version: "v1alpha",
      url: "http://rest-endpoint/v1alpha",
    });
  });

  it("should tag the created api instance with the resolved version", async () => {
    server.use(
      http.get(`https://${hostname}/api/v2/services`, () => {
        return HttpResponse.json(mockV2Info);
      })
    );

    const { createServiceApi } = renderHook(() => useServiceApi()).result.current;
    const api = await createServiceApi(hostname, "metadata");
    expect(api?.apiVersion).toBe("v1beta");
    expect(api?.serviceUrl).toBe("https://storage-v2/v1beta");
  });
});
