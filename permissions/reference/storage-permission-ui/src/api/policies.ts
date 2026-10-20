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

import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";
import { encodeResource, encodeResourceUrl } from "../utils/resources.js";
import type { Policy, PolicyPage } from "./schemas.js";
import { PolicyPageSchema } from "./schemas.js";

export interface Permissions {
  entries: PermissionEntries;
  page?: number;
  unsupported: Policy[];
}

export const ALL_USERS = "\0all_users";

export type PrincipalType = "user" | "group";

const GROUP_PREFIX = "\0group\0";

export function groupKey(id: string): string {
  return GROUP_PREFIX + id;
}

export function isGroupKey(key: string): boolean {
  return key.startsWith(GROUP_PREFIX);
}

export function groupIdFromKey(key: string): string {
  return key.slice(GROUP_PREFIX.length);
}

export interface PermissionEntries {
  [principal: string]: ResourceAccess;
}

export type ResourceAccess = {
  [action in ResourceActionName]?: Permission;
};
export type ResourceActionName = "read" | "write" | "edit";

interface ActionDefinition {
  service: string;
  name: string;
  label: string;
}

const ACTIONS: Record<ResourceActionName, ActionDefinition> = {
  read: { service: "storage-service", name: "read", label: "Read" },
  write: { service: "storage-service", name: "write", label: "Write" },
  edit: { service: "permissions", name: "edit", label: "ACL" },
};

export function actionLabel(action: ResourceActionName): string {
  return ACTIONS[action].label;
}

function actionFromPolicy(item: Policy): ResourceActionName | undefined {
  if (!item.action) {
    return undefined;
  }
  for (const [action, definition] of Object.entries(ACTIONS)) {
    if (
      definition.service === item.action.service &&
      definition.name === item.action.name
    ) {
      return action as ResourceActionName;
    }
  }
  return undefined;
}

export type PermissionEffect = "allow" | "deny" | "unset";

export interface Permission {
  effect: PermissionEffect;
  policies?: Policy[];
  readonly?: boolean;
}

export interface GetPermissionsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  continuation?: Permissions;
  resource: ResourceAddress;
}

export async function getPermissions(
  params: GetPermissionsParams,
): Promise<Permissions> {
  const encodedResource = encodeResource(params.resource);

  const url = new URL("/v1beta/policies/", params.config.url);
  url.searchParams.set("resource", encodedResource);
  url.searchParams.set("limit", "50");
  if (params.continuation?.page) {
    url.searchParams.set("page", (params.continuation.page + 1).toString());
  } else {
    url.searchParams.set("page", "1");
  }

  const headers = new Headers();
  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }

  const response = await fetch(url, {
    headers,
  });
  if (response.ok) {
    const json = await response.json();
    const page = PolicyPageSchema.parse(json);
    const relevant = {
      ...page,
      items: page.items.filter(isRelevantPolicy),
    };
    const permissions = parsePermissions(relevant);

    if (page.page_size === 50) {
      // Fetch other policies from the next page.
      return getPermissions({
        ...params,
        continuation: permissions,
      });
    }
    return permissions;
  }

  if (response.status === 401) {
    params.auth.onExpire?.();
  }

  const message = await response.text();
  throw new Error(
    `Failed to get permissions for ${encodedResource} -- HTTP${response.status}. ${message}`,
  );
}

export interface GlobalPolicies {
  policies: Policy[];
  unresolvable: Policy[];
}

export interface GetGlobalPoliciesParams {
  auth: PermissionAuth;
  config: PermissionConfig;
}

/**
 * Retrieves the policies that apply to every resource (the unscoped,
 * resource=NULL policies) across all pages. These cannot be managed from a
 * resource-scoped panel, so the caller can warn about them; the subset that also
 * cannot be reflected in the resolved permissions (conditional or non-read/write
 * policies) is reported separately.
 *
 * @param params.auth Authentication for the request.
 * @param params.config Service configuration providing the policy URL.
 * @returns All global policies and the subset that cannot be resolved.
 */
export async function getGlobalPolicies(
  params: GetGlobalPoliciesParams,
): Promise<GlobalPolicies> {
  const policies = await fetchResourcePolicies({
    auth: params.auth,
    config: params.config,
    resource: "NULL",
  });

  const result: GlobalPolicies = { policies, unresolvable: [] };
  for (const item of policies) {
    if (!isResolvablePolicy(item)) {
      result.unresolvable.push(item);
    }
  }

  return result;
}

export interface FetchResourcePoliciesParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: string;
  inherited?: boolean;
}

