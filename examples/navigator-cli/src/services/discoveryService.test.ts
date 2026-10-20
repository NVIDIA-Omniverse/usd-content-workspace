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

import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { discoveryService, type ApiName } from "./discoveryService";

const mockAuthConfig = {
  openid_configuration: "https://idp/.well-known/openid-configuration",
  clients: {
    navigator: { client_id: "nav-client", scope: "openid" },
  },
};

const mockServices = {
  services: [
    {
      id: "s1",
      name: "Storage",
      type: "storage",
      rest: "https://storage.example.com",
    },
    { id: "s2", name: "Perms", type: "permission" },
  ],
};

const server = setupServer(
  http.get(
    "https://discovery.example.com/api/v2/services",
    () => new HttpResponse(null, { status: 404 }),
  ),
  http.get("https://discovery.example.com/api/v1/auth-config", () =>
    HttpResponse.json(mockAuthConfig),
  ),
  http.get("https://discovery.example.com/api/v1/services", () =>
    HttpResponse.json(mockServices),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("discoveryService", () => {
  it("resolves every supported API from v2 endpoints", async () => {
    const names: ApiName[] = [
      "capabilities",
      "filefolder",
      "fileobject",
      "metadata",
      "versioning",
      "notifications-consumer",
      "permission",
      "userinfo",
      "thumbnail",
      "archive",
    ];
    server.use(
      http.get("https://discovery.example.com/api/v2/services", () =>
        HttpResponse.json({
          "schema-version": 2,
          services: [
            {
              name: "all",
              rest: {
                url: "https://api.example.com",
                endpoints: names.map((name) => ({
                  name,
                  versions: {
                    v1alpha: { path: `/${name}/v1alpha` },
                    v1beta: { path: `/${name}/v1beta` },
                  },
                })),
              },
            },
          ],
        }),
      ),
    );
    const resolved = await discoveryService(
      "discovery.example.com",
    ).resolveApis();
    for (const name of names) {
      const version = [
        "metadata",
        "notifications-consumer",
        "permission",
      ].includes(name)
        ? "v1beta"
        : "v1alpha";
      expect(resolved[name]?.url).toBe(
        `https://api.example.com/${name}/${version}`,
      );
    }
  });

  it("resolves every supported API from v1 service types", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v1/services", () =>
        HttpResponse.json({
          services: [
            {
              id: "storage",
              name: "Storage",
              type: "storage",
              rest: "https://storage.example.com",
            },
            {
              id: "notifications",
              name: "Notifications",
              type: "notification-consumer",
              rest: "https://notifications.example.com",
            },
            {
              id: "permission",
              name: "Permission",
              type: "permission",
              rest: "https://permission.example.com",
            },
            {
              id: "userinfo",
              name: "Userinfo",
              type: "userinfo",
              rest: "https://userinfo.example.com",
            },
            {
              id: "thumbnail",
              name: "Thumbnail",
              type: "thumbnail",
              rest: "https://thumbnail.example.com",
            },
            {
              id: "archive",
              name: "Archive",
              type: "archive",
              rest: "https://archive.example.com",
            },
          ],
        }),
      ),
    );
    const resolved = await discoveryService(
      "discovery.example.com",
    ).resolveApis();
    for (const name of [
      "capabilities",
      "filefolder",
      "fileobject",
      "metadata",
      "versioning",
    ] as ApiName[])
      expect(resolved[name]?.url).toBe("https://storage.example.com/v1alpha");
    expect(resolved["notifications-consumer"]?.url).toBe(
      "https://notifications.example.com/api/v1beta",
    );
    expect(resolved.permission?.url).toBe(
      "https://permission.example.com/v1beta",
    );
    expect(resolved.userinfo?.url).toBe(
      "https://userinfo.example.com/api/v1alpha",
    );
    expect(resolved.thumbnail?.url).toBe(
      "https://thumbnail.example.com/v1alpha",
    );
    expect(resolved.archive?.url).toBe("https://archive.example.com/v1alpha");
  });

  it("prefers a discovered v2 metadata version and reports absent APIs", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v2/services", () =>
        HttpResponse.json({
          "schema-version": 2,
          services: [
            {
              name: "storage-service",
              rest: {
                url: "https://storage.example.com",
                endpoints: [
                  {
                    name: "metadata",
                    versions: {
                      v1alpha: { path: "/v1alpha" },
                      v1beta: { path: "/v1beta" },
                    },
                  },
                ],
              },
            },
          ],
        }),
      ),
    );
    const service = discoveryService("discovery.example.com");
    expect(await service.resolveApi("metadata")).toEqual({
      baseUrl: "https://storage.example.com",
      versionPath: "/v1beta",
      version: "v1beta",
      url: "https://storage.example.com/v1beta",
    });
    expect(await service.resolveApi("fileobject")).toBeNull();
  });

  it("falls back to v1 when v2 is malformed", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v2/services", () =>
        HttpResponse.json({ services: [] }),
      ),
    );
    expect(
      (await discoveryService("discovery.example.com").resolveApi("filefolder"))
        ?.url,
    ).toBe("https://storage.example.com/v1alpha");
  });

  it("returns null when v2 advertises only unsupported endpoint versions", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v2/services", () =>
        HttpResponse.json({
          "schema-version": 2,
          services: [
            {
              name: "future",
              rest: {
                url: "https://storage.example.com",
                endpoints: [
                  { name: "metadata", versions: { v9: { path: "/v9" } } },
                ],
              },
            },
          ],
        }),
      ),
    );
    const service = discoveryService("discovery.example.com");
    expect(await service.resolveApi("metadata")).toBeNull();
    expect(await service.resolveApi("archive")).toBeNull();
  });

  it("resolves the optional archive service through v1 fallback", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v1/services", () =>
        HttpResponse.json({
          services: [
            ...mockServices.services,
            {
              id: "archive",
              name: "Archive",
              type: "archive",
              rest: "https://archive.example.com",
            },
          ],
        }),
      ),
    );
    expect(
      await discoveryService("discovery.example.com").resolveApi("archive"),
    ).toEqual({
      baseUrl: "https://archive.example.com",
      versionPath: "/v1alpha/",
      version: "v1alpha",
      url: "https://archive.example.com/v1alpha",
    });
  });
  it("builds URL from hostname", () => {
    const s = discoveryService("discovery.example.com");
    expect(s.url).toBe("https://discovery.example.com/api/v1/");
  });

  it("builds URL from full URL", () => {
    const s = discoveryService("https://discovery.example.com");
    expect(s.url).toBe("https://discovery.example.com/api/v1/");
  });

  it("getAuthConfig returns parsed config", async () => {
    const result = await discoveryService(
      "discovery.example.com",
    ).getAuthConfig();
    expect(result?.clients.navigator?.clientId).toBe("nav-client");
  });

  it("getAuthConfig returns null on 404", async () => {
    server.use(
      http.get(
        "https://discovery.example.com/api/v1/auth-config",
        () => new HttpResponse(null, { status: 404 }),
      ),
    );
    const result = await discoveryService(
      "discovery.example.com",
    ).getAuthConfig();
    expect(result).toBeNull();
  });

  it("getStackEntry filters by type with rest endpoint", async () => {
    const entries = await discoveryService(
      "discovery.example.com",
    ).getStackEntry("storage");
    expect(entries).toHaveLength(1);
    expect(entries[0]?.rest).toBe("https://storage.example.com");
  });

  it("getAllServices returns all services without filtering", async () => {
    const entries = await discoveryService(
      "discovery.example.com",
    ).getAllServices();
    expect(entries).toHaveLength(2);
    expect(entries[0]?.name).toBe("Storage");
    expect(entries[1]?.name).toBe("Perms");
  });
});
