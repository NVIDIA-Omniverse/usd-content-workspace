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

import z from "zod";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";

const UserInfo = z.object({
  id: z.string(),
  display_name: z.string().nullish(),
  given_name: z.string().nullish(),
  family_name: z.string().nullish(),
  email: z.string().nullish(),
  username: z.string().nullish(),
  active: z.boolean().nullish(),
  job_title: z.string().nullish(),
});

export type UserInfo = z.infer<typeof UserInfo>;

const UserPage = z.object({
  items: z.array(UserInfo),
  next_cursor: z.string().nullish(),
});

export type UserPage = z.infer<typeof UserPage>;

const GroupInfo = z.object({
  id: z.string(),
  display_name: z.string().nullish(),
  description: z.string().nullish(),
});

export type GroupInfo = z.infer<typeof GroupInfo>;

const GroupPage = z.object({
  items: z.array(GroupInfo),
  next_cursor: z.string().nullish(),
});

export type GroupPage = z.infer<typeof GroupPage>;

export interface FetchCurrentUserParams {
  auth: PermissionAuth;
  config: PermissionConfig;
}

export async function fetchCurrentUser({
  auth,
  config,
}: FetchCurrentUserParams): Promise<UserInfo> {
  const url = new URL("/api/v1alpha/users/me", config.userInfoUrl);

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return UserInfo.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch current user: HTTP${response.status} -- ${text}`,
  );
}

export interface FetchUserParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  userId: string;
}

export async function fetchUser({
  auth,
  config,
  userId,
}: FetchUserParams): Promise<UserInfo> {
  const url = new URL(
    `/api/v1alpha/users/${encodeURIComponent(userId)}`,
    config.userInfoUrl,
  );

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return UserInfo.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(`Failed to fetch user: HTTP${response.status} -- ${text}`);
}

export interface FetchUsersParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  displayName?: string;
  cursor?: string;
  limit?: number;
}

export async function fetchUsers({
  auth,
  config,
  displayName,
  cursor,
  limit,
}: FetchUsersParams): Promise<UserPage> {
  const url = new URL("/api/v1alpha/users", config.userInfoUrl);
  if (displayName) url.searchParams.set("display_name", displayName);
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return UserPage.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(`Failed to fetch users: HTTP${response.status} -- ${text}`);
}

export interface FetchGroupParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  groupId: string;
}

export async function fetchGroup({
  auth,
  config,
  groupId,
}: FetchGroupParams): Promise<GroupInfo> {
  const url = new URL(
    `/api/v1alpha/groups/${encodeURIComponent(groupId)}`,
    config.userInfoUrl,
  );

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return GroupInfo.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(`Failed to fetch group: HTTP${response.status} -- ${text}`);
}

export interface FetchGroupsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  displayName?: string;
  cursor?: string;
  limit?: number;
}

export async function fetchGroups({
  auth,
  config,
  displayName,
  cursor,
  limit,
}: FetchGroupsParams): Promise<GroupPage> {
  const url = new URL("/api/v1alpha/groups", config.userInfoUrl);
  if (displayName) url.searchParams.set("display_name", displayName);
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return GroupPage.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(`Failed to fetch groups: HTTP${response.status} -- ${text}`);
}

export interface FetchUserGroupsParams {
  auth: PermissionAuth;
  config: PermissionConfig;
  userId: string;
  cursor?: string;
  limit?: number;
}

/**
 * Retrieves every group a user belongs to by following the cursor pagination
 * until all pages have been read.
 *
 * @param params.auth Authentication for the request.
 * @param params.config Service configuration providing the user-info URL.
 * @param params.userId User whose group membership is fetched.
 * @param params.cursor Optional starting cursor; defaults to the first page.
 * @param params.limit Optional page size; defaults to 200.
 * @returns Every group the user is a member of.
 */
export async function fetchAllUserGroups(
  params: FetchUserGroupsParams,
): Promise<GroupInfo[]> {
  const groups: GroupInfo[] = [];
  const seenCursors = new Set<string>();
  let cursor = params.cursor;
  do {
    const page = await fetchUserGroups({
      ...params,
      cursor,
      limit: params.limit ?? 200,
    });
    groups.push(...page.items);

    const next = page.next_cursor ?? undefined;
    if (next && (next === cursor || seenCursors.has(next))) {
      console.warn(
        `Stopping user group pagination: cursor "${next}" was already seen.`,
      );
      break;
    }
    if (next) {
      seenCursors.add(next);
    }
    cursor = next;
  } while (cursor);

  return groups;
}

/**
 * Fetches a single page of the groups a user belongs to.
 *
 * @param params.auth Authentication for the request.
 * @param params.config Service configuration providing the user-info URL.
 * @param params.userId User whose groups are fetched.
 * @param params.cursor Optional pagination cursor for the page to fetch.
 * @param params.limit Optional page size.
 * @returns One page of groups together with the cursor to the next page.
 */
export async function fetchUserGroups({
  auth,
  config,
  userId,
  cursor,
  limit,
}: FetchUserGroupsParams): Promise<GroupPage> {
  const url = new URL(
    `/api/v1alpha/users/${encodeURIComponent(userId)}/groups`,
    config.userInfoUrl,
  );
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const headers = new Headers();
  if (auth.accessToken) {
    headers.set("Authorization", `Bearer ${auth.accessToken}`);
  }

  const response = await fetch(url, { headers });
  if (response.ok) {
    const json = await response.json();
    return GroupPage.parse(json);
  }

  if (response.status === 401) {
    auth.onExpire?.();
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch user groups: HTTP${response.status} -- ${text}`,
  );
}

export function formatUserName(user: UserInfo): string {
  if (user.display_name) return user.display_name;
  if (user.email) return user.email;
  return user.id;
}

export function formatUserLabel(user: UserInfo): string {
  const name = user.display_name ?? user.id;
  if (user.email) return `${name} (${user.email})`;
  return name;
}

export function formatGroupName(group: GroupInfo): string {
  if (group.display_name) return group.display_name;
  return group.id;
}
