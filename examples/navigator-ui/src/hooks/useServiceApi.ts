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

import { useMemo } from "react";
import { ApiInstance, ApiOptions, create } from "@/services/api";
import {
  DiscoveryStackInfo,
  DiscoveryV2Info,
  fetchDiscoveryDocument,
  resolveDiscoveryUrls,
} from "@/services/discoveryService";
import { useAppStore } from "@/state/appStore";
import { join } from "@/util/url";

export interface ServiceApi {
  /**
   * Resolves a storage API through discovery, returning its base and versioned URLs,
   * or null when the deployment does not advertise it.
   */
  getApi: (discovery: string, name: ApiName, options?: ApiOptions) => Promise<DiscoveredApi | null>;

  /**
   * Resolves a storage API through discovery and builds an ApiInstance against its full URL,
   * or null when the deployment does not advertise it.
   */
  createServiceApi: (discovery: string, name: ApiName, options?: ApiOptions) => Promise<VersionedApiInstance | null>;
}

// The store-bound `createServiceApi`, passed to imperative (non-hook) helpers so they can resolve
// storage APIs without accessing the discovery cache directly.
export type CreateServiceApi = ServiceApi["createServiceApi"];

/**
 * Hook that exposes service discovery bound to the per-session discovery cache in the store.
 *
 * The discovery document is fetched once per deployment and cached in the store, so resolution
 * closes over the store resolver here rather than reaching into global state from a service module.
 * The v2 probe wins when the deployment advertises a valid v2 document; otherwise the v1 stack
 * listing is used as a fallback.
 */
export function useServiceApi(): ServiceApi {
  const resolve = useAppStore((state) => state.resolveDiscoveryDocument);

  return useMemo(() => {
    async function getApi(discovery: string, name: ApiName, options?: ApiOptions): Promise<DiscoveredApi | null> {
      const { v1Url, v2Url } = resolveDiscoveryUrls(discovery);
      const document = await resolve(v1Url, () =>
        fetchDiscoveryDocument(create(v1Url, options), create(v2Url, options))
      );

      const registry = API_REGISTRY[name];
      if (document.schemaVersion === 2) {
        return findV2Api(document.services, name, registry.versions);
      }
      return findV1Api(document.services, registry);
    }

    async function createServiceApi(
      discovery: string,
      name: ApiName,
      options?: ApiOptions
    ): Promise<VersionedApiInstance | null> {
      const api = await getApi(discovery, name, options);
      return api ? { ...create(api.url, options), apiVersion: api.version } : null;
    }

    return { getApi, createServiceApi };
  }, [resolve]);
}

// Resolves an API from the v2 document, honouring the registry's version preference order: the
// first version the deployment advertises wins. A deployment that only advertises the older
// version therefore keeps working, and one that dual-serves is used through the newer API.
function findV2Api(document: DiscoveryV2Info, name: ApiName, versions: readonly string[]): DiscoveredApi | null {
  for (const version of versions) {
    for (const service of document.services) {
      const endpoint = service.rest?.endpoints.find((candidate) => candidate.name === name);
      const versionPath = endpoint?.versions[version]?.path;
      if (service.rest && versionPath) {
        return { baseUrl: service.rest.url, versionPath, version, url: join(service.rest.url, versionPath) };
      }
    }
  }
  return null;
}

function findV1Api(document: DiscoveryStackInfo, registry: ApiRegistryEntry): DiscoveredApi | null {
  const entry = document.services.find((service) => service.type === registry.v1Type && !!service.rest);
  if (!entry?.rest) {
    return null;
  }
  return {
    baseUrl: entry.rest,
    versionPath: registry.v1Path,
    version: registry.v1Version,
    url: join(entry.rest, registry.v1Path),
  };
}

// Result of resolving a single API through discovery.
// `url` is the full base to create an ApiInstance against (baseUrl + versionPath),
// `baseUrl` is passed to clients that append the version path themselves.
export interface DiscoveredApi {
  url: string;
  baseUrl: string;
  versionPath: string;
  // API version this instance was resolved for, e.g. "v1alpha". Clients that speak more than one
  // version of the same API (see `metadataService`) branch on it.
  version: string;
}

// An ApiInstance tagged with the API version discovery resolved it to.
export type VersionedApiInstance = ApiInstance & { apiVersion: string };

export type ApiName =
  | "capabilities"
  | "filefolder"
  | "fileobject"
  | "metadata"
  | "versioning"
  | "notifications-consumer"
  | "permission"
  | "userinfo"
  | "thumbnail"
  | "archive";

interface ApiRegistryEntry {
  // Accepted v2 versions in preference order; the first one the deployment advertises is used.
  versions: readonly string[];
  v1Type: string;
  v1Path: string;
  // Version the hardcoded v1 fallback path corresponds to.
  v1Version: string;
}

// Single source of truth for how each API is found:
// - v2: matched by endpoint `name` inside a service's rest endpoints, then by the first
//   advertised version from `versions`
// - v1: matched by service `type`, with a hardcoded path appended to the base URL
const API_REGISTRY: Record<ApiName, ApiRegistryEntry> = {
  capabilities: { versions: ["v1alpha"], v1Type: "storage", v1Path: "/v1alpha", v1Version: "v1alpha" },
  filefolder: { versions: ["v1alpha"], v1Type: "storage", v1Path: "/v1alpha", v1Version: "v1alpha" },
  fileobject: { versions: ["v1alpha"], v1Type: "storage", v1Path: "/v1alpha", v1Version: "v1alpha" },
  // The storage service dual-serves metadata: the top-level v1beta API is preferred and the
  // v1alpha API remains the fallback for deployments that do not advertise it yet.
  metadata: { versions: ["v1beta", "v1alpha"], v1Type: "storage", v1Path: "/v1alpha", v1Version: "v1alpha" },
  versioning: { versions: ["v1alpha"], v1Type: "storage", v1Path: "/v1alpha", v1Version: "v1alpha" },
  "notifications-consumer": {
    versions: ["v1beta"],
    v1Type: "notification-consumer",
    v1Path: "/api/v1beta",
    v1Version: "v1beta",
  },
  permission: { versions: ["v1beta"], v1Type: "permission", v1Path: "/v1beta/", v1Version: "v1beta" },
  userinfo: { versions: ["v1alpha"], v1Type: "userinfo", v1Path: "/api/v1alpha/", v1Version: "v1alpha" },
  thumbnail: { versions: ["v1alpha"], v1Type: "thumbnail", v1Path: "/v1alpha", v1Version: "v1alpha" },
  archive: { versions: ["v1alpha"], v1Type: "archive", v1Path: "/v1alpha/", v1Version: "v1alpha" },
};
