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
import { useAppConfig } from "@/hooks/useAppConfig";
import { Authentication } from "@/services/auth";
import { useAppStore } from "@/state/appStore";

export function useAddConnection() {
  const addConnection = useAppStore((state) => state.addConnection);
  const setActiveConnection = useAppStore((state) => state.setActiveConnection);
  const syncDefaultConnections = useAppStore((state) => state.syncDefaultConnections);
  const { defaultDiscovery } = useAppConfig();

  return useCallback(
    (hostname: string, storageEndpoints: string[], auth: Authentication | null) => {
      if (storageEndpoints.length === 0) {
        throw new Error("This deployment does not have any Storage API available for Navigator.");
      }

      // In default-server mode the connection list is the single source of truth from the
      // Helm-configured discovery, so post-OIDC additions sync (drop strays + attach friendly
      // names) instead of merely appending.
      if (defaultDiscovery && hostname === defaultDiscovery) {
        syncDefaultConnections(hostname, storageEndpoints, auth);
        return;
      }

      for (const endpoint of storageEndpoints) {
        addConnection(endpoint, {
          discovery: hostname,
          auth,
        });
        setActiveConnection(endpoint);
      }
    },
    [addConnection, defaultDiscovery, setActiveConnection, syncDefaultConnections]
  );
}
