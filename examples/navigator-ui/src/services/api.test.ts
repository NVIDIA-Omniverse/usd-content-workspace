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

import { waitFor } from "@testing-library/react";
import { HTTPError } from "ky";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { useAppStore } from "@/state/appStore";
import { ApiOptions, create, getApiOptions, isHttpError } from "./api";
import { Authentication } from "./auth";

describe("api", () => {
  const discovery = "discovery";
  const openidConfigurationUrl = "http://openidConfiguration";
  const authHeaderUrl = "http://auth";

  interface AuthHeaderTestResponse {
    token: string | null;
  }

  const server = setupServer(
    http.get(`https://${discovery}/api/v1/auth-config`, () => {
      return HttpResponse.json({ openid_configuration: openidConfigurationUrl, clients: {} });
    }),
    http.get(authHeaderUrl, ({ request }) => {
      return HttpResponse.json({ token: request.headers.get("Authorization") });
    })
  );

  beforeAll(() => {
    server.listen();
  });

  afterAll(() => {
    server.close();
  });

  afterEach(() => {
    server.resetHandlers();
    vi.resetAllMocks();
  });

  const mocks = vi.hoisted(() => {
    return {
      authorize: vi.fn(),
      refreshAuth: vi.fn(),
      getAuthConfig: vi.fn(),
    };
  });

  vi.mock("@/services/auth", () => {
    return {
      authorize: mocks.authorize,
      refreshAuth: mocks.refreshAuth,
      getAuthConfig: mocks.getAuthConfig,
    };
  });

  beforeEach(() => {
    mocks.getAuthConfig.mockResolvedValue({ openidConfiguration: "", clients: {} });
  });

  it("no authorization header when api instance has null auth info", async () => {
    const instance = create(authHeaderUrl);
    const response = await instance.get<AuthHeaderTestResponse>("");
    expect((await response.json()).token).toBeNull();
  });

  it("sets authorization header when api instance has auth info", async () => {
    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => ({
          accessToken: "accessToken",
          refreshToken: "refreshToken",
          expiresAt: new Date(2030, 1),
          idToken: "idToken",
        }),
        set: () => undefined,
      },
    };
    const instance = create(authHeaderUrl, apiOptions);
    const response = await instance.get<AuthHeaderTestResponse>("");
    expect((await response.json()).token).toBe(`Bearer ${apiOptions.auth.get()!.accessToken}`);
  });

  it("calls refreshAuth once for multiple requests", async () => {
    const authHeaderRefreshUrl = "http://authRefresh";

    server.use(
      http.get(authHeaderRefreshUrl, ({ request }) => {
        if (request.headers.get("Authorization") === "Bearer accessToken2") {
          return HttpResponse.json({}, { status: 200 });
        }
        return HttpResponse.json({}, { status: 401 });
      })
    );

    mocks.refreshAuth.mockImplementation(() => {
      return {
        accessToken: "accessToken2",
        refreshToken: "refreshToken",
        expiresAt: new Date(2030, 1),
        idToken: "idToken2",
      };
    });

    let auth: Authentication | null = {
      accessToken: "accessToken",
      refreshToken: "refreshToken",
      expiresAt: new Date(2025, 1),
      idToken: "idToken",
    };

    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => {
          return { ...auth } as Authentication;
        },
        set: (a: Authentication | null) => {
          auth = a;
        },
      },
    };

    const instance = create(authHeaderRefreshUrl, apiOptions);

    await Promise.all([instance.get(""), instance.get(""), instance.get(""), instance.get("")]);
    expect(mocks.refreshAuth).toHaveBeenCalledOnce();
  });

  it("calls authorize when token refresh fails", async () => {
    const authUrl = "http://authRefresh";

    server.use(
      http.get(authUrl, () => {
        return HttpResponse.json({}, { status: 401 });
      })
    );

    mocks.refreshAuth.mockImplementation(() => {
      throw new Error("refresh failed");
    });

    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => ({
          accessToken: "accessToken",
          refreshToken: "refreshToken",
          expiresAt: new Date(2030, 1),
          idToken: "idToken",
        }),
        set: vi.fn(),
      },
    };

    const instance = create(authUrl, apiOptions);
    try {
      await instance.get("");
    } catch (e) {
      expect(e).toBeInstanceOf(HTTPError);
    }

    expect(mocks.authorize).toHaveBeenCalledTimes(3);
    expect(apiOptions.auth.set).toHaveBeenCalledTimes(3);
  });

  it("retries the request three times after an initial 403 response", async () => {
    const retryUrl = "http://authretry-count";

    let requestCount = 0;

    server.use(
      http.get(retryUrl, () => {
        requestCount += 1;
        return HttpResponse.json({}, { status: 403 });
      })
    );

    mocks.refreshAuth.mockImplementation(() => {
      throw new Error("refreshAuth should not be called for 403 retries");
    });

    let auth: Authentication | null = {
      accessToken: "accessToken0",
      refreshToken: "refreshToken",
      expiresAt: new Date(2030, 1),
      idToken: "idToken0",
    };

    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => ({ ...auth }) as Authentication,
        set: (a: Authentication | null) => {
          auth = a;
        },
      },
    };

    const instance = create(retryUrl, apiOptions);

    await expect(instance.get("")).rejects.toBeInstanceOf(HTTPError);

    await waitFor(() => {
      expect(requestCount).toBe(4); // initial request + 3 retries
      expect(mocks.refreshAuth).not.toHaveBeenCalled();
    });
  });

  it("refreshes the token in the afterResponse handler", async () => {
    const endpoint = "http://afterResponse-refresh";
    let requestCount = 0;

    server.use(
      http.get(endpoint, ({ request }) => {
        requestCount += 1;
        if (request.headers.get("Authorization") === "Bearer accessToken2") {
          return HttpResponse.json({ success: true }, { status: 200 });
        }
        return HttpResponse.json({}, { status: 401 });
      })
    );

    mocks.refreshAuth.mockImplementation(() => {
      return {
        accessToken: "accessToken2",
        refreshToken: "refreshToken",
        expiresAt: new Date(2030, 1),
        idToken: "idToken2",
      } as Authentication;
    });

    let auth: Authentication | null = {
      accessToken: "accessToken",
      refreshToken: "refreshToken",
      expiresAt: new Date(2030, 1),
      idToken: "idToken",
    };

    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => ({ ...auth }) as Authentication,
        set: (a: Authentication | null) => {
          auth = a;
        },
      },
    };

    const instance = create(endpoint, apiOptions);

    const response = await instance.get("");
    expect(response.status).toBe(200);

    await waitFor(() => {
      expect(requestCount).toBe(2); // initial request + retry after refresh
      expect(mocks.refreshAuth).toHaveBeenCalledTimes(1);
      expect(auth?.accessToken).toBe("accessToken2");
    });
  });

  it("should not refresh the token if auth config is not available", async () => {
    const endpoint = "http://test-service/";

    server.use(
      http.get(endpoint, () => {
        return new HttpResponse(null, { status: 403 });
      }),
      http.get(`https://${discovery}/api/v1/auth-config`, () => {
        return new HttpResponse(null, { status: 404 });
      })
    );

    let auth: Authentication | null = {
      accessToken: "accessToken",
      refreshToken: "refreshToken",
      expiresAt: new Date(2030, 1),
      idToken: "idToken",
    };

    const apiOptions: ApiOptions = {
      discovery,
      auth: {
        get: () => auth,
        set: (a: Authentication | null) => {
          auth = a;
        },
      },
    };

    const instance = create(endpoint, apiOptions);
    await expect(instance.get("")).rejects.toThrow(/403 Forbidden/);
    expect(mocks.authorize).not.toBeCalled();
  });

  it("getApiOptions gets and sets correct auth info", () => {
    const activeEndpoint = "http://active";
    const state = useAppStore.getState();
    const activeConnection = state.addConnection(activeEndpoint, {
      auth: {
        accessToken: "token",
        refreshToken: "refresh",
        expiresAt: new Date(2025, 1),
        idToken: "id",
      },
      discovery,
    });
    state.setActiveConnection(activeEndpoint);

    const apiOptions = getApiOptions(activeConnection, { requestTimeout: 30000 });
    expect(apiOptions.auth.get()).toEqual(activeConnection.info.auth);
    expect(apiOptions.discovery).toEqual(discovery);
    expect(apiOptions.requestTimeout).toEqual(30000);
    const newAuth = {
      accessToken: "token2",
      refreshToken: "refresh2",
      expiresAt: new Date(2030, 1),
      idToken: "idToken",
    };
    apiOptions.auth.set(newAuth);
    expect(apiOptions.auth.get()).toEqual(newAuth);
  });

  describe("isHttpError", () => {
    it("recognizes a ky HTTP error", () => {
      const error = new HTTPError(new Response(null, { status: 404 }), new Request(authHeaderUrl), {} as never);
      expect(isHttpError(error)).toBe(true);
    });

    it.each([null, undefined, "boom", new Error("boom")])("returns false for %s", (error) => {
      expect(isHttpError(error)).toBe(false);
    });
  });
});
