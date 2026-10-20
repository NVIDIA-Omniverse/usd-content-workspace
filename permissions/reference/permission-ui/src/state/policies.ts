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
import type { User } from "oidc-client-ts";
import z from "zod";
import type { Config } from "../context/ConfigContext";
import type { PolicyFormValues } from "../pages/PolicyForm";

export interface FetchPoliciesParams {
  config: Config;
  user: User;
  search?: {
    principal?: string;
    group?: string;
    action?: string;
    resource?: string;
    order?: string;
  };
  page?: number;
  limit?: number;
}

export async function fetchPolicies({
  config,
  user,
  search = {},
  page = 1,
  limit = 50,
}: FetchPoliciesParams): Promise<PolicyPage> {
  const url = new URL("/v1beta/policies/", config.endpoints.backend);
  if (search?.principal) {
    url.searchParams.set("principal", search?.principal);
  }
  if (search?.group) {
    url.searchParams.set("group", search.group);
  }
  if (search?.action) {
    url.searchParams.set("action", search.action);
  }
  if (search?.resource) {
    url.searchParams.set("resource", search?.resource);
  }
  if (search?.order) {
    url.searchParams.set("order", search.order);
  }
  url.searchParams.set("page", page.toString());
  url.searchParams.set("limit", limit.toString());

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return PolicyPage.parse(json);
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch policies: HTTP${response.status} -- ${text}`,
  );
}

export interface FetchPolicyParams {
  policyId: string;
  config: Config;
  user: User;
}

export async function fetchPolicy({
  policyId,
  config,
  user,
}: FetchPolicyParams): Promise<Policy> {
  const url = new URL(`/v1beta/policies/${policyId}`, config.endpoints.backend);
  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return Policy.parse(json);
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch the policy: HTTP${response.status} -- ${text}`,
  );
}

export function getPolicyEffect(policy: Policy): "permit" | "forbid" {
  return policy.policy.trim().startsWith("permit") ? "permit" : "forbid";
}

export interface PutPolicyParams {
  policy: string;
  values: PolicyFormValues;
  config: Config;
  user: User;
}

export async function putPolicy({
  policy,
  values,
  config,
  user,
}: PutPolicyParams): Promise<Policy> {
  const isGroup = values.principalType === "group";
  const hasResource = !!values.resourceType && !!values.resourceId;
  const isResourcePrefix = values.resourceOperator === "in";
  const resource = hasResource
    ? { id: values.resourceId, type: values.resourceType }
    : undefined;
  const url = new URL(`/v1beta/policies/`, config.endpoints.backend);
  const response = await fetch(url, {
    method: "PUT",
    body: JSON.stringify({
      principal: !isGroup && values.principal ? values.principal : undefined,
      group: isGroup && values.principal ? values.principal : undefined,
      action:
        values.service && values.action
          ? {
              name: values.action,
              service: values.service,
            }
          : undefined,
      resource: isResourcePrefix ? undefined : resource,
      resource_prefix: isResourcePrefix ? resource : undefined,
      policy,
      order: values.order !== "" ? values.order : undefined,
      comment: values.comment ? values.comment : undefined,
    }),
    headers: {
      Authorization: `Bearer ${user.access_token}`,
      "Content-Type": "application/json",
    },
  });

  if (response.ok) {
    const json = await response.json();
    return Policy.parse(json);
  }

  let text = await response.text();
  if (text.startsWith("Failed to parse the Cedar policy")) {
    text += ".\nCheck if specified conditions have valid Cedar syntax.";
  }

  throw new Error(
    `Failed to create a policy: HTTP${response.status} -- ${text}`,
  );
}

export interface DeletePolicyParams {
  policyId: string | number;
  config: Config;
  user: User;
}

export async function deletePolicy({
  policyId,
  config,
  user,
}: DeletePolicyParams): Promise<void> {
  const url = new URL(`/v1beta/policies/${policyId}`, config.endpoints.backend);
  const response = await fetch(url, {
    method: "DELETE",
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    return;
  }

  const text = await response.text();
  throw new Error(
    `Failed to delete a policy: HTTP${response.status} -- ${text}`,
  );
}

const Principal = z.object({
  sub: z.string(),
  info: z.any().nullable(),
});

const Group = z.object({
  id: z.string(),
});

const Action = z.object({
  name: z.string(),
  service: z.string(),
});

const Resource = z.object({
  id: z.string(),
  type: z.string(),
  data: z.any().nullable(),
});

const Policy = z.object({
  id: z.number().int(),
  policy: z.string(),
  principal: Principal.nullable(),
  group: Group.nullable().optional(),
  action: Action.nullable(),
  resource: Resource.nullable(),
  resource_prefix: Resource.nullable().optional(),
  order: z.number().int().nullable().optional(),
  created_at: z.coerce.date().nullable().optional(),
  created_by: z.string().nullable().optional(),
  comment: z.string().nullable().optional(),
});

export type Policy = z.infer<typeof Policy>;

const PolicyPage = z.object({
  items: z.array(Policy),
  page: z.number().nullable(),
  page_size: z.number(),
});

export type PolicyPage = z.infer<typeof PolicyPage>;
