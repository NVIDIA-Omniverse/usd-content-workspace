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
  Breadcrumbs,
  Button,
  Group,
  Loader,
  Table,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { modals } from "@mantine/modals";
import { notifications } from "@mantine/notifications";
import { IconPencil, IconTrashFilled } from "@tabler/icons-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import dedent from "dedent";
import { useAuth } from "react-oidc-context";
import { Navigate, NavLink, useNavigate, useParams } from "react-router-dom";
import { Page } from "../components/Page";
import { PolicyBadge } from "../components/PolicyBadge";
import { useConfig } from "../hooks/useConfig";
import { usePrincipalName } from "../hooks/usePrincipalName";
import { deletePolicy, fetchPolicy } from "../state/policies";

export function PolicyDetails() {
  const config = useConfig();
  const auth = useAuth();
  const params = useParams<{ policyId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const { isLoading, isError, error, isSuccess, data } = useQuery({
    queryKey: ["policy-details", params.policyId],
    queryFn: async () => {
      return fetchPolicy({
        policyId: params.policyId!,
        config,
        user: auth.user!,
      });
    },
    enabled: !!params.policyId,
  });

  const createdByName = usePrincipalName(data?.created_by ?? undefined);

  const { mutate: confirmDelete } = useMutation({
    mutationFn: async () => {
      return deletePolicy({
        policyId: params.policyId!,
        config,
        user: auth.user!,
      });
    },
    onSuccess: () => {
      notifications.show({
        title: `Policy #${params.policyId} has been deleted.`,
        message: "",
        icon: <IconTrashFilled />,
        color: "green",
      });

      void queryClient.invalidateQueries({
        queryKey: ["policies"],
      });

      navigate("/policies/");
    },
    onError: (error) => {
      notifications.show({
        title: `Failed to delete policy #${params.policyId}.`,
        message: error.message,
        color: "red",
      });
    },
  });

  if (!params.policyId) {
    return <Navigate to={"/policies/"} />;
  }

  const showDeleteModal = () => {
    modals.openConfirmModal({
      title: `Delete policy #${params.policyId}`,
      children: (
        <Text size={"sm"}>
          Are you sure you want to delete policy #{params.policyId}? <br />
          This action cannot be undone.
        </Text>
      ),
      confirmProps: { color: "red" },
      labels: { confirm: "Delete", cancel: "Cancel" },
      onConfirm: confirmDelete,
    });
  };

  return (
    <Page title={"Policy details"}>
      <Title>Policy #{params.policyId}</Title>
      <Breadcrumbs>
        <Anchor component={NavLink} to={"/policies/"}>
          Policies
        </Anchor>
        <Anchor component={NavLink} to={`/policies/${params.policyId}`}>
          #{params.policyId}
        </Anchor>
      </Breadcrumbs>

      {isLoading ? (
        <Loader />
      ) : isError ? (
        <Alert color={"red"}>{error.message}</Alert>
      ) : isSuccess ? (
        <>
          <Group>
            <Button
              component={NavLink}
              color={"blue"}
              leftSection={<IconPencil />}
              to={`/policies/${params.policyId}/edit`}
            >
              Edit
            </Button>

            <Button
              color={"red"}
              leftSection={<IconTrashFilled />}
              onClick={showDeleteModal}
            >
              Delete
            </Button>
          </Group>

          <Table variant={"vertical"} withTableBorder>
            <Table.Tbody>
              <Table.Tr>
                <Table.Th w={150}>Effect</Table.Th>
                <Table.Td>
                  <PolicyBadge.Effect policy={data} />
                </Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Principal</Table.Th>
                <Table.Td>
                  <PolicyBadge.Principal policy={data} />
                </Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Service</Table.Th>
                <Table.Td>
                  <PolicyBadge.Service policy={data} />
                </Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Action</Table.Th>
                <Table.Td>{data.action?.name}</Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Resource type</Table.Th>
                <Table.Td>
                  <PolicyBadge.Resource policy={data} />
                </Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Resource id</Table.Th>
                <Table.Td>{data.resource?.id}</Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Order</Table.Th>
                <Table.Td>
                  {data.order != null ? data.order : "Auto"}
                </Table.Td>
              </Table.Tr>
              <Table.Tr>
                <Table.Th>Comment</Table.Th>
                <Table.Td style={{ whiteSpace: "pre-wrap" }}>
                  {data.comment ? data.comment : "\u2014"}
                </Table.Td>
              </Table.Tr>
              {data.created_by && (
                <Table.Tr>
                  <Table.Th>Created by</Table.Th>
                  <Table.Td>
                    {createdByName.displayName &&
                    createdByName.displayName !== data.created_by ? (
                      <Tooltip label={data.created_by} withArrow>
                        <span>{createdByName.displayName}</span>
                      </Tooltip>
                    ) : createdByName.isLoading ? (
                      <Loader size="xs" />
                    ) : (
                      data.created_by
                    )}
                  </Table.Td>
                </Table.Tr>
              )}
              {data.created_at && (
                <Table.Tr>
                  <Table.Th>Created at</Table.Th>
                  <Table.Td>
                    {data.created_at.toLocaleString()}
                  </Table.Td>
                </Table.Tr>
              )}
            </Table.Tbody>
          </Table>

          <label>
            <Text fw={500} size={"sm"}>
              Policy text:
            </Text>
            <CodeHighlight
              code={dedent(data?.policy)}
              language={"cedar"}
              withBorder
            />
          </label>
        </>
      ) : null}
    </Page>
  );
}