/**
 * Fetches every policy affecting a resource with a single, action-agnostic
 * request and keeps only those relevant to this UI: policies with an unscoped
 * action (which applies to all actions), a storage-service read/write action, or
 * the permissions:edit action. Policies for other services or actions are
 * discarded.
 *
 * @param params.auth Authentication for the request.
 * @param params.config Service configuration providing the policy URL.
 * @param params.resource Encoded resource string, or "NULL" for global policies.
 * @param params.inherited When true, includes policies inherited from ancestor
 *   folders (adds `scope=inherited`).
 * @returns The relevant policy items affecting the resource.
 */
export async function fetchResourcePolicies(
  params: FetchResourcePoliciesParams,
): Promise<Policy[]> {
  const items = await fetchPolicyItems(params);
  return items.filter(isRelevantPolicy);
}

/**
 * Reports whether a policy is relevant to this UI: its action is unscoped (and so
 * applies to everything), it belongs to the storage service, or it is the
 * permissions:edit action. Storage-service actions that this UI cannot represent
 * are still kept so they can be surfaced as unsupported; policies for other
 * services are discarded.
 *
 * @param item Policy to inspect.
 * @returns True when the policy targets storage or permissions:edit.
 */
function isRelevantPolicy(item: Policy): boolean {
  if (!item.action) {
    return true;
  }
  const { service, name } = item.action;
  if (service === ACTIONS.read.service) {
    return true;
  }
  return service === ACTIONS.edit.service && name === ACTIONS.edit.name;
}

export interface FetchPolicyItemsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: string;
  inherited?: boolean;
}

/**
 * Fetches every policy returned for a resource query, without filtering by
 * action, following pagination until the last page.
 *
 * @param params.auth Authentication for the request.
 * @param params.config Service configuration providing the policy URL.
 * @param params.resource Encoded resource string, or "NULL" for global policies.
 * @param params.inherited When true, includes policies inherited from ancestor
 *   folders (adds `scope=inherited`).
 * @returns All policy items across every page.
 */
export async function fetchPolicyItems(
  params: FetchPolicyItemsParams,
): Promise<Policy[]> {
  const headers = new Headers();
  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }

  const items: Policy[] = [];
  let page = 0;
  for (;;) {
    const url = new URL("/v1beta/policies/", params.config.url);
    url.searchParams.set("resource", params.resource);
    url.searchParams.set("limit", "50");
    url.searchParams.set("page", (page + 1).toString());
    if (params.inherited) {
      url.searchParams.set("scope", "inherited");
    }

    const response = await fetch(url, { headers });
    if (!response.ok) {
      if (response.status === 401) {
        params.auth.onExpire?.();
      }

      const message = await response.text();
      throw new Error(
        `Failed to get policies for ${params.resource} -- HTTP${response.status}. ${message}`,
      );
    }

    const json = await response.json();
    const parsed = PolicyPageSchema.parse(json);
    page = parsed.page;
    items.push(...parsed.items);
    if (parsed.page_size < 50) {
      return items;
    }
  }
}

const POLICY_REGEX =
  /(?<effect>permit|forbid)\s*\((?<scopes>[\s\S]*?)\)(\s*(?<clause>when|unless)\s*\{(?<conditions>[\s\S]*?)})?\s*;/;

function parsePermissions(
  response: PolicyPage,
  defaults?: Permissions,
): Permissions {
  const permissions: Permissions = defaults ?? {
    entries: {},
    page: response.page,
    unsupported: [],
  };
  permissions.page = response.page;

  for (const item of response.items) {
    const result = POLICY_REGEX.exec(item.policy);
    if (!result) {
      console.warn("Failed to parse policy: ", item.policy);
      permissions.unsupported.push(item);
      continue;
    }

    if (result.groups?.conditions || !item.action) {
      // Conditional and unscoped-action policies cannot be displayed.
      permissions.unsupported.push(item);
      continue;
    }

    const action = actionFromPolicy(item);
    if (!action) {
      permissions.unsupported.push(item);
      continue;
    }

    const key = item.group
      ? groupKey(item.group.id)
      : item.principal
        ? item.principal.sub
        : ALL_USERS;
    permissions.entries[key] ??= {};

    const effect = result.groups?.effect === "permit" ? "allow" : "deny";
    const manageable = isManageableResource(item, action);
    const current = permissions.entries[key][action];
    if (current) {
      current.policies!.push(item);
      if (!manageable) {
        current.readonly = true;
      }
    } else {
      permissions.entries[key][action] = {
        effect,
        policies: [item],
        ...(manageable ? {} : { readonly: true }),
      };
    }
  }

  const actions: ResourceActionName[] = ["read", "write", "edit"];
  for (const access of Object.values(permissions.entries)) {
    for (const action of actions) {
      const permission = access[action];
      if (permission?.policies?.length) {
        permission.effect = resolveEffect(permission.policies, action);
      }
      access[action] ??= { effect: "unset" };
    }
  }

  return permissions;
}

