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
  Alert,
  Anchor,
  Autocomplete,
  Breadcrumbs,
  Button,
  Fieldset,
  Group,
  NumberInput,
  Select,
  Stack,
  Text,
  Textarea,
  TextInput,
  Title,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { useDebouncedValue } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { IconAlertTriangleFilled, IconCheck } from "@tabler/icons-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import dedent from "dedent";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { Navigate, NavLink } from "react-router-dom";
import { z } from "zod/v4";
import { Page } from "../components/Page";
import {
  PrincipalAutocomplete,
  type PrincipalType,
} from "../components/PrincipalAutocomplete";
import {
  RESOURCE_OPERATOR_SECTION_WIDTH,
  ResourceOperatorSelector,
  type ResourceOperator,
} from "../components/ResourceOperatorSelector";
import { useConfig } from "../hooks/useConfig";
import { useServiceActionsAutocomplete } from "../hooks/useServiceActionsAutocomplete";
import { useServiceAutocomplete } from "../hooks/useServiceAutocomplete";
import { useServiceResourceTypesAutocomplete } from "../hooks/useServiceResourceTypesAutocomplete";
import { fetchServiceMeta, type ServiceMeta } from "../state/metadata";
import { deletePolicy, type Policy, putPolicy } from "../state/policies";
import { encodeResourceId } from "../util/encoding";

export interface PolicyFormProps {
  initial?: Policy;
}

export interface PolicyFormValues {
  scopes?: never;
  service: string;
  action: string;
  principal: string;
  principalType: PrincipalType;
  resourceType: string;
  resourceId: string;
  resourceOperator: ResourceOperator;
  effect: "Permit" | "Forbid";
  clause?: "None" | "When" | "Unless";
  conditions: string;
  order: number | "";
  comment: string;
}

const COMMENT_MAX_LENGTH = 4096;

