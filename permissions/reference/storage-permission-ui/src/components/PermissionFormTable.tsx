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

import {
  ActionIcon,
  Alert,
  Badge,
  Box,
  Button,
  Center,
  Checkbox,
  Divider,
  Group,
  Stack,
  Table,
  Text,
  Tooltip,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { notifications } from "@mantine/notifications";
import {
  IconCircleXFilled,
  IconInfoCircle,
  IconLock,
  IconPlus,
  IconUsersGroup,
} from "@tabler/icons-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { UserAccess } from "../api/authorization.js";
import {
  actionLabel,
  ALL_USERS,
  groupIdFromKey,
  groupKey,
  isGroupKey,
  type PermissionEffect,
  type Permissions,
  type PrincipalType,
  type ResourceAccess,
  type ResourceActionName,
  submitPermissions,
} from "../api/policies.js";
import { useGroupName } from "../hooks/useGroupName.js";
import { usePrincipalName } from "../hooks/usePrincipalName.js";
import type { PermissionAuth } from "../types/PermissionAuth";
import type { PermissionConfig } from "../types/PermissionConfig";
import type { ResourceAddress } from "../types/ResourceAddress";
import { PermissionCheckbox } from "./PermissionCheckbox.js";
import Placeholder from "./Placeholder.js";
import { PrincipalAutocomplete } from "./PrincipalAutocomplete.js";
import { GroupCell, PrincipalCell } from "./PrincipalCells.js";

export interface PermissionFormTableProps {
  auth: PermissionAuth;
  config: PermissionConfig;
  data: Permissions;
  resource: ResourceAddress;
  user: string;
  userAccess: UserAccess;
  onReset: () => void;
}

interface FormValues {
  permissions: [string, ResourceAccess][];
  principal: string;
}

export function PermissionFormTable({
  auth,
  config,
  data,
  resource,
  user,
  userAccess,
}: PermissionFormTableProps) {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationKey: [resource.address, "permissions"],
    mutationFn: async (values: FormValues) => {
      await submitPermissions({
        auth,
        config,
        submitting: Object.fromEntries(values.permissions),
        initial: data.entries,
        resource,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: [resource.address, "permissions"],
      });
      void queryClient.invalidateQueries({
        queryKey: [resource.address, "resolved-permissions"],
      });
      notifications.show({
        title: "Success",
        message: "Permissions updated successfully.",
        color: "green",
      });
    },
  });

  const form = useForm<FormValues>({
    initialValues: {
      permissions: Object.entries(data.entries).sort(([a], [b]) =>
        a === ALL_USERS ? -1 : b === ALL_USERS ? 1 : 0,
      ),
      principal: "",
    },
  });

  function addPrincipal(value: string, type: PrincipalType) {
    if (type === "user" && value === user) {
      form.setFieldError("principal", "You can't manage your own permissions.");
      return;
    }

    const maxPrincipalLength = 1000;
    if (value.length > maxPrincipalLength) {
      form.setFieldError("principal", "Principal is too long.");
      return;
    }

    const invalidCharacters = /['"\\]/;
    if (invalidCharacters.test(value)) {
      form.setFieldError("principal", "Principal contains invalid characters.");
      return;
    }

    const key = type === "group" ? groupKey(value) : value;
    form.setFieldValue(
      "permissions",
      (permissions): FormValues["permissions"] => {
        if (permissions.some(([principal]) => principal === key)) {
          return permissions;
        }
        return [
          ...permissions,
          [
            key,
            {
              read: { effect: "unset" },
              write: { effect: "unset" },
              edit: { effect: "unset" },
            },
          ],
        ];
      },
    );
    form.setFieldValue("principal", "");
  }

  const hasAllUsers = form.values.permissions.some(
    ([key]) => key === ALL_USERS,
  );

  function addAllUsers() {
    form.setFieldValue(
      "permissions",
      (permissions): FormValues["permissions"] => [
        [
          ALL_USERS,
          {
            read: { effect: "unset" },
            write: { effect: "unset" },
            edit: { effect: "unset" },
          },
        ],
        ...permissions,
      ],
    );
  }

  const hasPermissions = form.values.permissions.length > 0;

  if (!userAccess.canEditPermissions && !hasPermissions) {
    return (
      <Center>
        <Placeholder icon={<IconLock size={64} />}>
          There are no permissions to display, and you're not allowed to change
          them for this resource.
        </Placeholder>
      </Center>
    );
  }

  return (
    <form onSubmit={form.onSubmit((values) => mutation.mutate(values))}>
      {mutation.isError && (
        <Alert title={"Failed to submit permission changes"} color={"red"}>
          {mutation.error.message}
        </Alert>
      )}

      <Stack gap={"md"}>
        <Stack gap={0}>
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
                  <Group gap={4} wrap={"nowrap"}>
                    ACL
                    <Tooltip
                      label={
                        <Text size={"xs"}>
                          Controls whether the principal can edit the
                          permissions (ACL) of this file or folder.
                        </Text>
                      }
                      withArrow
                      multiline
                      w={220}
                    >
                      <IconInfoCircle
                        size={14}
                        style={{ cursor: "help" }}
                        aria-label={"About the ACL column"}
                      />
                    </Tooltip>
                  </Group>
                </Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {!hasAllUsers && userAccess.canEditPermissions && (
                <Table.Tr>
                  <Table.Td
                    p={"sm"}
                    onClick={mutation.isPending ? undefined : addAllUsers}
                    style={{ cursor: "pointer" }}
                  >
                    <Badge
                      size={"sm"}
                      variant={"light"}
                      color={"gray"}
                      leftSection={
                        <IconPlus
                          size={12}
                          style={{ width: 12, minWidth: 12 }}
                        />
                      }
                      style={{ pointerEvents: "none" }}
                    >
                      All Users
                    </Badge>
                  </Table.Td>
                  <Table.Td p={"sm"}>
                    <Checkbox style={{ visibility: "hidden" }} />
                  </Table.Td>
                  <Table.Td p={"sm"}>
                    <Checkbox style={{ visibility: "hidden" }} />
                  </Table.Td>
                  <Table.Td p={"sm"}>
                    <Checkbox style={{ visibility: "hidden" }} />
                  </Table.Td>
                </Table.Tr>
              )}
              {form.values.permissions.map(([key, access], index) => (
                <PermissionRow
                  key={key}
                  principalKey={key}
                  access={access}
                  index={index}
                  currentUser={user}
                  auth={auth}
                  config={config}
                  canEdit={userAccess.canEditPermissions}
                  pending={mutation.isPending}
                  onToggle={(rowIndex, action, effect) =>
                    form.setFieldValue(
                      `permissions.${rowIndex}.1.${action}.effect`,
                      effect,
                    )
                  }
                  onRemove={(rowIndex) =>
                    form.removeListItem("permissions", rowIndex)
                  }
                />
              ))}
            </Table.Tbody>
          </Table>

          {userAccess.canEditPermissions && (
            <PrincipalAutocomplete
              auth={auth}
              config={config}
              disabled={mutation.isPending}
              error={form.errors.principal}
              onSubmit={(value, type) => {
                form.clearFieldError("principal");
                addPrincipal(value, type);
              }}
            />
          )}
        </Stack>

        {userAccess.canEditPermissions && (
          <>
            <Divider />
            <Group>
              <Button
                disabled={mutation.isPending}
                loading={mutation.isPending}
                type={"submit"}
              >
                Submit
              </Button>
              <Button
                color={"gray"}
                disabled={mutation.isPending}
                type={"submit"}
                variant={"light"}
                onClick={() => form.reset()}
              >
                Cancel
              </Button>
            </Group>
          </>
        )}
      </Stack>
    </form>
  );
}

