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

function userServiceUrl(config: Config): string {
  const base = config.endpoints.userService;
  if (!base) throw new Error("User service endpoint is not configured");
  return base;
}

export interface ListUsersParams {
  config: Config;
  user: User;
  displayName?: string;
  email?: string;
  cursor?: string;
  limit?: number;
}

export async function fetchUsers({
  config,
  user,
  displayName,
  email,
  cursor,
  limit,
}: ListUsersParams): Promise<UserPage> {
  const url = new URL("/api/v1alpha/users", userServiceUrl(config));
  if (displayName) url.searchParams.set("display_name", displayName);
  if (email) url.searchParams.set("email", email);
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return UserPage.parse(json);
  }

  const text = await response.text();
  throw new Error(`Failed to fetch users: HTTP${response.status} -- ${text}`);
}

export interface GetUserParams {
  userId: string;
  config: Config;
  user: User;
}

export async function fetchUser({
  userId,
  config,
  user,
}: GetUserParams): Promise<UserInfo> {
  const url = new URL(
    `/api/v1alpha/users/${encodeURIComponent(userId)}`,
    userServiceUrl(config),
  );

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return UserInfo.parse(json);
  }

  const text = await response.text();
  throw new HttpError(
    response.status,
    `Failed to fetch user: HTTP${response.status} -- ${text}`,
  );
}

export interface FetchAllUserGroupsParams {
  userId: string;
  config: Config;
  user: User;
}

export async function fetchAllUserGroups({
  userId,
  config,
  user,
}: FetchAllUserGroupsParams): Promise<GroupInfo[]> {
  const groups: GroupInfo[] = [];
  let cursor: string | undefined;
  do {
    const page = await fetchUserGroups({ userId, config, user, cursor });
    groups.push(...page.items);
    cursor = page.next_cursor ?? undefined;
  } while (cursor);
  return groups;
}

export interface ListUserGroupsParams {
  userId: string;
  config: Config;
  user: User;
  cursor?: string;
  limit?: number;
}

export async function fetchUserGroups({
  userId,
  config,
  user,
  cursor,
  limit,
}: ListUserGroupsParams): Promise<GroupPage> {
  const url = new URL(
    `/api/v1alpha/users/${encodeURIComponent(userId)}/groups`,
    userServiceUrl(config),
  );
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return GroupPage.parse(json);
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch user groups: HTTP${response.status} -- ${text}`,
  );
}

export interface ListGroupsParams {
  config: Config;
  user: User;
  displayName?: string;
  cursor?: string;
  limit?: number;
}

export async function fetchGroups({
  config,
  user,
  displayName,
  cursor,
  limit,
}: ListGroupsParams): Promise<GroupPage> {
  const url = new URL("/api/v1alpha/groups", userServiceUrl(config));
  if (displayName) url.searchParams.set("display_name", displayName);
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return GroupPage.parse(json);
  }

  const text = await response.text();
  throw new Error(`Failed to fetch groups: HTTP${response.status} -- ${text}`);
}

export interface GetGroupParams {
  groupId: string;
  config: Config;
  user: User;
}

export async function fetchGroup({
  groupId,
  config,
  user,
}: GetGroupParams): Promise<GroupInfo> {
  const url = new URL(
    `/api/v1alpha/groups/${encodeURIComponent(groupId)}`,
    userServiceUrl(config),
  );

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return GroupInfo.parse(json);
  }

  const text = await response.text();
  throw new HttpError(
    response.status,
    `Failed to fetch group: HTTP${response.status} -- ${text}`,
  );
}

export interface ListGroupMembersParams {
  groupId: string;
  config: Config;
  user: User;
  cursor?: string;
  limit?: number;
}

export async function fetchGroupMembers({
  groupId,
  config,
  user,
  cursor,
  limit,
}: ListGroupMembersParams): Promise<UserPage> {
  const url = new URL(
    `/api/v1alpha/groups/${encodeURIComponent(groupId)}/users`,
    userServiceUrl(config),
  );
  if (cursor) url.searchParams.set("cursor", cursor);
  if (limit) url.searchParams.set("limit", String(limit));

  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${user.access_token}`,
    },
  });
  if (response.ok) {
    const json = await response.json();
    return UserPage.parse(json);
  }

  const text = await response.text();
  throw new Error(
    `Failed to fetch group members: HTTP${response.status} -- ${text}`,
  );
}

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

const GroupInfo = z.object({
  id: z.string(),
  display_name: z.string().nullish(),
  description: z.string().nullish(),
});

export type GroupInfo = z.infer<typeof GroupInfo>;

const UserPage = z.object({
  items: z.array(UserInfo),
  next_cursor: z.string().nullish(),
});

export type UserPage = z.infer<typeof UserPage>;

const GroupPage = z.object({
  items: z.array(GroupInfo),
  next_cursor: z.string().nullish(),
});

export type GroupPage = z.infer<typeof GroupPage>;

export class HttpError extends Error {
  status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}
