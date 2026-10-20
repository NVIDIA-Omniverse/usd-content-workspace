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
import { CodeHighlight } from "@mantine/code-highlight";
import {
  ActionIcon,
  Alert,
  Anchor,
  Autocomplete,
  Button,
  Checkbox,
  Fieldset,
  Group,
  JsonInput,
  Paper,
  Stack,
  Table,
  Text,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { useDebouncedValue } from "@mantine/hooks";
import { modals } from "@mantine/modals";
import {
  IconBinaryTree,
  IconPlus,
  IconTrash,
  IconTrashFilled,
} from "@tabler/icons-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useAuth } from "react-oidc-context";
import { NavLink } from "react-router-dom";
import { Page } from "../components/Page";
import { PolicyBadge } from "../components/PolicyBadge";
import { PolicyDeleteDialog } from "../components/PolicyDeleteDialog";
import { PrincipalAutocomplete } from "../components/PrincipalAutocomplete";
import { useConfig } from "../hooks/useConfig";
import { useServiceActionsAutocomplete } from "../hooks/useServiceActionsAutocomplete";
import { useServiceAutocomplete } from "../hooks/useServiceAutocomplete";
import { useServiceResourceTypes } from "../hooks/useServiceResourceTypes";
import {
  expandResourceHierarchy,
  fetchDiagnosticsBatch,
  type DiagnosticsBatchFormValues,
  type DiagnosticsBatchOutcome,
  type DiagnosticsBatchResult,
  type DiagnosticsPolicy,
} from "../state/diagnostics";
import type { Policy } from "../state/policies";

export function Diagnostics() {
  const config = useConfig();
  const auth = useAuth();
  const queryClient = useQueryClient();

  const form = useForm<DiagnosticsBatchFormValues>({
    initialValues: {
      principal: "",
      service: "",
      action: "",
      context: "",
      resources: [{ type: "", id: "" }],
    },

    validate: {
      service: (value) => {
        if (!value.trim()) {
          return "Service is required.";
        }
      },
      action: (value) => {
        if (!value.trim()) {
          return "Action is required.";
        }
      },
      context: (value) => {
        if (!value.trim()) return;
        try {
          JSON.parse(value);
        } catch {
          return "Context must be valid JSON.";
        }
      },
      resources: {
        type: (value, values, path) => {
          const index = Number(path.split(".")[1]);
          const id = values.resources[index]?.id?.trim();
          if (id && !value.trim()) {
            return "Resource type is required.";
          }
        },
      },
    },
  });

  const [service] = useDebouncedValue(form.values.service, 300);

  const serviceAutocomplete = useServiceAutocomplete();
  const actionsAutocomplete = useServiceActionsAutocomplete({ service });
  const resourceTypes = useServiceResourceTypes({ service });

  const resourceTypeAutocomplete = useMemo(
    () => resourceTypes.map((resource) => resource.type),
    [resourceTypes],
  );

  const parentTypes = useMemo(() => {
    const map = new Map<string, string>();
    for (const resource of resourceTypes) {
      if (resource.parentType) {
        map.set(resource.type, resource.parentType);
      }
    }
    return map;
  }, [resourceTypes]);

  const [submittedValues, setSubmittedValues] =
    useState<DiagnosticsBatchFormValues | null>(null);

  const { isPending, mutate, data, isError, error } = useMutation<
    DiagnosticsBatchOutcome,
    Error,
    DiagnosticsBatchFormValues
  >({
    mutationFn: async (values) =>
      fetchDiagnosticsBatch({ values, config, user: auth.user!, queryClient }),
  });

  const run = (values: DiagnosticsBatchFormValues) => {
    setSubmittedValues(values);
    mutate(values);
  };

  const expandHierarchy = (index: number) => {
    const row = form.values.resources[index];
    const ancestors = expandResourceHierarchy(
      row.id.trim(),
      row.type,
      parentTypes,
    );
    const existing = new Set(
      form.values.resources.map((resource) => resource.id.trim()),
    );
    let insertAt = index + 1;
    for (const ancestor of ancestors) {
      if (existing.has(ancestor.id)) {
        continue;
      }
      form.insertListItem("resources", ancestor, insertAt);
      existing.add(ancestor.id);
      insertAt++;
    }
  };

  return (
    <Page title="Diagnostics">
      <Title>Diagnostics</Title>
      <Text c="dimmed">
        Specify authorization parameters to see which policies would be
        evaluated and in what order. Add multiple resources to test them in a
        single run.
      </Text>

      <form onSubmit={form.onSubmit(run)}>
        <Stack>
          <Fieldset legend="Authorization parameters">
            <Stack>
              <PrincipalAutocomplete
                description="If not specified, the current user will be used."
                disabled={isPending}
                principalType="user"
                lockPrincipalType
                {...form.getInputProps("principal")}
              />

              <Autocomplete
                data={serviceAutocomplete}
                label="Service"
                disabled={isPending}
                {...form.getInputProps("service")}
              />

              <Autocomplete
                data={actionsAutocomplete}
                label="Action"
                disabled={isPending}
                {...form.getInputProps("action")}
              />

              <JsonInput
                label="Context"
                description="Optional JSON object passed as context to the authorization evaluation."
                disabled={isPending}
                formatOnBlur
                validationError="Context must be valid JSON."
                autosize
                minRows={3}
                {...form.getInputProps("context")}
              />
            </Stack>
          </Fieldset>

          <Fieldset legend="Resources">
            <Stack>
              {form.values.resources.map((row, index) => (
                <Group key={index} align="flex-start" wrap="nowrap">
                  <Autocomplete
                    data={resourceTypeAutocomplete}
                    label={index === 0 ? "Resource type" : undefined}
                    placeholder="type"
                    w={180}
                    disabled={isPending}
                    {...form.getInputProps(`resources.${index}.type`)}
                  />
                  <TextInput
                    label={index === 0 ? "Resource ID" : undefined}
                    placeholder="id"
                    style={{ flex: 1 }}
                    disabled={isPending}
                    {...form.getInputProps(`resources.${index}.id`)}
                  />
                  <Group gap="xs" wrap="nowrap" mt={index === 0 ? 25 : 0}>
                    <Tooltip
                      label="Expand into a resource hierarchy up to the root"
                      withArrow
                    >
                      <ActionIcon
                        variant="default"
                        size="lg"
                        aria-label={`Expand resource ${index + 1} into a hierarchy`}
                        disabled={isPending || !row.id.trim()}
                        onClick={() => expandHierarchy(index)}
                      >
                        <IconBinaryTree />
                      </ActionIcon>
                    </Tooltip>
                    <ActionIcon
                      variant="default"
                      size="lg"
                      color="red"
                      aria-label={`Remove resource ${index + 1}`}
                      disabled={
                        isPending || form.values.resources.length === 1
                      }
                      onClick={() => form.removeListItem("resources", index)}
                    >
                      <IconTrash />
                    </ActionIcon>
                  </Group>
                </Group>
              ))}

              <Button
                variant="light"
                size="sm"
                leftSection={<IconPlus size={16} />}
                disabled={isPending}
                onClick={() =>
                  form.insertListItem("resources", { type: "", id: "" })
                }
                style={{ alignSelf: "flex-start" }}
              >
                Add resource
              </Button>
            </Stack>
          </Fieldset>

          {isError && <Alert color="red">{error.message}</Alert>}

          <Group>
            <Button type="submit" loading={isPending} disabled={isPending}>
              Run diagnostics
            </Button>
            <Button
              type="reset"
              color="gray"
              variant="light"
              onClick={() => form.reset()}
            >
              Reset
            </Button>
          </Group>

          {data?.groupsResolutionFailed && (
            <Alert color="yellow" title="Group membership unavailable">
              Failed to fetch the principal's group membership from the user
              service. Group-scoped policies may be missing from these results.
            </Alert>
          )}

          {data && (
            <DiagnosticsBatchResults
              data={data}
              onRerun={() => submittedValues && mutate(submittedValues)}
            />
          )}
        </Stack>
      </form>
    </Page>
  );
}

interface DiagnosticsBatchResultsProps {
  data: DiagnosticsBatchOutcome;
  onRerun: () => void;
}

function DiagnosticsBatchResults({
  data,
  onRerun,
}: DiagnosticsBatchResultsProps) {
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());

  const policiesById = useMemo(() => {
    const map = new Map<number, DiagnosticsPolicy>();
    for (const result of data.results) {
      for (const actionGroup of result.actions) {
        for (const policy of actionGroup.policies) {
          map.set(policy.id, policy);
        }
      }
    }
    return map;
  }, [data]);

  useEffect(() => {
    setSelectedIds(new Set());
  }, [data]);

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

  const toggleSelectedGroup = (ids: number[]) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      const allSelected = ids.every((id) => next.has(id));
      for (const id of ids) {
        if (allSelected) {
          next.delete(id);
        } else {
          next.add(id);
        }
      }
      return next;
    });
  };

  const showDeleteModal = () => {
    const selectedPolicies = [...selectedIds]
      .map((id) => policiesById.get(id))
      .filter((policy): policy is DiagnosticsPolicy => policy !== undefined)
      .map(toPolicy);

    const modalId = "delete-diagnostics-policies";
    modals.open({
      modalId,
      title: `Delete ${selectedPolicies.length} ${selectedPolicies.length === 1 ? "policy" : "policies"}`,
      size: "lg",
      closeOnClickOutside: false,
      children: (
        <PolicyDeleteDialog
          policies={selectedPolicies}
          onClose={() => modals.close(modalId)}
          onPoliciesDeleted={(ids) => {
            setSelectedIds((prev) => {
              const next = new Set(prev);
              for (const id of ids) {
                next.delete(id);
              }
              return next;
            });
            onRerun();
          }}
        />
      ),
    });
  };

  return (
    <Paper withBorder p="md" style={{ width: "fit-content", minWidth: "100%" }}>
      <Stack>
        <Group justify="space-between">
          <Title order={3}>Results</Title>
          <Button
            color="red"
            leftSection={<IconTrashFilled />}
            disabled={selectedIds.size === 0}
            onClick={showDeleteModal}
          >
            Delete selected ({selectedIds.size})
          </Button>
        </Group>

        {data.results.length === 0 ? (
          <Text c="dimmed">No results for the specified parameters.</Text>
        ) : (
          data.results.map((result, index) => (
            <DiagnosticsResultGroup
              key={index}
              result={result}
              selectedIds={selectedIds}
              onToggle={toggleSelected}
              onToggleGroup={toggleSelectedGroup}
            />
          ))
        )}
      </Stack>
    </Paper>
  );
}