/**
 * Resolves the effective permission from the policies that target the same
 * principal and action, mirroring the resolved-permission rules: the policy with
 * the lowest order wins, and on an equal order the default evaluation priority
 * decides. Storage actions let a permit win over a forbid, while permissions:edit
 * follows the permission service and lets a forbid win over a permit.
 *
 * @param policies Policies competing for the same principal and action.
 * @param action Managed action the policies apply to.
 * @returns The winning effect.
 */
function resolveEffect(
  policies: Policy[],
  action: ResourceActionName,
): PermissionEffect {
  const preferred: PermissionEffect = action === "edit" ? "deny" : "allow";
  let bestOrder = Number.POSITIVE_INFINITY;
  let effect: PermissionEffect = "unset";
  for (const item of policies) {
    const itemEffect = policyEffect(item);
    if (!itemEffect) {
      continue;
    }
    const order = item.order ?? Number.MAX_SAFE_INTEGER;
    if (
      order < bestOrder ||
      (order === bestOrder && itemEffect === preferred && effect !== preferred)
    ) {
      bestOrder = order;
      effect = itemEffect;
    }
  }
  return effect;
}

/**
 * Reports whether a policy can be managed from the resource-scoped panel. System
 * policies (`created_by` is SYSTEM) are always read-only. Policies bound to a
 * concrete resource (an exact `resource == ...` match) are manageable.
 * Permissions:edit is scoped by containment (`resource in ...`) so it cascades to a
 * folder's descendants and comes back with a resource prefix instead; the endpoint
 * only returns policies bound to the queried resource, so such a policy is still
 * manageable. Containment policies for other actions remain read-only.
 *
 * @param item Policy to inspect.
 * @param action Managed action the policy maps to.
 * @returns True when the policy can be edited from this panel.
 */
function isManageableResource(
  item: Policy,
  action: ResourceActionName,
): boolean {
  if (item.created_by === "SYSTEM") {
    return false;
  }
  return !!item.resource || (action === "edit" && !!item.resource_prefix);
}

/**
 * Reports whether a policy can be reflected in the resolved permissions: it must
 * have no conditional clause and either an unscoped action (which applies to all
 * actions) or one of the managed actions (read/write or permissions:edit).
 * Unscoped resources and unscoped principals are allowed here because resolution
 * accounts for them.
 *
 * @param item Policy to inspect.
 * @returns True when the policy can contribute to resolved permissions.
 */
export function isResolvablePolicy(item: Policy): boolean {
  const result = POLICY_REGEX.exec(item.policy);
  if (!result || result.groups?.conditions) {
    return false;
  }
  if (!item.action) {
    return true;
  }
  return actionFromPolicy(item) !== undefined;
}

/**
 * Lists the managed actions a policy grants or denies: a specific managed action
 * maps to itself, an unscoped action expands to all of them, and any other
 * specific action maps to nothing.
 *
 * @param item Policy to inspect.
 * @returns The affected managed actions.
 */
export function resolvableActions(item: Policy): ResourceActionName[] {
  if (!item.action) {
    return ["read", "write", "edit"];
  }
  const action = actionFromPolicy(item);
  return action ? [action] : [];
}

/**
 * Extracts the effect a policy expresses from its Cedar text.
 *
 * @param item Policy to inspect.
 * @returns "allow" for permit, "deny" for forbid, or undefined when unparseable.
 */
export function policyEffect(item: Policy): "allow" | "deny" | undefined {
  const result = POLICY_REGEX.exec(item.policy);
  if (!result?.groups) {
    return undefined;
  }
  return result.groups.effect === "permit" ? "allow" : "deny";
}

export interface SubmitPermissionsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: ResourceAddress;
  submitting: PermissionEntries;
  initial: PermissionEntries;
}

