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
import {
  encodeResource,
  encodeResourceUrl,
  getResourceAncestors,
} from "../utils/resources.js";
import {
  ALL_USERS,
  fetchResourcePolicies,
  groupIdFromKey,
  groupKey,
  isGroupKey,
  isResolvablePolicy,
  policyEffect,
  resolvableActions,
  type PermissionEffect,
  type ResourceActionName,
} from "./policies.js";
import type { Policy } from "./schemas.js";
import { fetchAllUserGroups } from "./userInfo.js";

/** Sentinel source used for permissions that come from a global policy. */
export const GLOBAL_SOURCE = "\0global";

export interface ResolvedPermission {
  effect: PermissionEffect;
  source?: string;
  viaGroup?: string;
  viaAllUsers?: boolean;
  /**
   * True when the winning policy was assigned on an ancestor folder yet decided
   * the outcome at the selection level, overriding whatever is set on the
   * selection because it has a higher priority (lower order).
   */
  overridden?: boolean;
}

export type ResolvedAccess = {
  [action in ResourceActionName]: ResolvedPermission;
};

export interface ResolvedPermissions {
  entries: { [principal: string]: ResolvedAccess };
}

export type Membership = Map<string, Set<string>>;

export interface GetResolvedPermissionsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  resource: ResourceAddress;
}

/**
 * A single read/write decision expressed by one policy for one principal,
 * annotated with everything needed to rank it against competing policies.
 */
export interface Grant {
  /** Principal the policy targets: user id, group key, or All Users. */
  key: string;
  /** Action the decision applies to. */
  action: ResourceActionName;
  /** Whether the policy permits or forbids the action. */
  effect: "allow" | "deny";
  /** Evaluation order; lower wins, like a firewall rule. */
  order: number;
  /** Level the policy is evaluated at: resource address, or GLOBAL_SOURCE. */
  source: string;
  /**
   * Where the policy was actually assigned, for display only. A prefix policy is
   * evaluated at the queried level but assigned on an ancestor folder, so its
   * origin is that folder; otherwise the origin matches the source.
   */
  origin?: string;
}

/**
 * Computes the effective read, write and edit permissions that apply to a resource.
 * Storage read/write are resolved from the bottom up: starting at the selection and
 * moving toward the root, the first level that has a matching policy wins, so
 * closer levels override ancestors. Permissions:edit does not inherit down the
 * folder tree, so it is resolved at the selection only; a cascading edit policy
 * from a parent reaches the selection through its resource prefix rather than the
 * ancestor walk. Global (unscoped-resource) policies are considered at every level.
 * Within a level, the winning policy is the one with the lowest `order`, and on an
 * equal order the default evaluation priority applies: a permit wins over a forbid
 * for storage actions, whereas permissions:edit lets a forbid win over a permit.
 * Group membership is expanded so users inherit grants given to the groups they
 * belong to.
 *
 * @param params.auth Authentication used for the policy and user-info requests.
 * @param params.config Service configuration (policy URL and optional user-info
 *   URL); group expansion is skipped when no user-info URL is configured.
 * @param params.resource Selected resource whose effective permissions are
 *   resolved.
 * @returns The effective permission for every principal referenced by any level.
 */
export async function getResolvedPermissions(
  params: GetResolvedPermissionsParams,
): Promise<ResolvedPermissions> {
  const ancestors = getResourceAncestors(params.resource);

  const [levelItems, globalItems] = await Promise.all([
    Promise.all(
      ancestors.map((resource) =>
        fetchResourcePolicies({
          auth: params.auth,
          config: params.config,
          resource: encodeResource(resource),
          inherited: true,
        }),
      ),
    ),
    fetchResourcePolicies({
      auth: params.auth,
      config: params.config,
      resource: "NULL",
      inherited: true,
    }),
  ]);

  const levels = ancestors.map((resource) => resource.address);
  const levelsById = new Map<string, string>();
  for (const resource of ancestors) {
    levelsById.set(resource.address, resource.address);
    levelsById.set(encodeResourceUrl(resource.address), resource.address);
  }

  const grants: Grant[] = [];
  ancestors.forEach((resource, index) => {
    collectGrants(grants, levelItems[index], resource.address, levelsById);
  });
  collectGrants(grants, globalItems, GLOBAL_SOURCE);

  const membership = await fetchMembership(params, grants);
  return resolvePermissions(grants, membership, levels);
}

