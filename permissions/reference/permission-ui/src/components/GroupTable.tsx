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
import { Anchor, Table, Text } from "@mantine/core";
import { NavLink, useNavigate } from "react-router-dom";
import type { GroupInfo } from "../state/users";

export interface GroupTableProps {
  groups: GroupInfo[];
  emptyMessage?: string;
}

export function GroupTable({
  groups,
  emptyMessage = "No groups found.",
}: GroupTableProps) {
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
          <Table.Th>Description</Table.Th>
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {groups.length === 0 ? (
          <Table.Tr>
            <Table.Td colSpan={2}>
              <Text size={"sm"} c={"dimmed"} ta={"center"} py={"md"}>
                {emptyMessage}
              </Text>
            </Table.Td>
          </Table.Tr>
        ) : (
          groups.map((g) => (
            <Table.Tr
              key={g.id}
              style={{ cursor: "pointer" }}
              onClick={() =>
                navigate(`/groups/${encodeURIComponent(g.id)}`)
              }
            >
              <Table.Td>
                <Anchor
                  component={NavLink}
                  to={`/groups/${encodeURIComponent(g.id)}`}
                >
                  {g.display_name ?? g.id}
                </Anchor>
              </Table.Td>
              <Table.Td>
                <Text size={"sm"} lineClamp={2}>
                  {g.description}
                </Text>
              </Table.Td>
            </Table.Tr>
          ))
        )}
      </Table.Tbody>
    </Table>
  );
}