interface PermissionRowProps {
  principalKey: string;
  access: ResourceAccess;
  index: number;
  currentUser: string;
  auth: PermissionAuth;
  config: PermissionConfig;
  canEdit: boolean;
  pending: boolean;
  onToggle: (
    index: number,
    action: ResourceActionName,
    effect: PermissionEffect,
  ) => void;
  onRemove: (index: number) => void;
}

function PermissionRow({
  principalKey,
  access,
  index,
  currentUser,
  auth,
  config,
  canEdit,
  pending,
  onToggle,
  onRemove,
}: PermissionRowProps) {
  const isAllUsers = principalKey === ALL_USERS;
  const isGroup = isGroupKey(principalKey);
  const isUser = !isAllUsers && !isGroup;
  const displayId = isGroup ? groupIdFromKey(principalKey) : principalKey;
  const isCurrentUser = isUser && principalKey === currentUser;
  const disabled = pending || isCurrentUser || !canEdit;

  const principal = usePrincipalName(
    isUser ? principalKey : undefined,
    auth,
    config,
  );
  const group = useGroupName(isGroup ? displayId : undefined, auth, config);
  const resolvedName = isAllUsers
    ? "All Users"
    : isGroup
      ? (group.displayName ?? displayId)
      : (principal.user?.display_name ?? principal.user?.email ?? principalKey);

  const readEffect = access.read?.effect ?? "unset";
  const writeEffect = access.write?.effect ?? "unset";
  const editEffect = access.edit?.effect ?? "unset";
  const readReadonly = access.read?.readonly ?? false;
  const writeReadonly = access.write?.readonly ?? false;
  const editReadonly = access.edit?.readonly ?? false;
  const entryReadonly = readReadonly || writeReadonly || editReadonly;
  const readDescription = describeAction(
    readEffect,
    resolvedName,
    "read",
    readReadonly,
  );
  const writeDescription = describeAction(
    writeEffect,
    resolvedName,
    "write",
    writeReadonly,
  );
  const editDescription = describeAction(
    editEffect,
    resolvedName,
    "edit",
    editReadonly,
  );
  const removeDescription = `Remove ${resolvedName}`;

  return (
    <Table.Tr>
      <Table.Td p={"sm"}>
        <Group gap={"xs"} wrap={"nowrap"}>
          {isAllUsers ? (
            <Badge
              size={"sm"}
              variant={"light"}
              leftSection={
                <IconUsersGroup size={12} style={{ width: 12, minWidth: 12 }} />
              }
            >
              All Users
            </Badge>
          ) : isGroup ? (
            <GroupCell groupId={displayId} auth={auth} config={config} />
          ) : (
            <PrincipalCell
              principalId={principalKey}
              auth={auth}
              config={config}
            />
          )}
          {(isAllUsers || !isCurrentUser) && canEdit && !entryReadonly && (
            <Tooltip
              label={<Text size={"xs"}>{removeDescription}</Text>}
              withArrow
            >
              <ActionIcon
                color={"red"}
                disabled={disabled}
                aria-label={removeDescription}
                variant={"transparent"}
                onClick={() => onRemove(index)}
              >
                <IconCircleXFilled size={20} />
              </ActionIcon>
            </Tooltip>
          )}
        </Group>
      </Table.Td>
      <Table.Td p={"sm"}>
        <Tooltip label={<Text size={"xs"}>{readDescription}</Text>} withArrow>
          <Box w={"fit-content"}>
            <PermissionCheckbox
              disabled={disabled || readReadonly}
              effect={readEffect}
              ariaLabel={readDescription}
              onChange={(effect) => onToggle(index, "read", effect)}
            />
          </Box>
        </Tooltip>
      </Table.Td>
      <Table.Td p={"sm"}>
        <Tooltip label={<Text size={"xs"}>{writeDescription}</Text>} withArrow>
          <Box w={"fit-content"}>
            <PermissionCheckbox
              disabled={disabled || writeReadonly}
              effect={writeEffect}
              ariaLabel={writeDescription}
              onChange={(effect) => onToggle(index, "write", effect)}
            />
          </Box>
        </Tooltip>
      </Table.Td>
      <Table.Td p={"sm"}>
        <Tooltip label={<Text size={"xs"}>{editDescription}</Text>} withArrow>
          <Box w={"fit-content"}>
            <PermissionCheckbox
              disabled={disabled || editReadonly}
              effect={editEffect}
              ariaLabel={editDescription}
              onChange={(effect) => onToggle(index, "edit", effect)}
            />
          </Box>
        </Tooltip>
      </Table.Td>
    </Table.Tr>
  );
}

function describeAction(
  effect: PermissionEffect,
  name: string,
  action: ResourceActionName,
  readonly = false,
): string {
  const label = actionLabel(action);
  const status =
    effect === "allow"
      ? `${label} is allowed for ${name}`
      : effect === "deny"
        ? `${label} is denied for ${name}`
        : `${label} is not set for ${name}`;
  if (readonly) {
    return `${status}. Applied by a policy that can't be managed here.`;
  }
  return status;
}
