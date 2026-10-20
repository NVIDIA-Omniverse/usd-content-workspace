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
import { afterEach } from "vitest";
import {
  AuthenticationState,
  AuthExpiredError,
  authorize,
  CLIENT_NAME,
  DiscoveryAuthConfig,
  encodeState,
  exchangeCode,
  getAuthConfig,
  getUserEmail,
  logout,
  OpenIdConfiguration,
  refreshAuth,
  TokenResponse,
} from "@/services/auth";
import { mockWindowLocation, restoreWindowLocation } from "@/test-utils/location";

describe("Authentication", () => {
  const openidConfiguration = "http://idp/.well-known/openid-configuration";
  const clientId = "test-client-id";
  const scope = "openid email profile";

  const config: DiscoveryAuthConfig = {
    openidConfiguration,
    clients: {
      [CLIENT_NAME]: {
        clientId,
        scope,
      },
    },
  };

  const state: AuthenticationState = {
    urls: {
      discovery: "http://discovery",
    },
  };

  const server = setupServer(
    http.get(openidConfiguration, () => {
      return HttpResponse.json<OpenIdConfiguration>({
        authorization_endpoint: "http://idp/authorize",
        token_endpoint: "http://idp/token",
        userinfo_endpoint: "http://idp/userinfo",
        scopes_supported: ["openid", "email", "profile"],
      });
    })
  );

  beforeAll(() => {
    server.listen();
  });

  afterAll(() => {
    server.close();
  });

  beforeEach(() => {
    const now = new Date();
    vi.setSystemTime(now);

    mockWindowLocation();
  });

  afterEach(() => {
    server.resetHandlers();
    vi.useRealTimers();

    restoreWindowLocation();
  });

  it("authorize", async () => {
    await authorize(state, config);

    const url = new URL(window.location.href);
    expect(url.origin).toBe("http://idp");
    expect(url.pathname).toBe("/authorize");
    expect(url.searchParams.get("response_type")).toBe("code");
    expect(url.searchParams.get("client_id")).toBe(clientId);
    expect(url.searchParams.get("scope")).toBe(scope);
    expect(url.searchParams.get("redirect_uri")).toBe("http://localhost:5437/openid");
    expect(url.searchParams.get("nonce")).toBeTruthy();
    expect(url.searchParams.get("state")).toBe(encodeState(state));
    expect(url.searchParams.get("code_challenge")).toBeTruthy();
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
  });

  it("authorize can use the default client from the discovery auth-config", async () => {
    const config: DiscoveryAuthConfig = {
      openidConfiguration,
      clients: {
        default: {
          clientId,
          scope,
        },
      },
    };
    await authorize(state, config);

    const url = new URL(window.location.href);
    expect(url.origin).toBe("http://idp");
    expect(url.pathname).toBe("/authorize");
    expect(url.searchParams.get("client_id")).toBe(clientId);
    expect(url.searchParams.get("scope")).toBe(scope);
  });

  it("authorize throws an error if discovery does not include client information", async () => {
    const config: DiscoveryAuthConfig = {
      openidConfiguration,
      clients: {},
    };
    await expect(authorize(state, config)).rejects.toThrow(
      `Client "${CLIENT_NAME}" or the default client is not found in Discovery API.`
    );
  });

  it("logout redirects to the end session endpoint", async () => {
    server.use(
      http.get(openidConfiguration, () => {
        return HttpResponse.json<OpenIdConfiguration>({
          authorization_endpoint: "http://idp/authorize",
          token_endpoint: "http://idp/token",
          userinfo_endpoint: "http://idp/userinfo",
          scopes_supported: ["openid", "email", "profile"],
          end_session_endpoint: "http://idp/logout",
        });
      })
    );

    await logout(config, "test-id-token");

    const url = new URL(window.location.href);
    expect(url.origin).toBe("http://idp");
    expect(url.pathname).toBe("/logout");
    expect(url.searchParams.get("id_token_hint")).toBe("test-id-token");
    expect(url.searchParams.get("post_logout_redirect_uri")).toBe("http://localhost:5437/");
    expect(url.searchParams.get("client_id")).toBe(clientId);
  });

  it("logout returns to the app root when the IdP has no end session endpoint", async () => {
    await logout(config, "test-id-token");
    expect(window.location.href).toBe("http://localhost:5437/");
  });

  it("exchange code", async () => {
    server.use(
      http.post("http://idp/token", () => {
        return HttpResponse.json<TokenResponse>({
          access_token: "test-access-token",
          id_token: "test-id-token",
          expires_in: 1800,
        });
      })
    );

    await authorize(state, config);

    const authentication = await exchangeCode("test-code", config);
    expect(authentication.accessToken).toBe("test-access-token");
    expect(authentication.idToken).toBe("test-id-token");
  });

  it("exchange code throws an error if codeVerifier is not generated", async () => {
    await expect(exchangeCode("test-code", config)).rejects.toThrow(
      "Login results are expired. Please try to authenticate again."
    );
  });

  it("exchange code throws an error if token_endpoint returns an error", async () => {
    const wwwAuthenticate =
      'Bearer realm="login.idp.com",error="invalid_token",error_description="invalid token format"';
    server.use(
      http.post("http://idp/token", () => {
        return new HttpResponse("", {
          status: 401,
          headers: {
            "WWW-Authenticate": wwwAuthenticate,
          },
        });
      })
    );

    await authorize(state, config);
    await expect(exchangeCode("test-code", config)).rejects.toThrow(
      `Failed to authenticate: HTTP401. ${wwwAuthenticate}`
    );
  });

  it("refreshes tokens", async () => {
    const refreshToken = "test-refresh-token";

    server.use(
      http.post("http://idp/token", async ({ request }) => {
        const body = await request.text();
        const params = new URLSearchParams(body);
        if (params.get("refresh_token") !== refreshToken) {
          return new HttpResponse("Invalid refresh_token.", { status: 401 });
        }

        return HttpResponse.json<TokenResponse>({
          access_token: "test-access-token",
          id_token: "test-id-token",
          expires_in: 1800,
          refresh_token: "test-refresh-token-2",
        });
      })
    );

    const auth = await refreshAuth(refreshToken, config);
    expect(auth.refreshToken).toBe("test-refresh-token-2");
  });

  it("refresh token raises auth expired error", async () => {
    const wwwAuthenticate = 'Bearer realm="login.idp.com",error="invalid_token",error_description="invalid token"';

    server.use(
      http.post("http://idp/token", () => {
        return new HttpResponse("Token expired.", {
          status: 401,
          headers: {
            "WWW-Authenticate": wwwAuthenticate,
          },
        });
      })
    );

    await expect(refreshAuth("test-refresh-token", config)).rejects.toThrowError(
      new AuthExpiredError(`Failed to refresh the session: HTTP401. ${wwwAuthenticate}`)
    );
  });
});

