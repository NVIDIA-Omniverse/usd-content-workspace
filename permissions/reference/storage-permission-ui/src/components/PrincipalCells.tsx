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

import { Group, Loader, Text, Tooltip } from "@mantine/core";
import { IconUser, IconUsersGroup } from "@tabler/icons-react";
import { useGroupName } from "../hooks/useGroupName.js";
import { usePrincipalName } from "../hooks/usePrincipalName.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";

export interface PrincipalCellProps {
  principalId: string;
  auth: PermissionAuth;
  config: PermissionConfig;
}

/**
 * Displays a user principal using their resolved display name and email, falling
 * back to the raw identifier while the name is loading or cannot be resolved.
 * The raw identifier is always available in a tooltip.
 *
 * @param props.principalId Identifier of the user to display.
 * @param props.auth Authentication used to resolve the user's name.
 * @param props.config Service configuration providing the user-info endpoint.
 */
export function PrincipalCell({
  principalId,
  auth,
  config,
}: PrincipalCellProps) {
  const { user, isLoading } = usePrincipalName(principalId, auth, config);

  if (isLoading) {
    return <Loader size="xs" />;
  }

  const displayName = user?.display_name ?? user?.email;
  const resolved = !!(user && displayName && displayName !== principalId);

  return (
    <Group gap={6} wrap={"nowrap"} align={"center"}>
      <IconUser size={16} style={{ flexShrink: 0 }} />
      {resolved ? (
        <div>
          <Tooltip label={principalId} withArrow>
            <Text size="sm" fw={500} w="fit-content">
              {displayName}
            </Text>
          </Tooltip>
          {user!.email && user!.email !== displayName && (
            <Text size="xs" c="dimmed">
              {user!.email}
            </Text>
          )}
        </div>
      ) : (
        <Text size="sm">{principalId}</Text>
      )}
    </Group>
  );
}

export interface GroupCellProps {
  groupId: string;
  auth: PermissionAuth;
  config: PermissionConfig;
}

/**
 * Displays a group principal using its resolved name, showing the raw group id
 * in a tooltip and falling back to the id when the name is unavailable.
 *
 * @param props.groupId Identifier of the group to display.
 * @param props.auth Authentication used to resolve the group's name.
 * @param props.config Service configuration providing the user-info endpoint.
 */
export function GroupCell({ groupId, auth, config }: GroupCellProps) {
  const { displayName, isLoading } = useGroupName(groupId, auth, config);

  if (isLoading) {
    return <Loader size="xs" />;
  }

  const label = displayName && displayName !== groupId ? displayName : groupId;
  const content = (
    <Group gap={6} wrap={"nowrap"} align={"center"} w={"fit-content"}>
      <IconUsersGroup size={16} style={{ flexShrink: 0 }} />
      <Text size="sm" fw={500} w="fit-content">
        {label}
      </Text>
    </Group>
  );

  if (label !== groupId) {
    return (
      <Tooltip label={groupId} withArrow>
        {content}
      </Tooltip>
    );
  }

  return content;
}
