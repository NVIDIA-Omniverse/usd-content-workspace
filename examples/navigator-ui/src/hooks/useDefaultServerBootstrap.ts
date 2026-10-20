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

import { useQuery, UseQueryResult } from "@tanstack/react-query";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { ApiOptions, getApiOptions } from "@/services/api";
import { Authentication } from "@/services/auth";
import { useAppStore } from "@/state/appStore";

/**
 * Drives the default-server bootstrap: when `appConfig.defaultDiscovery` is set,
 * queries the discovery service for storage endpoints and replaces the connection
 * store with the result via `syncDefaultConnections`.
 *
 * Auth handling is delegated to `api.ts` — discovery is called with `ApiOptions`
 * so the existing ky `afterResponse` hook redirects to the IdP on 401 and refreshes
 * tokens proactively. Persisted auth for the same discovery is reused so reloads
 * don't always trigger a new auth round-trip.
 *
 * Consumers can read `isPending` to gate UI on bootstrap completion; the query is
 * disabled (and therefore inert) when no default discovery is configured.
 */
export function useDefaultServerBootstrap(): UseQueryResult<string[]> {
  const { defaultDiscovery, requestTimeout } = useAppConfig();
  const syncDefaultConnections = useAppStore((state) => state.syncDefaultConnections);
  const { getApi } = useServiceApi();

  return useQuery({
    queryKey: ["defaultServerBootstrap", defaultDiscovery],
    queryFn: async () => {
      const existing = Object.values(useAppStore.getState().connections).find(
        (connection) => connection.info.discovery === defaultDiscovery
      );

      let transientAuth: Authentication | null = null;
      const options: ApiOptions = existing
        ? getApiOptions(existing, { requestTimeout })
        : {
            discovery: defaultDiscovery,
            requestTimeout,
            auth: {
              get: () => transientAuth,
              set: (auth) => {
                transientAuth = auth;
              },
            },
          };

      const storage = await getApi(defaultDiscovery, "filefolder", options);
      const endpoints = storage ? [storage.baseUrl] : [];
      syncDefaultConnections(defaultDiscovery, endpoints, options.auth.get());
      return endpoints;
    },
    enabled: !!defaultDiscovery,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}
