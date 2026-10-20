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

import { Badge, Box, Table, Text, Tooltip } from "@mantine/core";
import { IconUsersGroup } from "@tabler/icons-react";
import {
  GLOBAL_SOURCE,
  type ResolvedPermission,
  type ResolvedPermissions,
} from "../api/resolved.js";
import {
  ALL_USERS,
  groupIdFromKey,
  isGroupKey,
  type ResourceActionName,
} from "../api/policies.js";
import { useGroupName } from "../hooks/useGroupName.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";
import { PermissionCheckbox } from "./PermissionCheckbox.js";
import { GroupCell, PrincipalCell } from "./PrincipalCells.js";

export interface ResolvedPermissionsTableProps {
  auth: PermissionAuth;
  config: PermissionConfig;
  data: ResolvedPermissions;
  resource: ResourceAddress;
}

/**
 * Renders the read-only table of effective permissions, one row per principal
 * with All Users shown first, and read/write/ACL displayed as disabled checkboxes
 * whose tooltips explain where each permission comes from.
 *
 * @param props.auth Authentication used to resolve principal names.
 * @param props.config Service configuration providing the user-info endpoint.
 * @param props.data Resolved permissions to display.
 * @param props.resource Selected resource, used to tell inherited permissions
 *   apart from ones applied directly to the selection.
 */
export function ResolvedPermissionsTable({
  auth,
  config,
  data,
  resource,
}: ResolvedPermissionsTableProps) {
  const rows = Object.entries(data.entries).sort(([a], [b]) =>
    a === ALL_USERS ? -1 : b === ALL_USERS ? 1 : 0,
  );

  if (rows.length === 0) {
    return null;
  }

  return (
    <Table withRowBorders={false}>
      <Table.Thead>
        <Table.Tr>
          <Table.Th px={"sm"}>Principal</Table.Th>
          <Table.Th px={"sm"} w={70}>
            Read
          </Table.Th>
          <Table.Th px={"sm"} w={70}>
            Write
          </Table.Th>
          <Table.Th px={"sm"} w={70}>
            ACL
          </Table.Th>
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {rows.map(([key, access]) => {
          const isAllUsers = key === ALL_USERS;
          const isGroup = isGroupKey(key);
          const displayId = isGroup ? groupIdFromKey(key) : key;
          const label = isAllUsers ? "All Users" : displayId;
          return (
            <Table.Tr key={key}>
              <Table.Td p={"sm"}>
                {isAllUsers ? (
                  <Badge
                    size={"sm"}
                    variant={"light"}
                    leftSection={
                      <IconUsersGroup
                        size={12}
                        style={{ width: 12, minWidth: 12 }}
                      />
                    }
                  >
                    All Users
                  </Badge>
                ) : isGroup ? (
                  <GroupCell groupId={displayId} auth={auth} config={config} />
                ) : (
                  <PrincipalCell
                    principalId={key}
                    auth={auth}
                    config={config}
                  />
                )}
              </Table.Td>
              <Table.Td p={"sm"}>
                <EffectCell
                  action={"read"}
                  auth={auth}
                  config={config}
                  label={label}
                  permission={access.read}
                  selection={resource.address}
                />
              </Table.Td>
              <Table.Td p={"sm"}>
                <EffectCell
                  action={"write"}
                  auth={auth}
                  config={config}
                  label={label}
                  permission={access.write}
                  selection={resource.address}
                />
              </Table.Td>
              <Table.Td p={"sm"}>
                <EffectCell
                  action={"edit"}
                  auth={auth}
                  config={config}
                  label={label}
                  permission={access.edit}
                  selection={resource.address}
                />
              </Table.Td>
            </Table.Tr>
          );
        })}
      </Table.Tbody>
    </Table>
  );
}

interface EffectCellProps {
  action: ResourceActionName;
  auth: PermissionAuth;
  config: PermissionConfig;
  label: string;
  permission: ResolvedPermission;
  selection: string;
}

/**
 * Renders a single disabled permission checkbox for one action, together with a
 * tooltip that explains where the effective permission originates (the current
 * selection, an ancestor, or a group). When no policy applies the permission is
 * shown as denied so an empty checkbox never leaves the effective access
 * ambiguous.
 *
 * @param props.action Action this cell represents.
 * @param props.auth Authentication used to resolve the source group's name.
 * @param props.config Service configuration providing the user-info endpoint.
 * @param props.label Human-readable principal label used for the accessible name.
 * @param props.permission Resolved permission to visualize.
 * @param props.selection Address of the selected resource, used to detect
 *   inheritance.
 */
function EffectCell({
  action,
  auth,
  config,
  label,
  permission,
  selection,
}: EffectCellProps) {
  const { displayName } = useGroupName(permission.viaGroup, auth, config);

  const notGranted = permission.effect === "unset";
  const description = notGranted
    ? "Denied by default, as no policy applies"
    : describeInheritance(permission, selection, displayName);
  const actionName = action === "edit" ? "ACL" : action;
  const effect = notGranted ? "deny" : permission.effect;
  const state = effect === "deny" ? "denied" : "allowed";
  const ariaLabel = [
    `Resolved ${actionName} for ${label}: ${state}`,
    description,
  ]
    .filter(Boolean)
    .join(". ");
  const checkbox = (
    <PermissionCheckbox
      disabled
      effect={effect}
      ariaLabel={ariaLabel}
      onChange={() => {}}
    />
  );

  if (!description) {
    return checkbox;
  }

  return (
    <Tooltip label={<Text size={"xs"}>{description}</Text>} withArrow>
      <Box w={"fit-content"}>{checkbox}</Box>
    </Tooltip>
  );
}

/**
 * Produces the tooltip text describing the origin of a resolved permission,
 * distinguishing a global policy, a higher-priority ancestor policy that
 * overrides the selection, inheritance from an ancestor, a grant via a group or
 * the All Users default, a combination of these, or a permission applied
 * directly to the selection.
 *
 * @param permission Resolved permission whose origin is described.
 * @param selection Address of the selected resource.
 * @param groupName Resolved name of the granting group, when the permission is
 *   granted via a group.
 * @returns The tooltip text, or undefined when there is nothing noteworthy to
 *   show.
 */
function describeInheritance(
  permission: ResolvedPermission,
  selection: string,
  groupName: string | undefined,
): string | undefined {
  const group = permission.viaGroup
    ? (groupName ?? permission.viaGroup)
    : undefined;
  const via = group
    ? `via group ${group}`
    : permission.viaAllUsers
      ? "via All Users"
      : undefined;

  if (permission.source === GLOBAL_SOURCE) {
    return via
      ? `Applied by a global policy ${via}`
      : "Applied by a global policy";
  }

  if (permission.overridden) {
    return via
      ? `Overridden by a higher-priority policy on ${permission.source} ${via}`
      : `Overridden by a higher-priority policy on ${permission.source}`;
  }

  const inheritedLevel = !!permission.source && permission.source !== selection;
  if (inheritedLevel) {
    return via
      ? `Inherited from ${permission.source} ${via}`
      : `Inherited from ${permission.source}`;
  }
  if (via) {
    return group
      ? `Granted via group ${group}`
      : "Applied to All Users in selection";
  }
  if (permission.source === selection) {
    return "Applied to the current selection";
  }

  return undefined;
}
