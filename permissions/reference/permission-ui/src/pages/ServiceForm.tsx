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
  ActionIcon,
  Alert,
  Button,
  Fieldset,
  Group,
  Select,
  Stack,
  Table,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { notifications } from "@mantine/notifications";
import { IconCheck, IconQuestionMark } from "@tabler/icons-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { Navigate } from "react-router-dom";
import { List } from "../components/List";
import { Page } from "../components/Page";
import { useConfig } from "../hooks/useConfig";
import {
  type ResourceInfo,
  type ServiceInfo,
  putService,
} from "../state/metadata";

export interface ServiceFormProps {
  initialValues?: ServiceInfo;
  title?: string;
}

export function ServiceForm({
  initialValues = {
    service: "",
    idClaim: "sub",
    actions: [],
    resources: [],
  },
  title = "Register service",
}: ServiceFormProps) {
  const config = useConfig();
  const auth = useAuth();
  const queryClient = useQueryClient();

  const form = useForm<ServiceInfo>({
    initialValues,
    validate: {
      service: (value) => {
        if (!value) {
          return "Service name is required.";
        }

        if (!value.match(/^[a-zA-Z0-9-_]+$/)) {
          return "Service name must contain only alphanumeric characters, hyphens and underscores.";
        }
      },

      idClaim: (value) => {
        if (!value) {
          return "Principal identifier is required.";
        }
      },

      actions: (value) => {
        for (const action of value) {
          if (!action.match(/^[a-z0-9-_]+$/)) {
            return "Action name must contain only lowercased alphanumeric characters, hyphens and underscores.";
          }
        }
      },
      resources: (value) => {
        for (const resource of value) {
          if (!resource.type.match(/^[a-zA-Z0-9]+$/)) {
            return "Resource type name must contain only alphanumeric characters.";
          }
        }
      },
    },
  });

  const { isPending, mutate, isError, error, isSuccess } = useMutation<
    void,
    Error,
    ServiceInfo
  >({
    mutationKey: ["service", form.values.service],
    mutationFn: async (values) => {
      return putService({ values, config, user: auth.user! });
    },
    onSuccess: () => {
      notifications.show({
        title: `Service ${form.values.service} has been updated.`,
        message: "",
        icon: <IconCheck />,
        color: "green",
      });

      void queryClient.invalidateQueries({
        queryKey: ["service", form.values.service],
      });
      void queryClient.invalidateQueries({
        queryKey: ["service-meta", form.values.service],
      });
      void queryClient.invalidateQueries({
        queryKey: ["service-actions", form.values.service],
      });
      void queryClient.invalidateQueries({
        queryKey: ["service-resource-types", form.values.service],
      });
    },
  });

  const renderResourceHeaders = useCallback(() => {
    return (
      <Table.Thead>
        <Table.Tr>
          <Table.Th fz={"xs"}>Resource type</Table.Th>
          <Table.Th fz={"xs"} w={160} miw={160}>
            <Group gap={"xs"} wrap={"nowrap"}>
              Evaluation priority
              <Tooltip
                label={
                  "Defines what decision must be prioritized when authorization has conflicting policies."
                }
                withArrow
              >
                <ActionIcon
                  size={12}
                  radius={"xl"}
                  title="Evaluation priority help"
                >
                  <IconQuestionMark size={8} />
                </ActionIcon>
              </Tooltip>
            </Group>
          </Table.Th>
        </Table.Tr>
      </Table.Thead>
    );
  }, []);

  const renderResource = useCallback(
    (resource: string, index: number) => {
      return (
        <>
          <Table.Td>{resource}</Table.Td>
          <Table.Td>
            <Select
              allowDeselect={false}
              checkIconPosition={"right"}
              clearable={false}
              data={["forbid", "permit"]}
              title={`${resource} evaluation priority`}
              {...form.getInputProps(`resources.${index}.priority`)}
            />
          </Table.Td>
        </>
      );
    },
    [form]
  );

  if (isSuccess) {
    return <Navigate to={`/services`} />;
  }

  return (
    <Page title={title}>
      <Title>{title}</Title>

      <form onSubmit={form.onSubmit((values) => mutate(values))}>
        <Stack>
          <Fieldset legend={"Service"}>
            <Stack>
              <TextInput
                label={"Name"}
                disabled={isPending}
                {...form.getInputProps("service")}
              />

              <TextInput
                label={"Principal identifier"}
                description={
                  <>
                    Defines what field from an access token is used for
                    identifying the user. <br />
                    <strong>sub</strong> field is used by default and represents
                    random unique identifier assigned to each user or service.
                  </>
                }
                disabled={isPending}
                {...form.getInputProps("idClaim")}
              />
            </Stack>
          </Fieldset>

          <Fieldset legend={"Actions"}>
            <List
              description={"Defines operations the service supports."}
              disabled={isPending}
              inputPlaceholder={"New action name..."}
              label={"Actions"}
              {...form.getInputProps("actions")}
            />
          </Fieldset>

          <Fieldset legend={"Resources"}>
            <List
              label={"Resources"}
              disabled={isPending}
              description={
                <>
                  Defines resource types service operations use.
                  <br />
                  The recommendation is to name all resource types in
                  PascalCase.
                </>
              }
              inputPlaceholder={"New resource type..."}
              renderHeaders={renderResourceHeaders}
              renderItem={renderResource}
              value={form
                .getValues()
                .resources.map((resource) => resource.type)}
              error={form.errors.resources}
              onAdd={(resourceType) =>
                form.insertListItem("resources", {
                  type: resourceType,
                  priority: "forbid",
                } satisfies ResourceInfo)
              }
              onRemove={(_, index) => form.removeListItem("resources", index)}
            />
          </Fieldset>

          {isError && <Alert color={"red"}>{error.message}</Alert>}

          <Group>
            <Button type={"submit"} disabled={isPending} loading={isPending}>
              Submit
            </Button>
          </Group>
        </Stack>
      </form>
    </Page>
  );
}
