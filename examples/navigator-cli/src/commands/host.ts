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

import { readConfig, updateConfig, withHostAuthLock } from "@/config";
import { deviceFlow } from "@/services/auth";
import { doTokenRefresh, isHttpError } from "@/services/api";
import {
  discoveryService,
  type DiscoveryStackEntry,
} from "@/services/discoveryService";

export async function addHost(
  name: string,
  discoveryHostname: string,
): Promise<void> {
  await withHostAuthLock(name, async () => {
    const svc = discoveryService(discoveryHostname);

    // Fetch auth config first (public endpoint) — this gives us what we need to authenticate.
    const authConfig = await svc.getAuthConfig();
    const authUrl = authConfig?.openidConfiguration ?? "";

    // Run device flow before fetching services — the /services endpoint may require auth.
    let accessToken: string | undefined;
    let refreshToken: string | undefined;
    let tokenExpiry: string | undefined;

    if (authConfig) {
      const auth = await deviceFlow(authConfig);
      accessToken = auth.accessToken;
      refreshToken = auth.refreshToken;
      tokenExpiry = auth.expiresAt.toISOString();
    }

    // Fetch services with the access token so a 401-protected endpoint works.
    const storage = await svc.resolveApi("filefolder", accessToken);
    const storageUrl = storage?.baseUrl;
    if (!storageUrl) {
      throw new Error("No storage service found at this discovery host.");
    }

    await updateConfig((config) => {
      config.hosts[name] = {
        discoveryUrl: svc.url,
        storageUrl,
        authUrl,
        accessToken,
        refreshToken,
        tokenExpiry,
      };
      config.activeHost = name;
    });
  });
}

export async function useHost(name: string): Promise<void> {
  await updateConfig((config) => {
    if (!config.hosts[name]) {
      throw new Error(`host '${name}' not found. Run 'sn host add'`);
    }
    config.activeHost = name;
  });
}

export async function removeHost(name: string): Promise<void> {
  await updateConfig((config) => {
    if (!config.hosts[name]) {
      throw new Error(`host '${name}' not found.`);
    }
    delete config.hosts[name];
    if (config.activeHost === name) {
      delete config.activeHost;
    }
  });
}

export interface HostListEntry {
  name: string;
  discoveryUrl: string;
  active: boolean;
}

export function listHosts(): HostListEntry[] {
  const config = readConfig();
  return Object.entries(config.hosts).map(([name, host]) => ({
    name,
    discoveryUrl: host.discoveryUrl,
    active: config.activeHost === name,
  }));
}

export async function listServices(
  hostName?: string,
): Promise<DiscoveryStackEntry[]> {
  const config = readConfig();
  const name = hostName ?? config.activeHost;
  if (!name) {
    throw new Error("No active host. Run 'sn host use <name>'.");
  }
  const hostCfg = config.hosts[name];
  if (!hostCfg) {
    throw new Error(`host '${name}' not found. Run 'sn host list'`);
  }
  const svc = discoveryService(hostCfg.discoveryUrl);
  try {
    return await svc.getAllServices(hostCfg.accessToken);
  } catch (e) {
    if (isHttpError(e) && e.response.status === 401) {
      await doTokenRefresh(name, hostCfg.discoveryUrl);
      const refreshed = readConfig().hosts[name];
      return svc.getAllServices(refreshed?.accessToken);
    }
    throw e;
  }
}
