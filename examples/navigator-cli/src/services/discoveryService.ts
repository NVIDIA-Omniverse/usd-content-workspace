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

import ky from "ky";
import { z } from "zod";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { join } from "@/util/url";

const DiscoveryAuthClientInfoSchema = fromSnakeCaseSchema(
  z.object({ clientId: z.string(), scope: z.string() }),
);
export type DiscoveryAuthClientInfo = z.infer<
  typeof DiscoveryAuthClientInfoSchema
>;

const DiscoveryAuthConfigSchema = fromSnakeCaseSchema(
  z.object({
    openidConfiguration: z.string(),
    clients: z.record(z.string(), DiscoveryAuthClientInfoSchema),
  }),
);
export type DiscoveryAuthConfig = z.infer<typeof DiscoveryAuthConfigSchema>;

const DiscoveryStackEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  type: z.string(),
  grpc: z.string().optional(),
  rest: z.string().optional(),
});
export type DiscoveryStackEntry = z.infer<typeof DiscoveryStackEntrySchema>;

const DiscoveryStackInfoSchema = z.object({
  services: z.array(DiscoveryStackEntrySchema),
});
export type DiscoveryStackInfo = z.infer<typeof DiscoveryStackInfoSchema>;

export type ServiceType =
  | "storage"
  | "notification-consumer"
  | "permission"
  | "thumbnail"
  | "userinfo"
  | "archive";

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

export interface DiscoveredApi {
  baseUrl: string;
  versionPath: string;
  version: string;
  url: string;
}

const DiscoveryV2InfoSchema = z.object({
  "schema-version": z.literal(2),
  services: z.array(
    z.object({
      name: z.string(),
      rest: z
        .object({
          url: z.string(),
          endpoints: z.array(
            z.object({
              name: z.string(),
              versions: z.record(z.string(), z.object({ path: z.string() })),
            }),
          ),
        })
        .optional(),
    }),
  ),
});

type DiscoveryV2Info = z.infer<typeof DiscoveryV2InfoSchema>;
type DiscoveryDocument =
  | { schemaVersion: 2; services: DiscoveryV2Info }
  | { schemaVersion: 1; services: DiscoveryStackInfo };

const registry: Record<
  ApiName,
  { versions: string[]; v1Type: ServiceType; v1Path: string; v1Version: string }
