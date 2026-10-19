/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import type { QueryClient } from "@tanstack/react-query";
import type { User } from "oidc-client-ts";
import z from "zod";
import type { Config } from "../context/ConfigContext";
import { fetchAllUserGroups } from "./users";

export interface DiagnosticsResourceRow {
  type: string;
  id: string;
}

export interface DiagnosticsBatchFormValues {
  principal: string;
  service: string;
  action: string;
  context: string;
  resources: DiagnosticsResourceRow[];
}

export interface FetchDiagnosticsBatchParams {
  values: DiagnosticsBatchFormValues;
  config: Config;
  user: User;
  queryClient: QueryClient;
}

export async function fetchDiagnosticsBatch({
  values,
  config,
  user,
  queryClient,
}: FetchDiagnosticsBatchParams): Promise<DiagnosticsBatchOutcome> {
  const url = new URL(
    "/v1beta/diagnostics/authorize/batch/",
    config.endpoints.backend,
  );

  const principal = values.principal.trim();
  const service = values.service.trim();
  const action = values.action.trim();
  const context = values.context.trim();
  const parsedContext = context ? JSON.parse(context) : undefined;

  const rows = values.resources
    .map((row) => ({ type: row.type.trim(), id: row.id.trim() }))
    .filter((row) => row.id);

  const principalResult = principal
    ? await buildPrincipalEntity({ principal, config, user, queryClient })
    : undefined;
  const principalEntity = principalResult?.principal;
  const groupsResolutionFailed = principalResult?.groupsResolutionFailed ?? false;
  const actions = [{ service, name: action }];

  const batches =
    rows.length > 0
      ? rows.map((row) => ({
          principal: principalEntity,
          actions,
          resource: { type: row.type, id: row.id },
          context: parsedContext,
        }))
      : [
          {
            principal: principalEntity,
            actions,
            resource: undefined,
            context: parsedContext,
          },
        ];

  const response = await fetch(url, {
    method: "POST",
    body: JSON.stringify({ batches }),
    headers: {
      Authorization: `Bearer ${user.access_token}`,
      "Content-Type": "application/json",
    },
  });

  if (response.ok) {
    const json = await response.json();
    return { ...DiagnosticsBatchResponse.parse(json), groupsResolutionFailed };
  }

  const text = await response.text();
  throw new Error(
    `Failed to run diagnostics: HTTP${response.status} -- ${text}`,
  );
}

interface DiagnosticsPrincipal {
  sub: string;
  info?: { groups: string[] };
}

interface BuildPrincipalEntityResult {
  principal: DiagnosticsPrincipal;
  groupsResolutionFailed: boolean;
}

interface BuildPrincipalEntityParams {
  principal: string;
  config: Config;
  user: User;
  queryClient: QueryClient;
}

async function buildPrincipalEntity({
  principal,
  config,
  user,
  queryClient,
}: BuildPrincipalEntityParams): Promise<BuildPrincipalEntityResult> {
  if (!config.endpoints.userService) {
    return { principal: { sub: principal }, groupsResolutionFailed: false };
  }

  try {
    const groups = await queryClient.fetchQuery({
      queryKey: ["user-groups-all", principal],
      queryFn: () => fetchAllUserGroups({ userId: principal, config, user }),
      staleTime: 3 * 60 * 1000,
      retry: false,
    });
    return {
      principal: {
        sub: principal,
        info: { groups: groups.map((group) => group.id) },
      },
      groupsResolutionFailed: false,
    };
  } catch (error) {
    console.warn(`Failed to resolve groups for principal ${principal}`, error);
    return { principal: { sub: principal }, groupsResolutionFailed: true };
  }
}

export function expandResourceHierarchy(
  url: string,
  type: string,
  parentTypes: Map<string, string>,
): DiagnosticsResourceRow[] {
  const ancestors = expandResourceAncestors(url);
  const rows: DiagnosticsResourceRow[] = [];
  let currentType = type.trim();
  for (const id of ancestors) {
    currentType = parentTypes.get(currentType) ?? currentType;
    rows.push({ type: currentType, id });
  }
  return rows;
}

export function expandResourceAncestors(url: string): string[] {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return [];
  }

  const base = `${parsed.protocol}//${parsed.host}/`;
  const segments = parsed.pathname.split("/").filter(Boolean);
  if (segments.length === 0) {
    return [];
  }

  const ancestors: string[] = [];
  for (let i = segments.length - 1; i >= 1; i--) {
    ancestors.push(`${base}${segments.slice(0, i).join("/")}/`);
  }
  ancestors.push(base);
  return ancestors;
}

const DiagnosticsPolicy = z.object({
  id: z.number().int(),
  order: z.number().int(),
  policy: z.string(),
  principal: z
    .object({ sub: z.string(), info: z.any().nullable() })
    .nullable()
    .optional(),
  group: z.object({ id: z.string() }).nullable().optional(),
  action: z
    .object({ name: z.string(), service: z.string() })
    .nullable()
    .optional(),
  resource: z
    .object({ id: z.string(), type: z.string(), data: z.any().nullable() })
    .nullable()
    .optional(),
});

export type DiagnosticsPolicy = z.infer<typeof DiagnosticsPolicy>;

const DiagnosticsActionPolicies = z.object({
  service: z.string(),
  action: z.string(),
  policies: z.array(DiagnosticsPolicy),
});

export type DiagnosticsActionPolicies = z.infer<
  typeof DiagnosticsActionPolicies
>;

const DiagnosticsBatchResult = z.object({
  resource: z
    .object({
      id: z.string(),
      type: z.string(),
      data: z.any().nullable().optional(),
    })
    .nullable()
    .optional(),
  evaluation_priority: z.enum(["forbid", "permit"]),
  actions: z.array(DiagnosticsActionPolicies),
});

export type DiagnosticsBatchResult = z.infer<typeof DiagnosticsBatchResult>;

const DiagnosticsBatchResponse = z.object({
  results: z.array(DiagnosticsBatchResult),
});

export type DiagnosticsBatchResponse = z.infer<typeof DiagnosticsBatchResponse>;

export type DiagnosticsBatchOutcome = DiagnosticsBatchResponse & {
  groupsResolutionFailed: boolean;
};