/**
 * Turns raw policies into grants, skipping ones that cannot be resolved
 * (conditional or non-read/write) and expanding unscoped actions into both read
 * and write. Every grant is evaluated at the queried level, but its display
 * origin records where the policy was assigned: a prefix policy (no concrete
 * resource but a resource prefix set) is evaluated here yet assigned on an
 * ancestor folder, so the user can be told it comes from that parent.
 *
 * @param grants Accumulator the extracted grants are pushed into.
 * @param items Raw policies for a single resource level or the global set.
 * @param source Resource address, or GLOBAL_SOURCE for global policies.
 * @param levelsById Maps a resource id (raw or encoded) to the level address it
 *   represents; used to name the origin folder of prefix policies.
 */
function collectGrants(
  grants: Grant[],
  items: Policy[],
  source: string,
  levelsById?: Map<string, string>,
): void {
  for (const item of items) {
    if (!isResolvablePolicy(item)) {
      continue;
    }
    const effect = policyEffect(item);
    if (!effect) {
      continue;
    }
    const key = item.group
      ? groupKey(item.group.id)
      : item.principal
        ? item.principal.sub
        : ALL_USERS;
    const order = item.order ?? Number.MAX_SAFE_INTEGER;
    const origin = grantOrigin(item, source, levelsById);
    for (const action of resolvableActions(item)) {
      grants.push({ key, action, effect, order, source, origin });
    }
  }
}

/**
 * Names the folder a grant was assigned on, for display only. Global policies
 * keep the global source and concrete (exact) policies keep the queried level. A
 * prefix policy (no concrete resource but a resource prefix) is assigned on the
 * folder named by its prefix, so that folder is returned instead.
 *
 * @param item Policy being converted into grants.
 * @param source Queried level address, or GLOBAL_SOURCE for global policies.
 * @param levelsById Maps a resource id to the level address it represents.
 * @returns The origin address (or GLOBAL_SOURCE) for display.
 */
function grantOrigin(
  item: Policy,
  source: string,
  levelsById?: Map<string, string>,
): string {
  if (source === GLOBAL_SOURCE || item.resource) {
    return source;
  }
  const prefixId = item.resource_prefix?.id;
  if (!prefixId) {
    return source;
  }
  return levelsById?.get(prefixId) ?? prefixId;
}

/**
 * Builds a lookup of which policy-referenced groups each user belongs to, so
 * group grants can later be attributed to individual users. Users are queried in
 * bounded-concurrency batches, and a user whose membership cannot be fetched is
 * recorded with an empty set rather than failing the whole resolution.
 *
 * @param params The same parameters passed to getResolvedPermissions; only auth
 *   and config are used here.
 * @param grants Grants whose keys name the users and groups of interest.
 * @returns A map from user id to the set of referenced groups they belong to;
 *   empty when no user-info URL is configured.
 */
async function fetchMembership(
  params: GetResolvedPermissionsParams,
  grants: Grant[],
): Promise<Membership> {
  const membership: Membership = new Map();
  if (!params.config.userInfoUrl) {
    return membership;
  }

  const users = new Set<string>();
  const groups = new Set<string>();
  for (const grant of grants) {
    if (grant.key === ALL_USERS) {
      continue;
    }
    if (isGroupKey(grant.key)) {
      groups.add(groupIdFromKey(grant.key));
    } else {
      users.add(grant.key);
    }
  }

  const userIds = [...users];
  const concurrency = 8;
  for (let start = 0; start < userIds.length; start += concurrency) {
    const batch = userIds.slice(start, start + concurrency);
    await Promise.all(
      batch.map(async (userId) => {
        try {
          const userGroups = await fetchAllUserGroups({
            auth: params.auth,
            config: params.config,
            userId,
          });
          const memberOf = new Set(
            userGroups.map((group) => group.id).filter((id) => groups.has(id)),
          );
          membership.set(userId, memberOf);
        } catch {
          membership.set(userId, new Set());
        }
      }),
    );
  }

  return membership;
}

/**
 * Reduces all collected grants into a single effective permission per managed
 * action for every principal that appears in any grant.
 *
 * @param grants Every grant collected across resource levels and globals.
 * @param membership Map from user id to the groups they belong to, used to apply
 *   group grants to individual users.
 * @param levels Resource-level sources ordered from the selection to the root.
 * @returns The effective permissions keyed by principal.
 */
export function resolvePermissions(
  grants: Grant[],
  membership: Membership,
  levels: string[],
): ResolvedPermissions {
  const principals = new Set<string>();
  for (const grant of grants) {
    principals.add(grant.key);
  }

  const actions: ResourceActionName[] = ["read", "write", "edit"];
  const entries: ResolvedPermissions["entries"] = {};

  for (const principal of principals) {
    const access = {} as ResolvedAccess;
    for (const action of actions) {
      access[action] = resolveAction(
        grants,
        membership,
        levels,
        principal,
        action,
      );
    }
    entries[principal] = access;
  }

  return { entries };
}

