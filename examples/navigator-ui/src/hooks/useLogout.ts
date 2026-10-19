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

import { useCallback } from "react";
import { useMutation } from "@tanstack/react-query";
import { getAuthConfig, logout } from "@/services/auth";
import { useAppStore } from "@/state/appStore";

/**
 * Returns a function that logs the user out of the connection identified by `endpoint`.
 *
 * A single sign-in backs every connection that shares the same discovery URL, so the
 * stored tokens are cleared for all of them before the IdP logout redirect. When the
 * deployment runs without authentication the page is simply reloaded.
 */
export function useLogout() {
  const connections = useAppStore((state) => state.connections);
  const setConnectionAuth = useAppStore((state) => state.setConnectionAuth);
  const clearDiscoveryCache = useAppStore((state) => state.clearDiscoveryCache);

  const mutation = useMutation({
    mutationFn: async ({ discovery, idToken }: { discovery: string; idToken?: string }) => {
      try {
        const config = await getAuthConfig(discovery);
        if (config) {
          await logout(config, idToken);
        } else {
          window.location.reload();
        }
      } catch {
        // Tokens are already cleared, so reload to drop session state and re-trigger auth
        // even when discovery or the IdP logout redirect fails.
        window.location.reload();
      }
    },
  });

  return useCallback(
    async (endpoint: string) => {
      const connection = connections[endpoint];
      if (!connection) {
        return;
      }

      const { discovery, auth } = connection.info;
      const idToken = auth?.idToken;

      for (const [key, candidate] of Object.entries(connections)) {
        if (candidate.info.discovery === discovery) {
          setConnectionAuth(key, null);
        }
      }

      clearDiscoveryCache();

      await mutation.mutateAsync({ discovery, idToken });
    },
    [connections, mutation, setConnectionAuth, clearDiscoveryCache]
  );
}
