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
import {
  afterAll,
  afterEach,
  beforeAll,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { create, isHttpError } from "./api";
import { readConfig, updateConfig, type Config } from "@/config";
import * as auth from "./auth";

vi.mock("@/config", () => {
  let tail = Promise.resolve();
  return {
    readConfig: vi.fn(),
    updateConfig: vi.fn(),
    withHostAuthLock: (_host: string, action: () => Promise<void>) => {
      const result = tail.then(action);
      tail = result.catch(() => undefined);
      return result;
    },
  };
});

const server = setupServer(
  http.get("https://storage.example.com/v1alpha/test", ({ request }) => {
    const auth = request.headers.get("Authorization");
    if (auth === "Bearer valid-token") {
      return HttpResponse.json({ ok: true });
    }
    return new HttpResponse(null, { status: 401 });
  }),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
});

function authenticatedApi() {
  const config: Config = {
    hosts: {
      dev: {
        discoveryUrl: "https://discovery.example",
        storageUrl: "https://storage.example.com",
        authUrl: "",
        accessToken: "old",
        refreshToken: "refresh",
        tokenExpiry: new Date(Date.now() + 3600000).toISOString(),
      },
    },
  };
  vi.mocked(readConfig).mockImplementation(() => structuredClone(config));
  vi.mocked(updateConfig).mockImplementation((update) => {
    update(config);
    return Promise.resolve();
  });
  let count = 0;
  const refresh = vi.spyOn(auth, "refreshAuth").mockImplementation(() =>
    Promise.resolve({
      accessToken: `new-${++count}`,
      idToken: "id",
      refreshToken: "refresh",
      expiresAt: new Date(Date.now() + 3600000),
    }),
  );
  server.use(
    http.get("https://discovery.example/api/v1/auth-config", () =>
      HttpResponse.json({
        openid_configuration: "https://idp.example/config",
        clients: { navigator: { client_id: "client", scope: "openid" } },
      }),
    ),
  );
  return {
    api: create("https://storage.example.com", {
      hostName: "dev",
      discovery: "https://discovery.example",
    }),
    refresh,
  };
}

describe("authentication retries", () => {
  it("shares a refresh failure between concurrent requests", async () => {
    const { api, refresh } = authenticatedApi();
    refresh.mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
      throw new Error("identity provider unavailable");
    });
    server.use(
      http.get(
        "https://storage.example.com/parallel-failure",
        () => new HttpResponse(null, { status: 401 }),
      ),
    );
    const results = await Promise.allSettled(
      Array.from({ length: 20 }, () => api.get("parallel-failure")),
    );
    expect(results.every((result) => result.status === "rejected")).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("refreshes one rotating token for simultaneous unauthorized requests", async () => {
    const { api, refresh } = authenticatedApi();
    server.use(
      http.get("https://storage.example.com/parallel", ({ request }) =>
        request.headers.get("Authorization") === "Bearer old"
          ? new HttpResponse(null, { status: 401 })
          : HttpResponse.json({ ok: true }),
      ),
    );
    await Promise.all(Array.from({ length: 20 }, () => api.get("parallel")));
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("preserves a refresh token when the provider omits a replacement", async () => {
    const { api, refresh } = authenticatedApi();
    refresh.mockResolvedValue({
      accessToken: "new",
      idToken: "id",
      expiresAt: new Date(Date.now() + 3600000),
    });
    server.use(
      http.get("https://storage.example.com/once", ({ request }) =>
        request.headers.get("Authorization") === "Bearer old"
          ? new HttpResponse(null, { status: 401 })
          : HttpResponse.json({ ok: true }),
      ),
    );
    await api.get("once");
    expect(readConfig().hosts.dev.refreshToken).toBe("refresh");
  });

  it("retains a per-request timeout on authentication retries", async () => {
    const { api } = authenticatedApi();
    server.use(
      http.get("https://storage.example.com/slow", async ({ request }) => {
        if (request.headers.get("Authorization") === "Bearer old")
          return new HttpResponse(null, { status: 401 });
        await new Promise((resolve) => setTimeout(resolve, 100));
        return HttpResponse.json({ ok: true });
      }),
    );
    await expect(api.get("slow", { timeout: 30 })).rejects.toMatchObject({
      name: "TimeoutError",
    });
  });

  it("returns forbidden responses without refreshing", async () => {
    const { api, refresh } = authenticatedApi();
    let requests = 0;
    server.use(
      http.get("https://storage.example.com/denied", () =>
        ++requests < 4
          ? new HttpResponse(null, { status: 403 })
          : HttpResponse.json({ ok: true }),
      ),
    );
    await expect(api.get("denied")).rejects.toMatchObject({
      response: { status: 403 },
    });
    expect(requests).toBe(1);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("stops after one refresh when a new token is still unauthorized", async () => {
    const { api, refresh } = authenticatedApi();
    let requests = 0;
    server.use(
      http.get("https://storage.example.com/denied", () =>
        ++requests < 4
          ? new HttpResponse(null, { status: 401 })
          : HttpResponse.json({ ok: true }),
      ),
    );
    await expect(api.get("denied")).rejects.toMatchObject({
      response: { status: 401 },
    });
    expect(requests).toBe(2);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("retries successfully with a refreshed token and preserves the request body", async () => {
    const { api, refresh } = authenticatedApi();
    server.use(
      http.post("https://storage.example.com/update", async ({ request }) => {
        if (request.headers.get("Authorization") === "Bearer old")
          return new HttpResponse(null, { status: 401 });
        return HttpResponse.json(await request.json());
      }),
    );
    expect(
      await api.post("update", { json: { value: "data" } }).json(),
    ).toEqual({ value: "data" });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("preserves throwHttpErrors false on the authenticated retry", async () => {
    const { api, refresh } = authenticatedApi();
    let requests = 0;
    server.use(
      http.get("https://storage.example.com/denied", () =>
        ++requests < 4
          ? new HttpResponse(null, { status: 401 })
          : HttpResponse.json({ ok: true }),
      ),
    );
    expect((await api.get("denied", { throwHttpErrors: false })).status).toBe(
      401,
    );
    expect(requests).toBe(2);
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});

describe("create", () => {
  it("honors client timeout, headers, and response error options", async () => {
    server.use(
      http.get("https://storage.example.com/options", async ({ request }) => {
        expect(request.headers.get("X-Custom")).toBe("configured");
        await new Promise((resolve) => setTimeout(resolve, 50));
        return new HttpResponse(null, { status: 409 });
      }),
    );
    const api = create("https://storage.example.com", {
      timeout: false,
      throwHttpErrors: false,
      headers: { "X-Custom": "configured" },
    });
    expect((await api.get("options")).status).toBe(409);
  });

  it("attaches Authorization header from provided accessToken", async () => {
    const api = create("https://storage.example.com", {
      accessToken: "valid-token",
    });
    const response = await api.get("v1alpha/test");
    expect(response.status).toBe(200);
  });

  it("logs to stderr when verbose is true", async () => {
    const writes: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      writes.push(String(msg));
      return true;
    });

    const api = create("https://storage.example.com", {
      accessToken: "valid-token",
      verbose: true,
    });
    await api.get("v1alpha/test");
    expect(writes.some((w) => w.includes("GET"))).toBe(true);
    vi.restoreAllMocks();
  });
});

describe("isHttpError", () => {
  it("identifies HTTPError", async () => {
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/test",
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    const api = create("https://storage.example.com");
    try {
      await api.get("v1alpha/test");
    } catch (e) {
      expect(isHttpError(e)).toBe(true);
    }
  });
});
