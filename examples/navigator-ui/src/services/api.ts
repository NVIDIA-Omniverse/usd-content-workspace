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

import { minutesToMilliseconds } from "date-fns";
import ky, { HTTPError, Options } from "ky";
import { Connection, useAppStore } from "@/state/appStore";
import { Authentication, authorize, getAuthConfig, refreshAuth } from "./auth";

// extends the ky Options interface with auth options
// used for getting auth info and setting auth info on token refresh
export interface ApiOptions extends Omit<Options, "baseUrl"> {
  discovery: string;
  // timeout in milliseconds applied to every request made by this instance
  requestTimeout?: number;
  auth: {
    // returns the current auth info for the active connection
    get: () => Authentication | null;
    // updates the auth info for the active connection
    set: (auth: Authentication | null) => void;
  };
}

export type ApiInstance = ReturnType<typeof create>;

export function create(serviceUrl: string, options?: ApiOptions) {
  const instance = ky.create({
    prefixUrl: serviceUrl,
    retry: 0,
    timeout: options?.requestTimeout,
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    hooks: {
      beforeRequest: [
        async (request) => {
          if (!options?.auth) {
            return;
          }
          let auth = options.auth.get();
          if (!auth) {
            return;
          }
          // Refresh the token proactively if it expires within the threshold
          if (auth.refreshToken && auth.expiresAt && shouldRefreshToken(auth.expiresAt)) {
            await refreshAuthentication(auth, options.discovery, options.auth);
          } else {
            await waitForSessionRefresh(auth);
          }

          // Read auth again in case it was updated
          auth = options.auth.get();
          if (auth?.accessToken) {
            request.headers.set("Authorization", `Bearer ${auth.accessToken}`);
          }
        },
      ],
      afterResponse: [
        async (request, _, response) => {
          if (!options?.auth) {
            return;
          }
          const key = request.url;

          if (![401, 403].includes(response.status)) {
            authRetryCountMap.delete(key);
            return;
          }

          // retry on 401 or 403 should not be needed but is a workaround for these issues:
          // 401 - Storage may return 401 for a request with a valid token
          // 403 - Storage may return 403 if Envoy cannot connect to the authentication extension
          const retryCount = authRetryCountMap.get(key) ?? 0;

          if (retryCount >= maxAuthRetries) {
            authRetryCountMap.delete(key);
            return;
          }

          const auth = options.auth.get();
          // auth can be null if user closed the browser during authorization flow
          if (!auth || !auth.refreshToken) {
            await startAuthorizationFlow(options.discovery);
            return;
          }

          authRetryCountMap.set(key, retryCount + 1);

          if (response.status === 403) {
            return instance(request);
          }

          await refreshAuthentication(auth, options.discovery, options.auth);
          const refreshedAuth = options.auth.get();
          if (refreshedAuth?.accessToken) {
            request.headers.set("Authorization", `Bearer ${refreshedAuth.accessToken}`);
            return instance(request);
          }
        },
      ],
    },
    ...options,
  });

  return {
    serviceUrl,
    head: (path: string, options?: Options) => {
      return instance.head(path, options);
    },
    get: <T>(path: string, options?: Options) => {
      return instance.get<T>(path, options);
    },
    post: <T>(path: string, options?: Options) => {
      return instance.post<T>(path, options);
    },
    put: <T>(path: string, options?: Options) => {
      return instance.put<T>(path, options);
    },
    delete: <T>(path: string, options?: Options) => {
      return instance.delete<T>(path, options);
    },
    instance,
  };
}

// runtime config threaded into the api instance built by getApiOptions
export interface ApiOptionsConfig {
  // request timeout in milliseconds from the app config
  requestTimeout: number;
}

/**
 * creates the auth options used for all api requests requiring authentication
 *
 * @param connection - Connection - the action connection
 * @param config - runtime config from the app config, e.g. request timeout
 * @returns ApiOptions
 */
export function getApiOptions(connection: Connection, { requestTimeout }: ApiOptionsConfig): ApiOptions {
  return {
    discovery: connection.info.discovery,
    requestTimeout,
    auth: {
      get: () => {
        return useAppStore.getState().connections[connection.endpoint]?.info.auth;
      },
      set: (auth: Authentication | null) => {
        const state = useAppStore.getState();
        state.setConnectionAuth(connection.endpoint, auth);
      },
    },
  };
}

export const isHttpError = (error: unknown): error is HTTPError => (error as Error | undefined)?.name === "HTTPError";

function waitForSessionRefresh(auth: Authentication | null): Promise<void> {
  if (!auth || !auth.refreshToken) {
    return Promise.resolve();
  }

  return refreshPromiseMap.get(auth.refreshToken) || Promise.resolve();
}

const refreshPromiseMap = new Map<string, Promise<void>>();

// tracks how many times we have retried a particular request URL due to a 401
const authRetryCountMap = new Map<string, number>();

const maxAuthRetries = 3;

// number of minutes before the token expires triggers a refresh
const refreshThresholdMs = minutesToMilliseconds(2);

function shouldRefreshToken(expiresAt: Date): boolean {
  return expiresAt.getTime() - Date.now() <= refreshThresholdMs;
}

async function startAuthorizationFlow(discovery: string) {
  const authConfig = await getAuthConfig(discovery);
  if (authConfig) {
    await authorize(
      {
        urls: {
          discovery,
        },
      },
      authConfig
    );
  }
}

function refreshAuthentication(
  auth: Authentication,
  discovery: string,
  authOptions: ApiOptions["auth"]
): Promise<void> {
  if (!auth.refreshToken) {
    return Promise.resolve();
  }

  let refreshing = refreshPromiseMap.get(auth.refreshToken);
  if (!refreshing) {
    refreshing = (async () => {
      const authConfig = await getAuthConfig(discovery);
      if (!authConfig) {
        // Authentication is disabled.
        authOptions.set(null);
        return;
      }

      try {
        const refreshedAuth = await refreshAuth(auth.refreshToken!, authConfig);
        authOptions.set(refreshedAuth);
      } catch (error) {
        // auth refresh failed. clear auth and re-authorize
        authOptions.set(null);
        await startAuthorizationFlow(discovery);
      }
    })();

    void refreshing.finally(() => {
      refreshPromiseMap.delete(auth.refreshToken!);
    });
    refreshPromiseMap.set(auth.refreshToken, refreshing);
  }

  return refreshing;
}
