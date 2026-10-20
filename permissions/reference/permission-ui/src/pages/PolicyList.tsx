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
  Accordion,
  ActionIcon,
  Alert,
  Anchor,
  Button,
  Checkbox,
  Group,
  Loader,
  NumberInput,
  Popover,
  Stack,
  Table,
  Text,
  ThemeIcon,
  Title,
  Tooltip,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import {
  useDebouncedValue,
  useDisclosure,
  useLocalStorage,
} from "@mantine/hooks";
import { modals } from "@mantine/modals";
import {
  IconAdjustmentsHorizontal,
  IconChevronDown,
  IconChevronUp,
  IconFilePlus,
  IconSearch,
  IconSearchOff,
  IconTrashFilled,
} from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { ComponentProps, ReactNode } from "react";
import { useAuth } from "react-oidc-context";
import { NavLink, useNavigate, useSearchParams } from "react-router-dom";
import { FilterInput } from "../components/FilterInput";
import { NavPagination } from "../components/NavPagination";
import { Page } from "../components/Page";
import { PolicyBadge } from "../components/PolicyBadge";
import { PolicyDeleteDialog } from "../components/PolicyDeleteDialog";
import {
  PrincipalAutocomplete,
  type PrincipalType,
} from "../components/PrincipalAutocomplete";
import { PrincipalName } from "../components/PrincipalName";
import { useConfig } from "../hooks/useConfig";
import { useServiceActionsAutocomplete } from "../hooks/useServiceActionsAutocomplete";
import { useServiceAutocomplete } from "../hooks/useServiceAutocomplete";
import { useServiceResourceTypesAutocomplete } from "../hooks/useServiceResourceTypesAutocomplete";
import { fetchPolicies, type Policy } from "../state/policies";

interface PolicyFilterForm {
  principal: string;
  principalType: PrincipalType;
  service: string;
  action: string;
  resourceType: string;
  resourceId: string;
  order: string;
}

type PolicyColumnKey =
  | "id"
  | "effect"
  | "order"
  | "principal"
  | "service"
  | "action"
  | "resourceType"
  | "resourceId"
  | "created_at"
  | "created_by"
  | "comment";

interface PolicyColumn {
  key: PolicyColumnKey;
  label: string;
  thProps?: ComponentProps<typeof Table.Th>;
  tdProps?: ComponentProps<typeof Table.Td>;
  render: (policy: Policy) => ReactNode;
}

interface ColumnSettings {
  order: PolicyColumnKey[];
  visible: PolicyColumnKey[];
}

