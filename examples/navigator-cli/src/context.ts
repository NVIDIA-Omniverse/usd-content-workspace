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

import { getActiveHost, readConfig } from "@/config";
import { CliError } from "@/output";
import {
  create,
  doTokenRefresh,
  isHttpError,
  type ApiInstance,
  type CliApiOptions,
} from "@/services/api";
import {
  discoveryService,
  type ApiName,
  type DiscoveredApi,
} from "@/services/discoveryService";

const serviceResolvers = new WeakMap<
  ApiInstance,
  () => Promise<Partial<Record<ApiName, DiscoveredApi>>>
>();

export async function getDiscoveredApi(
  api: ApiInstance,
  name: ApiName,
): Promise<DiscoveredApi | null> {
  return (await serviceResolvers.get(api)?.())?.[name] ?? null;
}

function serviceForLegacyPath(path: string): ApiName | null {
  const match =
    /^v1alpha\/(capabilities|filefolder|fileobject|metadata|versioning|thumbnails|tasks)(?:\/|\?|$)/.exec(
      path,
    );
  if (match)
    return match[1] === "thumbnails" || match[1] === "tasks"
      ? "thumbnail"
      : (match[1] as ApiName);
  if (path.startsWith("api/v1beta/events/")) return "notifications-consumer";
  return null;
}

export interface CommandContext {
  api: ApiInstance;
  storageUrl: string;
  hostName: string;
  discoveryUrl: string;
  cwd: string | undefined;
  verbose: boolean;
  json: boolean;
}

export function buildContext(opts: {
  verbose?: boolean;
  json?: boolean;
  host?: string;
}): CommandContext {
  const config = readConfig();
  const hostName = opts.host ?? config.activeHost ?? "";
  const host = opts.host ? config.hosts[opts.host] : getActiveHost(config);
  if (!host)
    throw new CliError(
      `host '${hostName}' not found. Run 'sn host list'`,
      "HOST_NOT_FOUND",
    );

  const svc = discoveryService(host.discoveryUrl);
  let discovered: Promise<Partial<Record<ApiName, DiscoveredApi>>> | undefined;
  const resolveServices = () => {
    if (discovered) return discovered;
    const token = readConfig().hosts[hostName]?.accessToken;
    return (discovered = svc
      .resolveApis(token)
      .catch(async (error: unknown) => {
        if (isHttpError(error) && error.response.status === 401) {
          await doTokenRefresh(hostName, host.discoveryUrl, token);
          return svc.resolveApis(readConfig().hosts[hostName]?.accessToken);
        }
        throw error;
      }));
  };

  const storagePrefix = host.storageUrl.replace(/\/$/, "") + "/";
  const apiOptions: CliApiOptions = {
    hostName,
    discovery: host.discoveryUrl,
    verbose: opts.verbose,
    route: async (url) => {
      if (!url.startsWith(storagePrefix)) return null;
      const legacyPath = url.slice(storagePrefix.length);
      const name = serviceForLegacyPath(legacyPath);
      if (!name) return null;
      const service = (await resolveServices())[name];
      if (!service)
        throw new Error(`${name} API is not available on this host`);
      const relativePath = legacyPath
        .replace(/^v1alpha\//, "")
        .replace(/^api\/v1beta\//, "");
      return `${service.url.replace(/\/$/, "")}/${relativePath}`;
    },
  };

  const api = create(host.storageUrl, apiOptions);
  serviceResolvers.set(api, resolveServices);

  return {
    api,
    storageUrl: host.storageUrl,
    hostName,
    discoveryUrl: host.discoveryUrl,
    cwd: host.cwd,
    verbose: opts.verbose ?? false,
    json: opts.json ?? false,
  };
}