interface DiagnosticsResultGroupProps {
  result: DiagnosticsBatchResult;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleGroup: (ids: number[]) => void;
}

function DiagnosticsResultGroup({
  result,
  selectedIds,
  onToggle,
  onToggleGroup,
}: DiagnosticsResultGroupProps) {
  return (
    <Paper withBorder p="md" radius="md">
      <Stack gap="sm">
        <Stack gap={2}>
          {result.resource ? (
            <>
              <Text
                fw={600}
                size="lg"
                tt="uppercase"
                style={{ wordBreak: "break-all" }}
              >
                {result.resource.type}
              </Text>
              <Text size="sm" style={{ wordBreak: "break-all" }}>
                {result.resource.id}
              </Text>
            </>
          ) : (
            <Text fw={600} size="lg">
              Any resource
            </Text>
          )}
        </Stack>

        <DiagnosticsField label="Evaluation priority">
          {result.evaluation_priority}
        </DiagnosticsField>

        {result.actions.map((actionGroup) => {
          const ids = actionGroup.policies.map((policy) => policy.id);
          const allSelected =
            ids.length > 0 && ids.every((id) => selectedIds.has(id));
          const someSelected = ids.some((id) => selectedIds.has(id));

          return (
            <Stack key={`${actionGroup.service}:${actionGroup.action}`} gap="xs">
              <Group justify="space-between" align="flex-end" wrap="nowrap">
                <DiagnosticsField label="Action">
                  {actionGroup.service}:{actionGroup.action}
                </DiagnosticsField>
                <Text size="sm" c="dimmed">
                  {actionGroup.policies.length}{" "}
                  {actionGroup.policies.length === 1 ? "policy" : "policies"}
                </Text>
              </Group>

              {actionGroup.policies.length === 0 ? (
                <Text c="dimmed" size="sm">
                  No policies match the specified parameters.
                </Text>
              ) : (
                <Table stickyHeader withTableBorder>
                  <Table.Thead>
                    <Table.Tr>
                      <Table.Th
                        w={48}
                        miw={48}
                        style={{ cursor: "pointer" }}
                        onClick={() => onToggleGroup(ids)}
                      >
                        <Checkbox
                          aria-label="Select all policies"
                          checked={allSelected}
                          indeterminate={!allSelected && someSelected}
                          readOnly
                        />
                      </Table.Th>
                      <Table.Th w={80}>Order</Table.Th>
                      <Table.Th w={80}>ID</Table.Th>
                      <Table.Th>Principal</Table.Th>
                      <Table.Th>Policy</Table.Th>
                    </Table.Tr>
                  </Table.Thead>
                  <Table.Tbody>
                    {actionGroup.policies.map((policy) => (
                      <Table.Tr key={policy.id}>
                        <Table.Td
                          style={{ cursor: "pointer" }}
                          onClick={() => onToggle(policy.id)}
                        >
                          <Checkbox
                            aria-label={`Select policy ${policy.id}`}
                            checked={selectedIds.has(policy.id)}
                            readOnly
                          />
                        </Table.Td>
                        <Table.Td>{policy.order}</Table.Td>
                        <Table.Td>
                          <Anchor
                            component={NavLink}
                            to={`/policies/${policy.id}`}
                          >
                            {policy.id}
                          </Anchor>
                        </Table.Td>
                        <Table.Td>
                          <PolicyBadge.Principal policy={toPolicy(policy)} />
                        </Table.Td>
                        <Table.Td>
                          <CodeHighlight
                            code={policy.policy}
                            language="cedar"
                            withBorder
                          />
                        </Table.Td>
                      </Table.Tr>
                    ))}
                  </Table.Tbody>
                </Table>
              )}
            </Stack>
          );
        })}
      </Stack>
    </Paper>
  );
}

function DiagnosticsField({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <Stack gap={2}>
      <Text size="xs" c="dimmed" tt="uppercase" fw={600}>
        {label}
      </Text>
      <Text size="sm" component="div" style={{ wordBreak: "break-all" }}>
        {children}
      </Text>
    </Stack>
  );
}

function toPolicy(policy: DiagnosticsPolicy): Policy {
  return {
    id: policy.id,
    policy: policy.policy,
    principal: policy.principal ?? null,
    group: policy.group ?? null,
    action: policy.action ?? null,
    resource: policy.resource ?? null,
    order: policy.order,
    created_at: null,
    created_by: null,
    comment: null,
  };
}