export function PolicyList() {
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams({
    page: "1",
    principal: "",
    action: "",
    resource: "",
  });
  const page = Number(searchParams.get("page")) || 1;

  const config = useConfig();
  const auth = useAuth();

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());

  const [columnsOpened, columnsHandlers] = useDisclosure(false);
  const [columnSettings, setColumnSettings] = useLocalStorage<ColumnSettings>({
    key: "policy-list-columns",
    defaultValue: DEFAULT_COLUMN_SETTINGS,
  });

  const order = useMemo(
    () => reconcileOrder(columnSettings.order),
    [columnSettings.order],
  );
  const visibleKeys = useMemo(
    () => columnSettings.visible.filter(isPolicyColumnKey),
    [columnSettings.visible],
  );
  const visibleSet = useMemo(() => new Set(visibleKeys), [visibleKeys]);
  const displayedColumns = useMemo(
    () => order.filter((key) => visibleSet.has(key)).map((key) => COLUMNS[key]),
    [order, visibleSet],
  );

  const toggleColumn = (key: PolicyColumnKey) => {
    setColumnSettings((prev) => {
      const visible = prev.visible.filter(isPolicyColumnKey);
      const isVisible = visible.includes(key);
      if (isVisible && visible.length === 1) {
        return prev;
      }
      return {
        order: reconcileOrder(prev.order),
        visible: isVisible
          ? visible.filter((k) => k !== key)
          : [...visible, key],
      };
    });
  };

  const moveColumn = (key: PolicyColumnKey, direction: "up" | "down") => {
    setColumnSettings((prev) => {
      const nextOrder = reconcileOrder(prev.order);
      const index = nextOrder.indexOf(key);
      const target = direction === "up" ? index - 1 : index + 1;
      if (target < 0 || target >= nextOrder.length) {
        return prev;
      }
      [nextOrder[index], nextOrder[target]] = [
        nextOrder[target],
        nextOrder[index],
      ];
      return {
        order: nextOrder,
        visible: prev.visible.filter(isPolicyColumnKey),
      };
    });
  };

  const form = useForm<PolicyFilterForm>({
    initialValues: {
      principal:
        searchParams.get("group") || searchParams.get("principal") || "",
      principalType: searchParams.get("group") ? "group" : "user",
      service: searchParams.get("service") ?? "",
      action: searchParams.get("action") ?? "",
      resourceType: searchParams.get("resourceType") ?? "",
      resourceId: searchParams.get("resourceId") ?? "",
      order: searchParams.get("order") ?? "",
    },

    validate: {
      order: (value) => {
        if (value && !/^\d+$/.test(value)) {
          return "Order must be a non-negative integer.";
        }
      },
      service: (value, values) => {
        if (!value && values.action && values.action !== "NULL") {
          return "Service is required if action is specified.";
        }
      },
      resourceType: (value, values) => {
        if (!value && values.resourceId && values.resourceId !== "NULL") {
          return "Resource type is required if resource ID is specified.";
        }
      },
    },
  });

  const { isLoading, isError, error, isSuccess, data, refetch } = useQuery({
    queryKey: ["policies", searchParams.toString()],
    queryFn: async () => {
      return fetchPolicies({
        config,
        user: auth.user!,
        page,
        search: {
          principal:
            form.values.principalType === "group" ? "" : form.values.principal,
          group:
            form.values.principalType === "group" ? form.values.principal : "",
          action: getActionFilter(form.values),
          resource: getResourceFilter(form.values),
          order: form.values.order,
        },
      });
    },
  });

  const [service] = useDebouncedValue(form.values.service, 300);
  const serviceAutocomplete = useServiceAutocomplete();
  const actionsAutocomplete = useServiceActionsAutocomplete({ service });
  const resourceTypeAutocomplete = useServiceResourceTypesAutocomplete({
    service,
  });

  useEffect(() => {
    setSelectedIds(new Set());
  }, [searchParams]);

  const items = isSuccess ? data.items : [];
  const allSelected =
    items.length > 0 && items.every((p) => selectedIds.has(p.id));
  const someSelected = items.some((p) => selectedIds.has(p.id));

  const toggleSelected = (id: number) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(items.map((p) => p.id)));
  };

  const showDeleteModal = () => {
    const selectedPolicies = items.filter((p) => selectedIds.has(p.id));
    const modalId = "delete-policies";
    modals.open({
      modalId,
      title: `Delete ${selectedPolicies.length} ${selectedPolicies.length === 1 ? "policy" : "policies"}`,
      size: "lg",
      closeOnClickOutside: false,
      children: (
        <PolicyDeleteDialog
          policies={selectedPolicies}
          onClose={() => modals.close(modalId)}
          onPoliciesDeleted={(ids) =>
            setSelectedIds((prev) => {
              const next = new Set(prev);
              for (const id of ids) {
                next.delete(id);
              }
              return next;
            })
          }
        />
      ),
    });
  };

  return (
    <Page title={"Policies"}>
      <Title>Policies</Title>

      <Accordion variant={"separated"}>
        <Accordion.Item value={"search"}>
          <Accordion.Control icon={<IconSearch />}>Search</Accordion.Control>
          <Accordion.Panel>
            <form
              method={"GET"}
              onSubmit={form.onSubmit((values) => {
                values.service = values.service.trim();
                values.action = values.action.trim();
                values.resourceType = values.resourceType.trim();
                values.resourceId = values.resourceId.trim();
                values.principal = values.principal.trim();
                values.order = values.order.trim();

                if (values.service === "NULL" && values.action !== "NULL") {
                  values.action = "";
                }
                if (values.action === "NULL" && values.service !== "NULL") {
                  values.service = "";
                }
                if (
                  values.resourceType === "NULL" &&
                  values.resourceId !== "NULL"
                ) {
                  values.resourceId = "";
                }
                if (
                  values.resourceId === "NULL" &&
                  values.resourceType !== "NULL"
                ) {
                  values.resourceType = "";
                }
                const isGroup = values.principalType === "group";
                setSearchParams((prev) => ({
                  ...prev,
                  ...values,
                  principal: isGroup ? "" : values.principal,
                  group: isGroup ? values.principal : "",
                }));
                void refetch();
              })}
            >
              <Group grow align={"stretch"}>
                <Stack justify={"space-between"}>
                  <PrincipalAutocomplete
                    label={"Principal"}
                    placeholder={
                      form.values.principalType === "group"
                        ? "Group ID or name"
                        : "User ID or email"
                    }
                    name={"principal"}
                    unscoped={<PolicyBadge.Principal />}
                    principalType={form.values.principalType}
                    onPrincipalTypeChange={(type) => {
                      form.setFieldValue("principalType", type);
                      form.setFieldValue("principal", "");
                    }}
                    {...form.getInputProps("principal")}
                  />
                  <NumberInput
                    allowDecimal={false}
                    allowNegative={false}
                    label={"Order"}
                    placeholder={"order"}
                    name={"order"}
                    value={
                      form.values.order === "" ? "" : Number(form.values.order)
                    }
                    error={form.errors.order}
                    onChange={(value) =>
                      form.setFieldValue(
                        "order",
                        value === "" ? "" : String(value),
                      )
                    }
                  />
                </Stack>

                <Stack justify={"space-between"}>
                  <FilterInput
                    data={serviceAutocomplete}
                    label={"Service"}
                    placeholder={"service"}
                    name={"service"}
                    unscoped={<PolicyBadge.Service />}
                    {...form.getInputProps("service")}
                  />
                  <FilterInput
                    data={actionsAutocomplete}
                    label={"Action"}
                    placeholder={"action"}
                    name={"action"}
                    unscoped={<PolicyBadge.Service />}
                    {...form.getInputProps("action")}
                  />
                </Stack>

                <Stack justify={"space-between"}>
                  <FilterInput
                    data={resourceTypeAutocomplete}
                    label={"Resource type"}
                    placeholder={"type"}
                    name={"resourceType"}
                    unscoped={<PolicyBadge.Resource />}
                    {...form.getInputProps("resourceType")}
                  />
                  <FilterInput
                    label={"Resource ID"}
                    placeholder={"id"}
                    name={"resourceId"}
                    unscoped={<PolicyBadge.Resource />}
                    {...form.getInputProps("resourceId")}
                  />
                </Stack>
              </Group>

              <Button type={"submit"} mt={"md"} size={"sm"}>
                Find policies
              </Button>
            </form>
          </Accordion.Panel>
        </Accordion.Item>
      </Accordion>

      <Group>
        <NavLink to={"/policies/new"}>
          <Button leftSection={<IconFilePlus />} color={"green"}>
            New policy
          </Button>
        </NavLink>

        <Button
          color={"red"}
          leftSection={<IconTrashFilled />}
          disabled={selectedIds.size === 0}
          onClick={showDeleteModal}
        >
          Delete selected ({selectedIds.size})
        </Button>

        <Popover
          position={"bottom-end"}
          withArrow
          shadow={"md"}
          opened={columnsOpened}
          onChange={columnsHandlers.toggle}
        >
          <Popover.Target>
            <ActionIcon
              variant={"default"}
              size={"lg"}
              aria-label={"Configure columns"}
              ml={"auto"}
              onClick={columnsHandlers.toggle}
            >
              <IconAdjustmentsHorizontal />
            </ActionIcon>
          </Popover.Target>
          <Popover.Dropdown>
            <Stack gap={"xs"}>
              <Text size={"xs"} c={"dimmed"} tt={"uppercase"} fw={600}>
                Columns
              </Text>
              {order.map((key, index) => {
                const column = COLUMNS[key];
                return (
                  <Group
                    key={key}
                    justify={"space-between"}
                    wrap={"nowrap"}
                    gap={"md"}
                  >
                    <Checkbox
                      label={column.label}
                      title={`Toggle ${column.label} column`}
                      checked={visibleSet.has(key)}
                      onChange={() => toggleColumn(key)}
                    />
                    <Group gap={4} wrap={"nowrap"}>
                      <ActionIcon
                        variant={"subtle"}
                        aria-label={`Move ${column.label} up`}
                        title={`Move ${column.label} up`}
                        disabled={index === 0}
                        onClick={() => moveColumn(key, "up")}
                      >
                        <IconChevronUp size={16} />
                      </ActionIcon>
                      <ActionIcon
                        variant={"subtle"}
                        aria-label={`Move ${column.label} down`}
                        title={`Move ${column.label} down`}
                        disabled={index === order.length - 1}
                        onClick={() => moveColumn(key, "down")}
                      >
                        <IconChevronDown size={16} />
                      </ActionIcon>
                    </Group>
                  </Group>
                );
              })}
              <Button
                variant={"subtle"}
                size={"compact-sm"}
                mt={"xs"}
                onClick={() => setColumnSettings(DEFAULT_COLUMN_SETTINGS)}
              >
                Restore defaults
              </Button>
            </Stack>
          </Popover.Dropdown>
        </Popover>
      </Group>

      {isLoading ? (
        <Loader />
      ) : isError ? (
        <Alert color={"red"}>{error.message}</Alert>
      ) : isSuccess ? (
        data.items.length === 0 ? (
          <Stack align={"center"} gap={"xs"} py={"xl"}>
            <ThemeIcon variant={"light"} color={"gray"} size={96} radius={"xl"}>
              <IconSearchOff size={56} />
            </ThemeIcon>
            <Text size={"lg"} fw={500}>
              No results found
            </Text>
            <Text size={"sm"} c={"dimmed"} mb={"xl"}>
              No policies match your search on this page.
            </Text>

            <NavPagination page={page} />
          </Stack>
        ) : (
          <>
            <Table highlightOnHover stickyHeader>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th
                    miw={48}
                    w={48}
                    style={{ cursor: "pointer" }}
                    onClick={toggleSelectAll}
                  >
                    <Checkbox
                      aria-label={"Select all policies"}
                      checked={allSelected}
                      indeterminate={!allSelected && someSelected}
                      readOnly
                    />
                  </Table.Th>
                  {displayedColumns.map((column) => (
                    <Table.Th key={column.key} {...column.thProps}>
                      {column.label}
                    </Table.Th>
                  ))}
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {data.items.map((policy) => (
                  <Table.Tr
                    key={policy.id}
                    style={{ cursor: "pointer" }}
                    onClick={() => {
                      const selection = window.getSelection();
                      const selectedText = selection?.toString();
                      if (!selectedText) {
                        navigate(`/policies/${policy.id}`);
                      }
                    }}
                  >
                    <Table.Td
                      style={{ cursor: "pointer" }}
                      onClick={(event) => {
                        event.stopPropagation();
                        toggleSelected(policy.id);
                      }}
                    >
                      <Checkbox
                        aria-label={`Select policy ${policy.id}`}
                        checked={selectedIds.has(policy.id)}
                        readOnly
                      />
                    </Table.Td>
                    {displayedColumns.map((column) => (
                      <Table.Td key={column.key} {...column.tdProps}>
                        {column.render(policy)}
                      </Table.Td>
                    ))}
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>

            <NavPagination page={page} />
          </>
        )
      ) : null}
    </Page>
  );
}

const POLICY_COLUMNS: PolicyColumn[] = [
  {
    key: "id",
    label: "ID",
    thProps: { miw: 30, w: 30 },
    render: (policy) => (
      <Anchor
        component={NavLink}
        to={`/policies/${policy.id}`}
        onClick={(event) => event.stopPropagation()}
      >
        {policy.id}
      </Anchor>
    ),
  },
  {
    key: "effect",
    label: "Policy",
    thProps: { miw: 100, w: 100 },
    render: (policy) => <PolicyBadge.Effect policy={policy} />,
  },
  {
    key: "order",
    label: "Order",
    render: (policy) => policy.order ?? null,
  },
  {
    key: "principal",
    label: "Principal",
    render: (policy) => <PolicyBadge.Principal policy={policy} />,
  },
  {
    key: "service",
    label: "Service",
    render: (policy) => <PolicyBadge.Service policy={policy} />,
  },
  {
    key: "action",
    label: "Action",
    render: (policy) => policy.action?.name,
  },
  {
    key: "resourceType",
    label: "Resource type",
    thProps: { miw: 140 },
    render: (policy) => <PolicyBadge.Resource policy={policy} />,
  },
  {
    key: "resourceId",
    label: "Resource ID",
    render: (policy) => policy.resource?.id,
  },
  {
    key: "created_at",
    label: "Created at",
    render: (policy) => policy.created_at?.toLocaleString() ?? null,
  },
  {
    key: "created_by",
    label: "Created by",
    render: (policy) => <PrincipalName sub={policy.created_by ?? undefined} />,
  },
  {
    key: "comment",
    label: "Comment",
    tdProps: { maw: 250 },
    render: (policy) =>
      policy.comment ? (
        <Tooltip
          label={policy.comment}
          multiline
          maw={320}
          withArrow
          events={{ hover: true, focus: true, touch: true }}
        >
          <Text lineClamp={1} component={"span"} size={"sm"}>
            {policy.comment}
          </Text>
        </Tooltip>
      ) : null,
  },
];

const COLUMNS: Record<PolicyColumnKey, PolicyColumn> = Object.fromEntries(
  POLICY_COLUMNS.map((column) => [column.key, column]),
) as Record<PolicyColumnKey, PolicyColumn>;

const ALL_COLUMN_KEYS = POLICY_COLUMNS.map((column) => column.key);

const DEFAULT_VISIBLE: PolicyColumnKey[] = [
  "id",
  "effect",
  "principal",
  "service",
  "action",
  "resourceType",
  "resourceId",
  "comment",
];

const DEFAULT_COLUMN_SETTINGS: ColumnSettings = {
  order: ALL_COLUMN_KEYS,
  visible: DEFAULT_VISIBLE,
};

function isPolicyColumnKey(key: string): key is PolicyColumnKey {
  return key in COLUMNS;
}

function reconcileOrder(order: PolicyColumnKey[]): PolicyColumnKey[] {
  const known = order.filter(isPolicyColumnKey);
  const missing = ALL_COLUMN_KEYS.filter((key) => !known.includes(key));
  return [...known, ...missing];
}

function getActionFilter(form: PolicyFilterForm): string {
  if (form.action === "NULL" || form.service === "NULL") {
    return "NULL";
  }
  if (form.service) {
    return `${form.service}:${form.action || "*"}`;
  }
  return "";
}

function getResourceFilter(form: PolicyFilterForm): string {
  if (form.resourceType === "NULL" || form.resourceId === "NULL") {
    return "NULL";
  }
  if (form.resourceType) {
    return `${form.resourceType}::${form.resourceId || "*"}`;
  }
  return "";
}
