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

import { mkdirSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { readConfig, writeConfig } from "@/config";
import { addHost, listHosts, listServices, removeHost, useHost } from "./host";

vi.mock("os", async (importOriginal) => {
  const orig = await importOriginal<typeof import("os")>();
  return { ...orig, homedir: () => testHome };
});

let testHome: string;
beforeEach(() => {
  testHome = join(tmpdir(), `sn-test-${Date.now()}`);
  mkdirSync(testHome, { recursive: true });
});
afterEach(() => {
  vi.restoreAllMocks();
  server.resetHandlers(); // reset per-test handler overrides
});

const server = setupServer(
  http.get(
    "https://discovery.example.com/api/v2/services",
    () => new HttpResponse(null, { status: 404 }),
  ),
  http.get("https://discovery.example.com/api/v1/auth-config", () =>
    HttpResponse.json({
      openid_configuration: "https://idp/.well-known/openid-configuration",
      clients: { navigator: { client_id: "nav", scope: "openid" } },
    }),
  ),
  http.get("https://idp/.well-known/openid-configuration", () =>
    HttpResponse.json({
      authorization_endpoint: "https://idp/auth",
      token_endpoint: "https://idp/token",
      device_authorization_endpoint: "https://idp/device",
    }),
  ),
  http.get("https://discovery.example.com/api/v1/services", () =>
    HttpResponse.json({
      services: [
        {
          id: "s1",
          name: "Storage",
          type: "storage",
          rest: "https://storage.example.com",
        },
      ],
    }),
  ),
  http.post("https://idp/device", () =>
    HttpResponse.json({
      device_code: "dev",
      user_code: "USER",
      verification_uri: "https://idp/activate",
      expires_in: 300,
      interval: 0,
    }),
  ),
  http.post("https://idp/token", () =>
    HttpResponse.json({
      access_token: "at",
      id_token: "it",
      refresh_token: "rt",
      expires_in: 3600,
    }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());

describe("addHost", () => {
  it("stores the advertised v2 filefolder service base URL", async () => {
    server.use(
      http.get("https://discovery.example.com/api/v2/services", () =>
        HttpResponse.json({
          "schema-version": 2,
          services: [
            {
              name: "storage-service",
              rest: {
                url: "https://storage-v2.example.com",
                endpoints: [
                  {
                    name: "filefolder",
                    versions: { v1alpha: { path: "/rest/v1alpha" } },
                  },
                ],
              },
            },
          ],
        }),
      ),
    );
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    await addHost("v2", "discovery.example.com");
    expect(readConfig().hosts.v2?.storageUrl).toBe(
      "https://storage-v2.example.com",
    );
  });

  it("adds host, runs device flow, and writes config", async () => {
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    await addHost("prod", "discovery.example.com");
    const config = readConfig();
    expect(config.hosts.prod).toBeDefined();
    expect(config.hosts.prod?.accessToken).toBe("at");
    expect(config.hosts.prod?.storageUrl).toBe("https://storage.example.com");
    expect(config.activeHost).toBe("prod");
  });

  it("authenticates first when /services returns 401 without a token", async () => {
    let servicesAuthHeader: string | null = null;
    server.use(
      http.get(
        "https://discovery.example.com/api/v1/services",
        ({ request }) => {
          const auth = request.headers.get("Authorization");
          servicesAuthHeader = auth;
          if (!auth) {
            return new HttpResponse(null, { status: 401 });
          }
          return HttpResponse.json({
            services: [
              {
                id: "s1",
                name: "Storage",
                type: "storage",
                rest: "https://storage.example.com",
              },
            ],
          });
        },
      ),
    );
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    await addHost("prod2", "discovery.example.com");
    expect(servicesAuthHeader).toBe("Bearer at");
    expect(readConfig().hosts.prod2?.storageUrl).toBe(
      "https://storage.example.com",
    );
  });
});

describe("useHost", () => {
  it("sets the active host", async () => {
    writeConfig({
      hosts: {
        prod: {
          discoveryUrl: "https://d",
          storageUrl: "https://s",
          authUrl: "https://a",
        },
      },
    });
    await useHost("prod");
    expect(readConfig().activeHost).toBe("prod");
  });

  it("throws if host does not exist", async () => {
    writeConfig({ hosts: {} });
    await expect(useHost("missing")).rejects.toThrow(
      "host 'missing' not found",
    );
  });
});

describe("removeHost", () => {
  it("removes the host from config", async () => {
    writeConfig({
      activeHost: "prod",
      hosts: {
        prod: {
          discoveryUrl: "https://d",
          storageUrl: "https://s",
          authUrl: "https://a",
        },
      },
    });
    await removeHost("prod");
    const config = readConfig();
    expect(config.hosts.prod).toBeUndefined();
    expect(config.activeHost).toBeUndefined();
  });

  it("throws if host does not exist", async () => {
    writeConfig({ hosts: {} });
    await expect(removeHost("ghost")).rejects.toThrow("host 'ghost' not found");
  });
});

describe("listHosts", () => {
  it("returns empty array when no hosts configured", () => {
    writeConfig({ hosts: {} });
    expect(listHosts()).toEqual([]);
  });

  it("returns list of host names with active marker", () => {
    writeConfig({
      activeHost: "prod",
      hosts: {
        prod: {
          discoveryUrl: "https://d",
          storageUrl: "https://s",
          authUrl: "https://a",
        },
        dev: {
          discoveryUrl: "https://d2",
          storageUrl: "https://s2",
          authUrl: "https://a2",
        },
      },
    });
    const result = listHosts();
    expect(result.find((h) => h.name === "prod")?.active).toBe(true);
    expect(result.find((h) => h.name === "dev")?.active).toBe(false);
  });
});

describe("listServices", () => {
  it("returns services for the active host when no name given", async () => {
    writeConfig({
      activeHost: "prod",
      hosts: {
        prod: {
          discoveryUrl: "https://discovery.example.com",
          storageUrl: "https://s",
          authUrl: "https://a",
        },
      },
    });
    const services = await listServices();
    expect(services).toHaveLength(1);
    expect(services[0]?.name).toBe("Storage");
  });

  it("returns services for a named host", async () => {
    writeConfig({
      hosts: {
        dev: {
          discoveryUrl: "https://discovery.example.com",
          storageUrl: "https://s",
          authUrl: "https://a",
        },
      },
    });
    const services = await listServices("dev");
    expect(services).toHaveLength(1);
    expect(services[0]?.rest).toBe("https://storage.example.com");
  });

  it("throws when named host is not in config", async () => {
    writeConfig({ hosts: {} });
    await expect(listServices("ghost")).rejects.toThrow(
      "host 'ghost' not found",
    );
  });

  it("throws when no name given and no active host", async () => {
    writeConfig({ hosts: {} });
    await expect(listServices()).rejects.toThrow("No active host");
  });
});
