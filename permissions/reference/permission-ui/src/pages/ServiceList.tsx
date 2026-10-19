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
  Anchor,
  Button,
  Group,
  Loader,
  Stack,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { modals } from "@mantine/modals";
import { IconServerSpark, IconTrashFilled } from "@tabler/icons-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { NavLink, useNavigate } from "react-router-dom";
import { Page } from "../components/Page";
import { useConfig } from "../hooks/useConfig";
import { deleteService, fetchServices } from "../state/metadata";

export function ServiceList() {
  const navigate = useNavigate();
  const config = useConfig();
  const auth = useAuth();
  const queryClient = useQueryClient();

  const { isLoading, isError, error, isSuccess, data } = useQuery({
    queryKey: ["services"],
    queryFn: async () => {
      return fetchServices({ config, user: auth.user! });
    },
  });

  const { mutate: confirmDelete } = useMutation({
    mutationFn: async (service: string) => {
      return deleteService({ service, config, user: auth.user! });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["services"],
      });
    },
  });

  const showDeleteModal = (service: string) => {
    modals.openConfirmModal({
      title: `Delete ${service}`,
      children: (
        <Stack gap={"xs"}>
          <Text size={"sm"}>
            Are you sure you want to delete service <strong>{service}</strong>?
            This deletes all registered metadata for this service such as known
            actions and resource types.
          </Text>
          <Text size={"sm"}>This does not delete existing policies.</Text>
          <Text size={"sm"}>This action cannot be undone.</Text>
        </Stack>
      ),
      confirmProps: { color: "red" },
      labels: { confirm: "Delete", cancel: "Cancel" },
      size: "lg",
      onConfirm: () => confirmDelete(service),
    });
  };

  return (
    <Page title={"Services"}>
      <Title>Services</Title>

      <Group>
        <NavLink to={"/services/new"}>
          <Button leftSection={<IconServerSpark />} color={"green"}>
            Register service
          </Button>
        </NavLink>
      </Group>

      {isLoading ? (
        <Loader />
      ) : isError ? (
        <Alert color={"red"}>{error.message}</Alert>
      ) : isSuccess ? (
        <Table highlightOnHover stickyHeader>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Service</Table.Th>
              <Table.Th w={30} miw={30}></Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {data.map((service) => (
              <Table.Tr
                key={service.service}
                style={{ cursor: "pointer" }}
                onClick={() => navigate(`/services/${service.service}`)}
              >
                <Table.Td>
                  <Anchor
                    component={NavLink}
                    to={`/services/${service.service}`}
                  >
                    {service.service}
                  </Anchor>
                </Table.Td>

                <Table.Td>
                  <ActionIcon
                    color={"red"}
                    title={`Delete ${service.service}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      showDeleteModal(service.service);
                    }}
                  >
                    <IconTrashFilled />
                  </ActionIcon>
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      ) : null}
    </Page>
  );
}