export function PolicyForm({ initial }: PolicyFormProps) {
  const config = useConfig();
  const auth = useAuth();
  const queryClient = useQueryClient();
  const [principalResolved, setPrincipalResolved] = useState(false);
  const hasUserService = !!config.endpoints.userService;

  const form = useForm<PolicyFormValues>({
    initialValues: {
      service: initial?.action?.service ?? "",
      action: initial?.action?.name ?? "",
      principal: initial?.group?.id ?? initial?.principal?.sub ?? "",
      principalType: initial?.group ? "group" : "user",
      resourceType:
        initial?.resource?.type ?? initial?.resource_prefix?.type ?? "",
      resourceId: initial?.resource?.id ?? initial?.resource_prefix?.id ?? "",
      resourceOperator: initial?.resource_prefix ? "in" : "==",
      effect: initial?.policy ? parsePolicyEffect(initial.policy) : "Permit",
      clause: initial?.policy ? parsePolicyClause(initial.policy) : "None",
      conditions: initial?.policy ? parsePolicyConditions(initial.policy) : "",
      order: initial?.order ?? "",
      comment: initial?.comment ?? "",
    },

    validate: {
      scopes: (_, values) => {
        if (
          !values.principal &&
          !values.service &&
          !values.action &&
          !values.resourceType &&
          !values.resourceId &&
          !values.conditions
        ) {
          return "At least one scope or condition must be specified.";
        }
      },
      service: (value, values) => {
        if (value && !value.match(/^[a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*$/)) {
          return "Service name must only contain alphanumeric characters and dashes.";
        }
        if (!value && values.action) {
          return "Service is required if action is specified.";
        }
      },
      action: (value, values) => {
        if (value && !value.match(/^[a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*$/)) {
          return "Action must only contain alphanumeric characters and dashes.";
        }
        if (!value && values.service) {
          return "Action is required if service is specified.";
        }
      },
      resourceType: (value, values) => {
        if (value && !value.match(/^[a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*$/)) {
          return "Resource type must only contain alphanumeric characters and dashes.";
        }
        if (!value && values.resourceId) {
          return "Resource type is required if resource ID is specified.";
        }
      },
      resourceId: (value, values) => {
        if (value && value.includes('"')) {
          return "Resource ID must not contain quotes.";
        }
        if (!value && values.resourceType) {
          return "Resource ID is required if resource type is specified.";
        }
      },
      conditions: (value, values) => {
        if (!value && values.clause && values.clause !== "None") {
          return "Conditions must be specified.";
        }
      },
      order: (value) => {
        if (value === "") return;
        if (!Number.isInteger(value) || value < 0) {
          return "Order must be a non-negative integer.";
        }
      },
      comment: (value) => {
        if (value.length > COMMENT_MAX_LENGTH) {
          return `Comment must be at most ${COMMENT_MAX_LENGTH} characters.`;
        }
      },
    },
  });

  const { isPending, mutate, data, isError, error } = useMutation<
    Policy,
    Error,
    typeof form.values
  >({
    mutationFn: async (values) => {
      values.service = values.service.trim();
      values.action = values.action.trim();
      values.principal = values.principal.trim();
      values.resourceType = values.resourceType.trim();
      values.resourceId = values.resourceId.trim();
      values.conditions = values.conditions.trim();
      values.comment = values.comment.trim();

      // Service-side deduplication makes PUT return the existing row's id
      // whenever the canonical policy hash matches, while still refreshing
      // mutable metadata (order, comment, audit fields) on that row. When
      // editing, only delete the original if PUT created a different policy;
      // otherwise the "old" id is the same row we just upserted.
      const newPolicy = await putPolicy({
        policy,
        values,
        config,
        user: auth.user!,
      });
      if (initial && newPolicy.id !== initial.id) {
        await deletePolicy({ policyId: initial.id, config, user: auth.user! });
      }
      return newPolicy;
    },
    onSuccess: (data) => {
      let title: string;
      if (!initial) {
        title = `Policy #${data.id} has been created.`;
      } else if (data.id === initial.id) {
        title = `Policy #${initial.id} has been updated.`;
      } else {
        title = `Policy #${initial.id} has been replaced with new policy #${data.id}`;
      }

      void queryClient.invalidateQueries({
        queryKey: ["policies"],
      });

      notifications.show({
        title,
        message: "",
        icon: <IconCheck />,
        color: "green",
      });
    },
  });

  const [service] = useDebouncedValue(form.values.service, 300);

  const serviceAutocomplete = useServiceAutocomplete();
  const actionsAutocomplete = useServiceActionsAutocomplete({
    service: service,
  });
  const resourceTypeAutocomplete = useServiceResourceTypesAutocomplete({
    service: service,
  });

  const principalMeta = useQuery({
    queryKey: ["service-meta", service],
    queryFn: async () => {
      return fetchServiceMeta({
        service,
        config,
        user: auth.user!,
      });
    },
    enabled: !!service,
  });

  const policy = constructPolicy(form.values);
  const policyPreview = constructPolicy(form.values, {
    encodeResourceId: true,
  });

  if (data) {
    return <Navigate to={`/policies/${data.id}`} />;
  }

  const title = initial ? `Policy #${initial.id}` : "New policy";
  return (
    <Page title={title}>
      <Title>{title}</Title>
      <Breadcrumbs>
        <Anchor component={NavLink} to={"/policies/"}>
          Policies
        </Anchor>
        {initial && (
          <Anchor component={NavLink} to={`/policies/${initial.id}`}>
            #{initial.id}
          </Anchor>
        )}
      </Breadcrumbs>

      <form onSubmit={form.onSubmit((values) => mutate(values))}>
        <Stack>
          <Fieldset legend={"Scopes"}>
            <Stack>
              <Autocomplete
                data={serviceAutocomplete}
                disabled={isPending}
                label={"Service"}
                {...form.getInputProps("service")}
              />

              <Autocomplete
                data={actionsAutocomplete}
                disabled={isPending}
                label={"Action"}
                {...form.getInputProps("action")}
              />

              <Autocomplete
                data={resourceTypeAutocomplete}
                description={
                  "The recommendation is to name all resource types in PascalCase."
                }
                disabled={isPending}
                label={"Resource type"}
                {...form.getInputProps("resourceType")}
              />

              <TextInput
                label={"Resource ID"}
                disabled={isPending}
                leftSection={
                  <ResourceOperatorSelector
                    value={form.values.resourceOperator}
                    disabled={isPending}
                    onChange={(operator) =>
                      form.setFieldValue("resourceOperator", operator)
                    }
                  />
                }
                leftSectionWidth={RESOURCE_OPERATOR_SECTION_WIDTH}
                leftSectionPointerEvents={"all"}
                {...form.getInputProps("resourceId")}
              />

              {URL.canParse(form.values.resourceId) && (
                <Alert
                  color={"orange"}
                  title={"Warning"}
                  icon={<IconAlertTriangleFilled />}
                >
                  This resource ID is a URL -- each path segment will be
                  percent-encoded.
                </Alert>
              )}

              <PrincipalAutocomplete
                idClaim={
                  form.values.principalType === "user"
                    ? principalMeta.data?.id_claim
                    : undefined
                }
                description={
                  form.values.principalType === "user" ? (
                    <>
                      The <strong>{form.values.service}</strong> service uses{" "}
                      <strong>{principalMeta.data?.id_claim ?? "sub"}</strong>{" "}
                      field as the user identifier.
                    </>
                  ) : (
                    <>
                      The policy will apply to all members of the selected group.
                    </>
                  )
                }
                disabled={isPending}
                principalType={form.values.principalType}
                onPrincipalTypeChange={(type) => {
                  form.setFieldValue("principalType", type);
                  form.setFieldValue("principal", "");
                  setPrincipalResolved(false);
                }}
                onResolvedChange={setPrincipalResolved}
                {...form.getInputProps("principal")}
              />

              {form.values.principalType === "user" &&
                form.values.principal &&
                isEmailRequired(principalMeta.data) &&
                !isEmail(form.values.principal) && (
                  <Alert
                    color={"orange"}
                    title={"Warning"}
                    icon={<IconAlertTriangleFilled />}
                  >
                    Principal for <strong>{form.values.service}</strong> service
                    should be a valid email.
                    <br />
                    Use the current value only to attach policies to service
                    accounts.
                  </Alert>
                )}

              {form.values.principalType === "user" &&
                form.values.principal &&
                hasUserService &&
                !principalResolved &&
                !isEmailRequired(principalMeta.data) && (
                  <Alert
                    color={"orange"}
                    title={"Warning"}
                    icon={<IconAlertTriangleFilled />}
                  >
                    The entered principal could not be resolved to a known user.
                    The value will be used as-is in the policy.
                  </Alert>
                )}

              {form.values.principalType === "group" &&
                form.values.principal &&
                hasUserService &&
                !principalResolved && (
                  <Alert
                    color={"orange"}
                    title={"Warning"}
                    icon={<IconAlertTriangleFilled />}
                  >
                    The entered group could not be resolved to a known group.
                    The value will be used as-is in the policy.
                  </Alert>
                )}
            </Stack>
          </Fieldset>

          <Fieldset legend={"Policy"}>
            <Stack>
              <Select
                allowDeselect={false}
                checkIconPosition={"right"}
                clearable={false}
                data={["Permit", "Forbid"]}
                description={
                  "Defines if the action needs to be allowed or denied."
                }
                disabled={isPending}
                label={"Effect"}
                {...form.getInputProps("effect")}
              />

              <Select
                allowDeselect={false}
                checkIconPosition={"right"}
                clearable={false}
                data={["None", "When", "Unless"]}
                description={
                  <>
                    Defines conditions when this policy is applied in addition
                    to policy scopes.
                    <br />
                    If not specified, only policy scopes from above are used.
                  </>
                }
                disabled={isPending}
                label={"Conditions (advanced)"}
                {...form.getInputProps("clause")}
              />

              {form.values.clause && form.values.clause !== "None" && (
                <Textarea
                  disabled={isPending}
                  description={
                    <>
                      See{" "}
                      <Anchor
                        size={"xs"}
                        href={
                          "https://docs.cedarpolicy.com/policies/syntax-policy.html#term-parc-context"
                        }
                        target={"_blank"}
                      >
                        this guide
                      </Anchor>{" "}
                      to understand how to write Cedar conditions.
                    </>
                  }
                  {...form.getInputProps("conditions")}
                />
              )}

              <NumberInput
                allowDecimal={false}
                allowNegative={false}
                description={
                  "Policies with lower order are evaluated first. Leave empty to let the service assign order automatically."
                }
                disabled={isPending}
                label={"Order"}
                placeholder={"Auto"}
                {...form.getInputProps("order")}
              />

              <Textarea
                autosize
                minRows={2}
                maxLength={COMMENT_MAX_LENGTH}
                disabled={isPending}
                description={
                  "Optional administrator note. Stored as metadata only and not used during policy evaluation."
                }
                label={"Comment"}
                {...form.getInputProps("comment")}
              />

              <div>
                <Text fw={"500"} size={"sm"} id="policy-preview-label">
                  Preview
                </Text>
                <CodeHighlight
                  code={dedent(policyPreview)}
                  language={"cedar"}
                  withBorder
                  aria-labelledby="policy-preview-label"
                />
              </div>
            </Stack>
          </Fieldset>

          {!form.values.principal && (
            <Alert
              color={"orange"}
              title={"Warning"}
              icon={<IconAlertTriangleFilled />}
            >
              This policy does not include a principal scope -- it will be
              applied for all users.
            </Alert>
          )}

          {!form.values.service && !form.values.action && (
            <Alert
              color={"orange"}
              title={"Warning"}
              icon={<IconAlertTriangleFilled />}
            >
              This policy does not include a service scope -- it will be applied
              for all operations for all services.
            </Alert>
          )}

          {form.errors.scopes && (
            <Alert color={"red"}>{form.errors.scopes}</Alert>
          )}

          {isError && <Alert color={"red"}>{error.message}</Alert>}

          <Group>
            <Button type={"submit"} loading={isPending} disabled={isPending}>
              Submit
            </Button>
            <Button
              type={"reset"}
              color={"gray"}
              variant={"light"}
              onClick={() => form.reset()}
            >
              Reset
            </Button>
          </Group>
        </Stack>
      </form>
    </Page>
  );
}

function constructPolicy(
  values: PolicyFormValues,
  opts?: { encodeResourceId?: boolean },
): string {
  const conditions = ` ${values.clause?.toLowerCase()} {
    ${values.conditions.trim()}
  }`;

  const resourceId = opts?.encodeResourceId
    ? encodeResourceId(values.resourceId.trim())
    : values.resourceId.trim();

  const principalScope = values.principal
    ? values.principalType === "group"
      ? ` in Group::"${values.principal.trim()}"`
      : ` == Principal::"${values.principal.trim()}"`
    : "";

  return `${values.effect.toLowerCase()}(
    principal${principalScope},
    action${values.action ? ` == Action::"${values.service.trim()}:${values.action.trim()}"` : ""},
    resource${values.resourceType ? ` ${values.resourceOperator} ${values.resourceType.trim()}::"${resourceId}"` : ""}
  )${values.clause && values.clause !== "None" ? conditions : ""};`;
}

const policyRegex =
  /(?<effect>permit|forbid)\s*\((?<scopes>[\s\S]*?)\)(\s*(?<clause>when|unless)\s*\{(?<conditions>[\s\S]*?)})?\s*;/;

function parsePolicyEffect(policy: string): "Permit" | "Forbid" {
  const regex = new RegExp(policyRegex);
  const result = regex.exec(policy);
  switch (result?.groups?.effect) {
    case "permit":
      return "Permit";
    case "forbid":
      return "Forbid";
    default:
      return "Permit";
  }
}

function parsePolicyClause(policy: string): "None" | "When" | "Unless" {
  const regex = new RegExp(policyRegex);
  const result = regex.exec(policy);
  switch (result?.groups?.clause) {
    case "when":
      return "When";
    case "unless":
      return "Unless";
    default:
      return "None";
  }
}

function parsePolicyConditions(policy: string): string {
  const regex = new RegExp(policyRegex);
  const result = regex.exec(policy);
  return result?.groups?.conditions?.trim() ?? "";
}

function isEmailRequired(meta?: ServiceMeta): boolean {
  return meta?.id_claim === "email";
}

function isEmail(value: string): boolean {
  const schema = z.email();
  const result = schema.safeParse(value);
  return result.success;
}