export async function submitPermissions(
  params: SubmitPermissionsParams,
): Promise<void> {
  const tasks: Array<Promise<void>> = [];
  const newPolicies: PolicyPayload[] = [];

  for (const [principal, access] of Object.entries(params.submitting)) {
    const initialAccess = params.initial[principal];
    for (const [action, newPermission] of Object.entries(access)) {
      const oldPermission = initialAccess?.[action as ResourceActionName];
      if (oldPermission?.effect === newPermission.effect) {
        continue;
      }

      if (oldPermission?.policies) {
        tasks.push(
          deletePolicyList({
            auth: params.auth,
            config: params.config,
            policies: oldPermission.policies,
          }),
        );
      }

      if (newPermission.effect === "unset") {
        continue;
      }

      newPolicies.push(
        createPolicyPayload(
          newPermission.effect === "allow" ? "permit" : "forbid",
          principal,
          action as ResourceActionName,
          params.resource,
        ),
      );
    }
  }

  for (const [principal, access] of Object.entries(params.initial)) {
    const newAccess = params.submitting[principal];
    for (const [action, oldPermission] of Object.entries(access)) {
      const newPermission = newAccess?.[action as ResourceActionName];
      if (!newPermission && oldPermission.policies) {
        tasks.push(
          deletePolicyList({
            auth: params.auth,
            config: params.config,
            policies: oldPermission.policies,
          }),
        );
      }
    }
  }

  tasks.push(
    submitBatch({
      auth: params.auth,
      config: params.config,
      batch: newPolicies,
    }),
  );
  await Promise.all(tasks);
}

export interface PolicyPayload {
  principal?: string;
  group?: string;
  action: {
    service: string;
    name: string;
  };
  resource: {
    id: string;
    type: ResourceAddress["type"];
  };
  policy: string;
}

function createPolicyPayload(
  effect: "permit" | "forbid",
  key: string,
  action: ResourceActionName,
  resource: ResourceAddress,
): PolicyPayload {
  const { service, name } = ACTIONS[action];
  const group = isGroupKey(key);
  const groupId = group ? groupIdFromKey(key) : "";
  const principalScope =
    key === ALL_USERS
      ? "principal"
      : group
        ? `principal in Group::"${escapeCedarLiteral(groupId)}"`
        : `principal == Principal::"${escapeCedarLiteral(key)}"`;
  // Editing permissions must cascade to a folder's descendants, so it is scoped
  // by containment ("in") rather than an exact match.
  const resourceScope =
    action === "edit" && resource.type === "folder" ? "in" : "==";
  const payload: PolicyPayload = {
    action: {
      service,
      name,
    },
    resource: {
      id: encodeResourceUrl(resource.address),
      type: resource.type,
    },
    policy: `${effect}(${principalScope}, action == Action::"${service}:${name}", resource ${resourceScope} ${encodeResource(resource)});`,
  };
  if (group) {
    payload.group = groupId;
  } else if (key !== ALL_USERS) {
    payload.principal = key;
  }
  return payload;
}

function escapeCedarLiteral(value: string): string {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t");
}

interface DeletePolicyListParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  policies: Policy[];
}

async function deletePolicyList(params: DeletePolicyListParams): Promise<void> {
  await Promise.all(
    params.policies.map((policy) =>
      deletePolicy({ auth: params.auth, config: params.config, policy }),
    ),
  );
}

interface DeletePolicyParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  policy: Policy;
}

async function deletePolicy(params: DeletePolicyParams): Promise<void> {
  const url = new URL(
    `/v1beta/policies/${params.policy.id}`,
    params.config.url,
  );
  const headers = new Headers();
  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }
  const response = await fetch(url, {
    method: "DELETE",
    headers,
  });

  if (!response.ok) {
    if (response.status === 401) {
      params.auth.onExpire?.();
    }

    throw new Error(
      `Failed to delete policy #${params.policy.id} -- HTTP${response.status}.`,
    );
  }
}

interface SubmitBatchParams {
  auth: PermissionAuth;
  batch: PolicyPayload[];
  config: PermissionConfig;
}

async function submitBatch(params: SubmitBatchParams): Promise<void> {
  const url = new URL("/v1beta/policies/batch/", params.config.url);
  const headers = new Headers();
  headers.set("Content-Type", "application/json");

  if (params.auth.accessToken) {
    headers.set("Authorization", `Bearer ${params.auth.accessToken}`);
  }
  const response = await fetch(url, {
    body: JSON.stringify(params.batch),
    method: "PUT",
    headers: {
      Authorization: `Bearer ${params.auth.accessToken}`,
      "Content-Type": "application/json",
    },
  });

  if (!response.ok) {
    if (response.status === 401) {
      params.auth.onExpire?.();
    }
    throw new Error(`Failed to submit new policies -- HTTP${response.status}.`);
  }
}