describe("getUserEmail", () => {
  function createIdToken(claims: Record<string, unknown>): string {
    const payload = btoa(JSON.stringify(claims)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
    return `header.${payload}.signature`;
  }

  it("returns the email claim from a valid id token", () => {
    const idToken = createIdToken({ email: "user@nvidia.com", sub: "123" });
    expect(getUserEmail(idToken)).toBe("user@nvidia.com");
  });

  it("returns undefined when the email claim is missing", () => {
    const idToken = createIdToken({ sub: "123" });
    expect(getUserEmail(idToken)).toBeUndefined();
  });

  it("returns undefined for a malformed id token", () => {
    expect(getUserEmail("not-a-jwt")).toBeUndefined();
  });
});

describe("getAuthConfig", () => {
  const hostname = "discovery";

  const mockAuthConfig: DiscoveryAuthConfig = {
    openidConfiguration: "http://openid-config",
    clients: {
      [CLIENT_NAME]: {
        clientId: "test-client-id",
        scope: "openid",
      },
    },
  };

  const server = setupServer(
    http.get(`https://${hostname}/api/v1/auth-config`, () => {
      return HttpResponse.json(mockAuthConfig);
    })
  );

  beforeAll(() => server.listen());
  afterEach(() => server.resetHandlers());
  afterAll(() => server.close());

  it("returns the auth configuration", async () => {
    const result = await getAuthConfig(hostname);
    expect(result).toEqual(mockAuthConfig);
  });

  it("returns null when auth is disabled", async () => {
    server.use(
      http.get(`https://${hostname}/api/v1/auth-config`, () => {
        return new HttpResponse(null, { status: 404 });
      })
    );

    const result = await getAuthConfig(hostname);
    expect(result).toBeNull();
  });
});