> = {
  capabilities: {
    versions: ["v1alpha"],
    v1Type: "storage",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  filefolder: {
    versions: ["v1alpha"],
    v1Type: "storage",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  fileobject: {
    versions: ["v1alpha"],
    v1Type: "storage",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  metadata: {
    versions: ["v1beta", "v1alpha"],
    v1Type: "storage",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  versioning: {
    versions: ["v1alpha"],
    v1Type: "storage",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  "notifications-consumer": {
    versions: ["v1beta"],
    v1Type: "notification-consumer",
    v1Path: "/api/v1beta",
    v1Version: "v1beta",
  },
  permission: {
    versions: ["v1beta"],
    v1Type: "permission",
    v1Path: "/v1beta/",
    v1Version: "v1beta",
  },
  userinfo: {
    versions: ["v1alpha"],
    v1Type: "userinfo",
    v1Path: "/api/v1alpha/",
    v1Version: "v1alpha",
  },
  thumbnail: {
    versions: ["v1alpha"],
    v1Type: "thumbnail",
    v1Path: "/v1alpha",
    v1Version: "v1alpha",
  },
  archive: {
    versions: ["v1alpha"],
    v1Type: "archive",
    v1Path: "/v1alpha/",
    v1Version: "v1alpha",
  },
};

export function discoveryService(hostname: string) {
  const normalized = hostname.replace(/\/api\/v[12]\/?$/, "");
  const url = normalized.startsWith("http")
    ? join(normalized, "/api/v1", "/")
    : `https://${normalized}/api/v1/`;
  const v2Url = join(
    normalized.startsWith("http") ? normalized : `https://${normalized}`,
    "/api/v2",
    "/",
  );

  async function fetchDocument(
    accessToken?: string,
  ): Promise<DiscoveryDocument> {
    const headers: Record<string, string> = {};
    if (accessToken) headers.Authorization = `Bearer ${accessToken}`;
    try {
      const response = await ky.get(`${v2Url}services`, { headers });
      const parsed = DiscoveryV2InfoSchema.safeParse(await response.json());
      if (parsed.success) return { schemaVersion: 2, services: parsed.data };
    } catch {
      // Older deployments and invalid v2 documents fall back to the v1 listing.
    }
    const response = await ky.get(`${url}services`, { headers });
    return {
      schemaVersion: 1,
      services: DiscoveryStackInfoSchema.parse(await response.json()),
    };
  }

  async function resolveApi(
    name: ApiName,
    accessToken?: string,
  ): Promise<DiscoveredApi | null> {
    const document = await fetchDocument(accessToken);
    const entry = registry[name];
    if (document.schemaVersion === 1) {
      const service = document.services.services.find(
        (item) => item.type === entry.v1Type && item.rest,
      );
      return service?.rest
        ? {
            baseUrl: service.rest,
            versionPath: entry.v1Path,
            version: entry.v1Version,
            url: join(service.rest, entry.v1Path),
          }
        : null;
    }
    for (const version of entry.versions) {
      for (const service of document.services.services) {
        const endpoint = service.rest?.endpoints.find(
          (candidate) => candidate.name === name,
        );
        const versionPath = endpoint?.versions[version]?.path;
        if (service.rest && versionPath) {
          return {
            baseUrl: service.rest.url,
            versionPath,
            version,
            url: join(service.rest.url, versionPath),
          };
        }
      }
    }
    return null;
  }

  async function resolveApis(
    accessToken?: string,
  ): Promise<Partial<Record<ApiName, DiscoveredApi>>> {
    const document = await fetchDocument(accessToken);
    const result: Partial<Record<ApiName, DiscoveredApi>> = {};
    for (const name of Object.keys(registry) as ApiName[]) {
      const entry = registry[name];
      if (document.schemaVersion === 1) {
        const service = document.services.services.find(
          (item) => item.type === entry.v1Type && item.rest,
        );
        if (service?.rest)
          result[name] = {
            baseUrl: service.rest,
            versionPath: entry.v1Path,
            version: entry.v1Version,
            url: join(service.rest, entry.v1Path),
          };
      } else {
        for (const version of entry.versions) {
          const service = document.services.services.find((item) =>
            item.rest?.endpoints.some(
              (endpoint) =>
                endpoint.name === name && endpoint.versions[version],
            ),
          );
          const versionPath = service?.rest?.endpoints.find(
            (endpoint) => endpoint.name === name,
          )?.versions[version]?.path;
          if (service?.rest && versionPath) {
            result[name] = {
              baseUrl: service.rest.url,
              versionPath,
              version,
              url: join(service.rest.url, versionPath),
            };
            break;
          }
        }
      }
    }
    return result;
  }

  return {
    url,
    resolveApi,
    resolveApis,
    async getAuthConfig(): Promise<DiscoveryAuthConfig | null> {
      try {
        const response = await ky.get(`${url}auth-config`);
        return DiscoveryAuthConfigSchema.parse(await response.json());
      } catch (error) {
        if (
          (error as { response?: { status: number } }).response?.status === 404
        ) {
          return null;
        }
        throw error;
      }
    },
    async getStackEntry(
      type: ServiceType,
      accessToken?: string,
    ): Promise<DiscoveryStackEntry[]> {
      const services = await this.getAllServices(accessToken);
      return services.filter((s) => s.type === type && !!s.rest);
    },
    async getAllServices(accessToken?: string): Promise<DiscoveryStackEntry[]> {
      const document = await fetchDocument(accessToken);
      if (document.schemaVersion === 1) return document.services.services;
      return document.services.services.flatMap((service) =>
        (service.rest?.endpoints ?? []).flatMap((endpoint) =>
          Object.entries(endpoint.versions).map(([version, info]) => ({
            id: `${service.name}:${endpoint.name}:${version}`,
            name: `${endpoint.name} (${version})`,
            type: endpoint.name,
            rest: join(service.rest!.url, info.path),
          })),
        ),
      );
    },
  };
}