/**
 * Determines the effective permission for a single principal and action by
 * walking the levels from the selection toward the root and stopping at the
 * first level that has a matching grant. Global grants are considered at every
 * level, so they compete with the closest resource-scoped grants. Within a level
 * the winner is chosen by order (lowest wins), then by effect: permit over forbid
 * for storage actions, forbid over permit for permissions:edit. The edit action is
 * a further exception: it never inherits exact policies from ancestor levels, so
 * only the selection level is considered for it.
 *
 * @param grants Every grant collected across resource levels and globals.
 * @param membership Map from user id to the groups they belong to.
 * @param levels Resource-level sources ordered from the selection to the root.
 * @param principal Principal key being resolved (user id, group key or All
 *   Users).
 * @param action Action whose effect is being resolved.
 * @returns The resolved permission, or an unset permission when nothing applies.
 */
function resolveAction(
  grants: Grant[],
  membership: Membership,
  levels: string[],
  principal: string,
  action: ResourceActionName,
): ResolvedPermission {
  const keys = candidateKeys(principal, membership);
  const globals = grants.filter(
    (grant) =>
      grant.source === GLOBAL_SOURCE &&
      grant.action === action &&
      keys.has(grant.key),
  );

  // Storage actions inherit down the folder tree, but permissions:edit does not:
  // an exact policy applies only to the node it names. A policy that is meant to
  // cascade uses a resource prefix and is already returned at the selection level
  // (with its origin folder recorded), so edit is resolved at the selection only,
  // never by inheriting exact policies from ancestor levels.
  const resolvedLevels = action === "edit" ? levels.slice(0, 1) : levels;

  for (const level of resolvedLevels) {
    let best: Grant | undefined;
    for (const grant of grants) {
      if (grant.source !== level) {
        continue;
      }
      if (grant.action !== action) {
        continue;
      }
      if (!keys.has(grant.key)) {
        continue;
      }
      if (best === undefined || compareGrants(grant, best, action) < 0) {
        best = grant;
      }
    }
    for (const grant of globals) {
      if (best === undefined || compareGrants(grant, best, action) < 0) {
        best = grant;
      }
    }

    if (best) {
      const resolved: ResolvedPermission = {
        effect: best.effect,
        source: best.origin ?? best.source,
      };
      if (isGroupKey(best.key) && best.key !== principal) {
        resolved.viaGroup = groupIdFromKey(best.key);
      } else if (best.key === ALL_USERS && principal !== ALL_USERS) {
        resolved.viaAllUsers = true;
      }
      if (
        best.source === levels[0] &&
        best.origin !== undefined &&
        best.origin !== best.source
      ) {
        resolved.overridden = true;
      }
      return resolved;
    }
  }

  return { effect: "unset" };
}

/**
 * Collects every principal key whose grants can apply to `principal`: the
 * principal itself, All Users, and, for a user, the groups they belong to. These
 * keys only decide which grants are considered; they carry no precedence.
 *
 * @param principal Principal being resolved.
 * @param membership Map from user id to the groups they belong to.
 * @returns The set of principal keys to consider.
 */
function candidateKeys(principal: string, membership: Membership): Set<string> {
  const keys = new Set<string>([principal]);
  if (principal === ALL_USERS) {
    return keys;
  }

  keys.add(ALL_USERS);
  if (!isGroupKey(principal)) {
    for (const groupId of membership.get(principal) ?? []) {
      keys.add(groupKey(groupId));
    }
  }
  return keys;
}

/**
 * Orders two candidate grants by evaluation order (lowest wins) and, on an equal
 * order, by the default evaluation priority. Storage actions let a permit (allow)
 * win over a forbid (deny), while permissions:edit follows the permission service
 * and lets a forbid win over a permit. Nothing else affects the ranking.
 *
 * @param a First grant to compare.
 * @param b Second grant to compare.
 * @param action Managed action both grants apply to.
 * @returns A negative number when the first grant outranks the second.
 */
function compareGrants(a: Grant, b: Grant, action: ResourceActionName): number {
  if (a.order !== b.order) {
    return a.order - b.order;
  }
  return effectRank(a.effect, action) - effectRank(b.effect, action);
}

function effectRank(
  effect: "allow" | "deny",
  action: ResourceActionName,
): number {
  const preferred = action === "edit" ? "deny" : "allow";
  return effect === preferred ? 0 : 1;
}
