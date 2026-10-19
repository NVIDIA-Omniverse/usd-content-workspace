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
import {
  Anchor,
  Alert,
  Badge,
  Breadcrumbs,
  Group,
  Loader,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { IconUser } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { Navigate, NavLink, useParams } from "react-router-dom";
import { CursorPagination } from "../components/CursorPagination";
import { GroupTable } from "../components/GroupTable";
import { Page } from "../components/Page";
import { useConfig } from "../hooks/useConfig";
import { useCursorPagination } from "../hooks/useCursorPagination";
import { fetchUser, fetchUserGroups } from "../state/users";

export function UserDetails() {
  const { userId = "" } = useParams<{ userId: string }>();
  const config = useConfig();
  const auth = useAuth();
  const pagination = useCursorPagination("groupsCursor");

  const userQuery = useQuery({
    queryKey: ["user", userId],
    queryFn: () => fetchUser({ userId, config, user: auth.user! }),
    enabled: !!userId,
  });

  const groupsQuery = useQuery({
    queryKey: ["user-groups", userId, pagination.cursor],
    queryFn: () =>
      fetchUserGroups({
        userId,
        config,
        user: auth.user!,
        cursor: pagination.cursor,
      }),
    enabled: !!userId,
  });

  if (!userId) return <Navigate to={"/users"} />;

  if (userQuery.isLoading) {
    return (
      <Page title={"User"}>
        <Loader />
      </Page>
    );
  }

  if (userQuery.isError) {
    return (
      <Page title={"User"}>
        <Alert color={"red"}>{userQuery.error.message}</Alert>
      </Page>
    );
  }

  const user = userQuery.data!;
  const title = user.display_name ?? user.id;

  return (
    <Page title={`User — ${title}`}>
      <Group gap={"sm"} align={"flex-start"}>
        <IconUser size={32} style={{ marginTop: 22 }} />
        <div>
          <Text size={"xs"} tt={"uppercase"} c={"dimmed"} fw={700}>
            User
          </Text>
          <Title>{title}</Title>
        </div>
      </Group>
      <Breadcrumbs>
        <Anchor component={NavLink} to={"/users"}>
          Users
        </Anchor>
        <Anchor
          component={NavLink}
          to={`/users/${encodeURIComponent(userId)}`}
        >
          {title}
        </Anchor>
      </Breadcrumbs>

      <Table variant={"vertical"} withTableBorder>
        <Table.Tbody>
          <Table.Tr>
            <Table.Th w={150}>ID</Table.Th>
            <Table.Td>{user.id}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Email</Table.Th>
            <Table.Td>{user.email}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Username</Table.Th>
            <Table.Td>{user.username}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Given name</Table.Th>
            <Table.Td>{user.given_name}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Family name</Table.Th>
            <Table.Td>{user.family_name}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Job title</Table.Th>
            <Table.Td>{user.job_title}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Status</Table.Th>
            <Table.Td>
              {user.active != null && (
                <Badge color={user.active ? "green" : "gray"}>
                  {user.active ? "Active" : "Inactive"}
                </Badge>
              )}
            </Table.Td>
          </Table.Tr>
        </Table.Tbody>
      </Table>

      <Title order={3} mt={"lg"}>
        Group membership
      </Title>

      {groupsQuery.isLoading ? (
        <Loader />
      ) : groupsQuery.isError ? (
        <Alert color={"red"}>{groupsQuery.error.message}</Alert>
      ) : groupsQuery.isSuccess ? (
        <>
          <GroupTable
            groups={groupsQuery.data.items}
            emptyMessage={"No group memberships found."}
          />

          <CursorPagination
            hasPrevious={pagination.hasPrevious}
            hasNext={!!groupsQuery.data.next_cursor}
            onPrevious={pagination.goPrevious}
            onNext={() => pagination.goNext(groupsQuery.data.next_cursor!)}
          />
        </>
      ) : null}
    </Page>
  );
}

