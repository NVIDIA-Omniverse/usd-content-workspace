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
import ky, { HTTPError, type Options, type KyInstance } from "ky";
import { readConfig, updateConfig, withHostAuthLock } from "@/config";
import { AuthExpiredError, deviceFlow, refreshAuth } from "./auth";
import { discoveryService } from "./discoveryService";

export interface CliApiOptions extends Omit<Options, "prefixUrl"> {
  /** Name of the host entry in config (for reading/writing tokens during refresh). */
  hostName?: string;
  /** Discovery URL for the host (needed to look up auth config during refresh). */
  discovery?: string;
  /** Access token to attach directly (skips config lookup — used for testing). */
  accessToken?: string;
  /** Print each request/response to stderr. */
  verbose?: boolean;
  /** Resolve a legacy service route to its discovered endpoint. */
  route?: (url: string) => string | null | Promise<string | null>;
}

export type ApiInstance = ReturnType<typeof create>;

const refreshThresholdMs = minutesToMilliseconds(2);
const tokenRefreshes = new Map<string, Promise<void>>();

function shouldRefreshToken(expiresAt: Date): boolean {
  return expiresAt.getTime() - Date.now() <= refreshThresholdMs;
}

export const isHttpError = (error: unknown): error is HTTPError =>
  error instanceof HTTPError;

export async function doTokenRefresh(
  hostName: string,
  discovery: string,
  expectedToken = readConfig().hosts[hostName]?.accessToken,
): Promise<void> {
  const key = JSON.stringify([hostName, discovery]);
  const existing = tokenRefreshes.get(key);
  if (existing) return existing;
  const refresh = withHostAuthLock(hostName, async () => {
    const host = readConfig().hosts[hostName];
    if (!host || host.discoveryUrl !== discovery) {
      throw new Error(`Host '${hostName}' changed during authentication.`);
    }
    // Another request/process may already have refreshed this exact token.
    if (
      host.accessToken &&
      host.accessToken !== expectedToken &&
      (!host.tokenExpiry || new Date(host.tokenExpiry).getTime() > Date.now())
    )
      return;
    const authConfig = await discoveryService(discovery).getAuthConfig();
    if (!authConfig) return;
    let auth;
    let preserveRefreshToken = false;
    if (host.refreshToken) {
      try {
        auth = await refreshAuth(host.refreshToken, authConfig);
        preserveRefreshToken = true;
      } catch (error) {
        if (!(error instanceof AuthExpiredError)) throw error;
        auth = await deviceFlow(authConfig);
      }
    } else {
      auth = await deviceFlow(authConfig);
    }
    await updateConfig((config) => {
      const current = config.hosts[hostName];
      if (!current || current.discoveryUrl !== discovery) {
        throw new Error(`Host '${hostName}' changed during authentication.`);
      }
      current.accessToken = auth.accessToken;
      current.refreshToken =
        auth.refreshToken ??
        (preserveRefreshToken ? host.refreshToken : undefined);
      current.tokenExpiry = auth.expiresAt.toISOString();
    });
  });
  tokenRefreshes.set(key, refresh);
  try {
    await refresh;
  } finally {
    if (tokenRefreshes.get(key) === refresh) tokenRefreshes.delete(key);
  }
}

export function create(serviceUrl: string, options?: CliApiOptions) {
  let requestIndex = 0;
  const requestIndexMap = new WeakMap<Request, number>();

  const httpOptions = { ...options };
  for (const key of [
    "hostName",
    "discovery",
    "accessToken",
    "verbose",
    "route",
  ] as const)
    delete httpOptions[key];
  const instance: KyInstance = ky
    .create({
      prefixUrl: serviceUrl,
      retry: 0,
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      hooks: {
        beforeRequest: [
          async (request, requestOptions) => {
            if (options?.verbose) {
              const idx = ++requestIndex;
              requestIndexMap.set(request, idx);
              process.stderr.write(
                `${new Date().toISOString()} [${idx}] → ${request.method} ${request.url}\n`,
              );
            }

            // Direct token provided (e.g. tests) — use it as-is
            if (options?.accessToken) {
              request.headers.set(
                "Authorization",
                `Bearer ${options.accessToken}`,
              );
            } else if (options?.hostName && options.discovery) {
              const config = readConfig();
              const host = config.hosts[options.hostName];
              if (host?.accessToken) {
                // Proactive refresh
                if (
                  !requestOptions.context.snAuthRetried &&
                  host.tokenExpiry &&
                  shouldRefreshToken(new Date(host.tokenExpiry))
                ) {
                  await doTokenRefresh(
                    options.hostName,
                    options.discovery,
                    host.accessToken,
                  );
                }
                const refreshed = readConfig().hosts[options.hostName];
                if (refreshed?.accessToken) {
                  request.headers.set(
                    "Authorization",
                    `Bearer ${refreshed.accessToken}`,
                  );
                }
              }
            }

            const routedUrl = await options?.route?.(request.url);
            if (routedUrl && routedUrl !== request.url) {
              const routed = new Request(routedUrl, request);
              const index = requestIndexMap.get(request);
              if (index !== undefined) requestIndexMap.set(routed, index);
              return routed;
            }
          },
        ],
        afterResponse: [
          async (request, requestOptions, response) => {
            if (options?.verbose) {
              const idx = requestIndexMap.get(request) ?? "?";
              process.stderr.write(
                `${new Date().toISOString()} [${idx}]   ${response.status}\n`,
              );
            }

            if (
              response.status !== 401 ||
              requestOptions.context.snAuthRetried
            ) {
              return;
            }
            if (!options?.hostName || !options?.discovery) {
              return;
            }

            const tokenBefore = request.headers
              .get("Authorization")
              ?.replace(/^Bearer /, "");
            await doTokenRefresh(
              options.hostName,
              options.discovery,
              tokenBefore,
            );
            const refreshed = readConfig().hosts[options.hostName];
            if (
              refreshed?.accessToken &&
              refreshed.accessToken !== tokenBefore
            ) {
              request.headers.set(
                "Authorization",
                `Bearer ${refreshed.accessToken}`,
              );
              return instance(request, {
                context: { ...requestOptions.context, snAuthRetried: true },
                // The original call applies its error policy to this response.
                throwHttpErrors: false,
                signal: requestOptions.signal,
                // Ky retains timeout at runtime but excludes it from NormalizedOptions.
                timeout: (requestOptions as Options).timeout,
              });
            }
          },
        ],
      },
    })
    .extend(httpOptions);

  return {
    serviceUrl,
    head: (path: string, opts?: Options) => instance.head(path, opts),
    get: <T>(path: string, opts?: Options) => instance.get<T>(path, opts),
    post: <T>(path: string, opts?: Options) => instance.post<T>(path, opts),
    put: <T>(path: string, opts?: Options) => instance.put<T>(path, opts),
    delete: <T>(path: string, opts?: Options) => instance.delete<T>(path, opts),
    instance,
  };
}
