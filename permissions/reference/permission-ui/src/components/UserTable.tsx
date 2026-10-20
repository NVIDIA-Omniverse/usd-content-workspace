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
import { Anchor, Badge, Table, Text } from "@mantine/core";
import { NavLink, useNavigate } from "react-router-dom";
import type { UserInfo } from "../state/users";

export interface UserTableProps {
  users: UserInfo[];
  emptyMessage?: string;
}

export function UserTable({
  users,
  emptyMessage = "No users found.",
}: UserTableProps) {
  const navigate = useNavigate();

  return (
    <Table
      highlightOnHover
      stickyHeader
      striped={"even"}
      withTableBorder
      withRowBorders
      verticalSpacing={"sm"}
    >
      <Table.Thead>
        <Table.Tr>
          <Table.Th>Name</Table.Th>
          <Table.Th>Email</Table.Th>
          <Table.Th>Username</Table.Th>
          <Table.Th w={100}>Status</Table.Th>
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {users.length === 0 ? (
          <Table.Tr>
            <Table.Td colSpan={4}>
              <Text size={"sm"} c={"dimmed"} ta={"center"} py={"md"}>
                {emptyMessage}
              </Text>
            </Table.Td>
          </Table.Tr>
        ) : (
          users.map((u) => (
            <Table.Tr
              key={u.id}
              style={{ cursor: "pointer" }}
              onClick={() => navigate(`/users/${encodeURIComponent(u.id)}`)}
            >
              <Table.Td>
                <Anchor
                  component={NavLink}
                  to={`/users/${encodeURIComponent(u.id)}`}
                >
                  {u.display_name ?? u.id}
                </Anchor>
              </Table.Td>
              <Table.Td>{u.email}</Table.Td>
              <Table.Td>{u.username}</Table.Td>
              <Table.Td>
                {u.active != null && (
                  <Badge
                    variant={"light"}
                    color={u.active ? "green" : "gray"}
                    size={"sm"}
                  >
                    {u.active ? "Active" : "Inactive"}
                  </Badge>
                )}
              </Table.Td>
            </Table.Tr>
          ))
        )}
      </Table.Tbody>
    </Table>
  );
}
