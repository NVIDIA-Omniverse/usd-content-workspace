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
  Breadcrumbs,
  Group,
  Loader,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { IconUsersGroup } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { Navigate, NavLink, useParams } from "react-router-dom";
import { CursorPagination } from "../components/CursorPagination";
import { Page } from "../components/Page";
import { UserTable } from "../components/UserTable";
import { useConfig } from "../hooks/useConfig";
import { useCursorPagination } from "../hooks/useCursorPagination";
import { fetchGroup, fetchGroupMembers } from "../state/users";

export function GroupDetails() {
  const { groupId = "" } = useParams<{ groupId: string }>();
  const config = useConfig();
  const auth = useAuth();
  const pagination = useCursorPagination("membersCursor");

  const groupQuery = useQuery({
    queryKey: ["group", groupId],
    queryFn: () => fetchGroup({ groupId, config, user: auth.user! }),
    enabled: !!groupId,
  });

  const membersQuery = useQuery({
    queryKey: ["group-members", groupId, pagination.cursor],
    queryFn: () =>
      fetchGroupMembers({
        groupId,
        config,
        user: auth.user!,
        cursor: pagination.cursor,
      }),
    enabled: !!groupId,
  });

  if (!groupId) return <Navigate to={"/groups"} />;

  if (groupQuery.isLoading) {
    return (
      <Page title={"Group"}>
        <Loader />
      </Page>
    );
  }

  if (groupQuery.isError) {
    return (
      <Page title={"Group"}>
        <Alert color={"red"}>{groupQuery.error.message}</Alert>
      </Page>
    );
  }

  const group = groupQuery.data!;
  const title = group.display_name ?? group.id;

  return (
    <Page title={`Group — ${title}`}>
      <Group gap={"sm"} align={"flex-start"}>
        <IconUsersGroup size={32} style={{ marginTop: 22 }} />
        <div>
          <Text size={"xs"} tt={"uppercase"} c={"dimmed"} fw={700}>
            Group
          </Text>
          <Title>{title}</Title>
        </div>
      </Group>
      <Breadcrumbs>
        <Anchor component={NavLink} to={"/groups"}>
          Groups
        </Anchor>
        <Anchor
          component={NavLink}
          to={`/groups/${encodeURIComponent(groupId)}`}
        >
          {title}
        </Anchor>
      </Breadcrumbs>

      <Table variant={"vertical"} withTableBorder>
        <Table.Tbody>
          <Table.Tr>
            <Table.Th w={150}>ID</Table.Th>
            <Table.Td>{group.id}</Table.Td>
          </Table.Tr>
          <Table.Tr>
            <Table.Th>Description</Table.Th>
            <Table.Td>{group.description}</Table.Td>
          </Table.Tr>
        </Table.Tbody>
      </Table>

      <Title order={3} mt={"lg"}>
        Members
      </Title>

      {membersQuery.isLoading ? (
        <Loader />
      ) : membersQuery.isError ? (
        <Alert color={"red"}>{membersQuery.error.message}</Alert>
      ) : membersQuery.isSuccess ? (
        <>
          <UserTable
            users={membersQuery.data.items}
            emptyMessage={"No members found."}
          />

          <CursorPagination
            hasPrevious={pagination.hasPrevious}
            hasNext={!!membersQuery.data.next_cursor}
            onPrevious={pagination.goPrevious}
            onNext={() => pagination.goNext(membersQuery.data.next_cursor!)}
          />
        </>
      ) : null}
    </Page>
  );
}
