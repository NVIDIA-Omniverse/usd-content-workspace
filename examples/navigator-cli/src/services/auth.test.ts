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

import * as sleepUtil from "@/util/sleep";
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
import {
  AuthExpiredError,
  deviceFlow,
  refreshAuth,
  setDeviceFlowAllowed,
} from "./auth";
import type { DiscoveryAuthConfig } from "./discoveryService";

const mockConfig: DiscoveryAuthConfig = {
  openidConfiguration: "https://idp/.well-known/openid-configuration",
  clients: { navigator: { clientId: "nav-id", scope: "openid" } },
};

const mockOpenIdConfig = {
  authorization_endpoint: "https://idp/auth",
  token_endpoint: "https://idp/token",
  device_authorization_endpoint: "https://idp/device_authorization",
};

const server = setupServer(
  http.get("https://idp/.well-known/openid-configuration", () =>
    HttpResponse.json(mockOpenIdConfig),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  setDeviceFlowAllowed(true);
});

describe("refreshAuth", () => {
  it("returns new tokens on success", async () => {
    server.use(
      http.post("https://idp/token", () =>
        HttpResponse.json({
          access_token: "new-access",
          id_token: "new-id",
          refresh_token: "new-refresh",
          expires_in: 3600,
        }),
      ),
    );
    const result = await refreshAuth("old-refresh", mockConfig);
    expect(result.accessToken).toBe("new-access");
    expect(result.refreshToken).toBe("new-refresh");
    expect(result.expiresAt).toBeInstanceOf(Date);
  });

  it("throws AuthExpiredError on 401", async () => {
    server.use(
      http.post(
        "https://idp/token",
        () => new HttpResponse(null, { status: 401 }),
      ),
    );
    await expect(refreshAuth("bad-refresh", mockConfig)).rejects.toThrow(
      AuthExpiredError,
    );
  });

  it("throws AuthExpiredError on 400 invalid_grant", async () => {
    server.use(
      http.post("https://idp/token", () =>
        HttpResponse.json({ error: "invalid_grant" }, { status: 400 }),
      ),
    );
    await expect(refreshAuth("expired-refresh", mockConfig)).rejects.toThrow(
      AuthExpiredError,
    );
  });

  it("throws plain Error on 400 with other error codes", async () => {
    server.use(
      http.post("https://idp/token", () =>
        HttpResponse.json({ error: "invalid_client" }, { status: 400 }),
      ),
    );
    await expect(refreshAuth("bad-refresh", mockConfig)).rejects.toThrow(
      "HTTP400 invalid_client",
    );
    await expect(refreshAuth("bad-refresh", mockConfig)).rejects.not.toThrow(
      AuthExpiredError,
    );
  });
});

describe("deviceFlow", () => {
  it("increases every subsequent polling interval by five seconds on slow_down", async () => {
    const intervals: number[] = [];
    vi.spyOn(sleepUtil, "sleep").mockImplementation((ms) => {
      intervals.push(ms);
      return Promise.resolve();
    });
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    let polls = 0;
    server.use(
      http.post("https://idp/device_authorization", () =>
        HttpResponse.json({
          device_code: "code",
          user_code: "user",
          verification_uri: "https://idp/activate",
          expires_in: 300,
          interval: 1,
        }),
      ),
      http.post("https://idp/token", () =>
        ++polls < 3
          ? HttpResponse.json({ error: "slow_down" }, { status: 400 })
          : HttpResponse.json({
              access_token: "access",
              id_token: "id",
              expires_in: 3600,
            }),
      ),
    );
    expect((await deviceFlow(mockConfig)).accessToken).toBe("access");
    expect(intervals).toEqual([1000, 6000, 11000]);
  });

  it("fails immediately when interactive login is disabled", async () => {
    setDeviceFlowAllowed(false);
    await expect(deviceFlow(mockConfig)).rejects.toMatchObject({
      code: "AUTH_REQUIRED",
    });
  });
  it("polls until authorized and returns tokens", async () => {
    server.use(
      http.post("https://idp/device_authorization", () =>
        HttpResponse.json({
          device_code: "dev-code",
          user_code: "USER-CODE",
          verification_uri: "https://idp/activate",
          expires_in: 300,
          interval: 1,
        }),
      ),
      http.post("https://idp/token", () =>
        HttpResponse.json({
          access_token: "access",
          id_token: "id",
          refresh_token: "refresh",
          expires_in: 3600,
        }),
      ),
    );

    // suppress console.error output during test
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    const result = await deviceFlow(mockConfig);
    expect(result.accessToken).toBe("access");
  });
});
