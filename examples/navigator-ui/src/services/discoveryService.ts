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

import { z } from "zod";
import { logger } from "@/util/logger";
import { join } from "@/util/url";
import { ApiInstance, isHttpError } from "./api";

export interface DiscoveryUrls {
  base: string;
  v1Url: string;
  v2Url: string;
}

// Builds the discovery base and versioned listing URLs from a hostname or full URL.
export function resolveDiscoveryUrls(hostname: string): DiscoveryUrls {
  const base = hostname.startsWith("http") ? hostname : `https://${hostname}`;
  return {
    base,
    v1Url: join(base, "/api/v1", "/"),
    v2Url: join(base, "/api/v2", "/"),
  };
}

/**
 * Fetches the discovery document from a deployment. Probes v2 first and falls back to the v1 stack
 * listing whenever the v2 probe fails for any reason. Consumers build the v1/v2 clients (see
 * `useServiceApi`) so this module only performs the request and validates the response.
 */
export async function fetchDiscoveryDocument(v1Api: ApiInstance, v2Api: ApiInstance): Promise<DiscoveryDocument> {
  const v2Document = await probeV2Document(v2Api);
  if (v2Document) {
    return { schemaVersion: 2, services: v2Document };
  }

  const response = await v1Api.get("services");
  const v1Document = DiscoveryStackInfoSchema.parse(await response.json());
  return { schemaVersion: 1, services: v1Document };
}

// Probes the v2 discovery endpoint. Returns null on any failure - the endpoint being unsupported
// (404), a transport or authentication error, a non-JSON body (e.g. an SPA HTML fallback), or a
// body that does not match the v2 schema - so the caller falls back to v1. The reason is logged
// because a failed probe is indistinguishable from a v1-only deployment otherwise.
async function probeV2Document(v2Api: ApiInstance): Promise<DiscoveryV2Info | null> {
  try {
    const response = await v2Api.get("services");
    const parsed = DiscoveryV2InfoSchema.safeParse(await response.json());
    if (!parsed.success) {
      logV2ProbeFailure(v2Api.serviceUrl, parsed.error);
      return null;
    }
    return parsed.data;
  } catch (error) {
    logV2ProbeFailure(v2Api.serviceUrl, error);
    return null;
  }
}

function logV2ProbeFailure(v2Url: string, error: unknown) {
  const reason = describeV2ProbeError(error);
  logger.warn(`Discovery v2 is not used: the probe of ${v2Url}services failed (${reason}), falling back to v1.`);
}

function describeV2ProbeError(error: unknown): string {
  if (isHttpError(error)) {
    return `HTTP ${error.response.status} ${error.response.statusText}`;
  }
  if (error instanceof z.ZodError) {
    const issues = error.issues.map((issue) => `${issue.path.join(".") || "<root>"}: ${issue.message}`);
    return `the response does not match the v2 schema - ${issues.join("; ")}`;
  }
  if (error instanceof Error) {
    return `${error.name}: ${error.message}`;
  }
  return String(error);
}

const DiscoveryStackEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  type: z.string(),
  grpc: z.string().optional(),
  rest: z.string().optional(),
});

export const DiscoveryStackInfoSchema = z.object({
  services: z.array(DiscoveryStackEntrySchema),
});

export type DiscoveryStackInfo = z.infer<typeof DiscoveryStackInfoSchema>;

const DiscoveryV2EndpointSchema = z.object({
  name: z.string(),
  versions: z.record(z.string(), z.object({ path: z.string() })),
});

const DiscoveryV2ServiceSchema = z.object({
  name: z.string(),
  rest: z
    .object({
      url: z.string(),
      endpoints: z.array(DiscoveryV2EndpointSchema),
    })
    .optional(),
});

export const DiscoveryV2InfoSchema = z.object({
  "schema-version": z.literal(2),
  services: z.array(DiscoveryV2ServiceSchema),
});

export type DiscoveryV2Info = z.infer<typeof DiscoveryV2InfoSchema>;

// Normalized discovery document cached per connection. The v2 probe wins when the deployment
// advertises a valid v2 document; otherwise the v1 stack listing is used as a fallback.
export type DiscoveryDocument =
  | { schemaVersion: 2; services: DiscoveryV2Info }
  | { schemaVersion: 1; services: DiscoveryStackInfo };
