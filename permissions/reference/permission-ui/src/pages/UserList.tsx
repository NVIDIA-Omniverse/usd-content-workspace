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
  Alert,
  Button,
  Group,
  Loader,
  TextInput,
  Title,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { IconSearch } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "react-oidc-context";
import { useSearchParams } from "react-router-dom";
import { CursorPagination } from "../components/CursorPagination";
import { Page } from "../components/Page";
import { UserTable } from "../components/UserTable";
import { useConfig } from "../hooks/useConfig";
import { useCursorPagination } from "../hooks/useCursorPagination";
import { fetchUser, fetchUsers, HttpError, type UserPage } from "../state/users";

export function UserList() {
  const config = useConfig();
  const auth = useAuth();
  const pagination = useCursorPagination();
  const [searchParams, setSearchParams] = useSearchParams({
    id: "",
    displayName: "",
    email: "",
  });

  const form = useForm({
    initialValues: {
      id: searchParams.get("id") ?? "",
      displayName: searchParams.get("displayName") ?? "",
      email: searchParams.get("email") ?? "",
    },
  });

  const id = searchParams.get("id") || undefined;
  const displayName = searchParams.get("displayName") || undefined;
  const email = searchParams.get("email") || undefined;

  const { data, isLoading, isError, error, isSuccess } = useQuery({
    queryKey: ["users", id, displayName, email, pagination.cursor],
    queryFn: async (): Promise<UserPage> => {
      if (id) {
        try {
          const found = await fetchUser({
            userId: id,
            config,
            user: auth.user!,
          });
          return { items: [found], next_cursor: null };
        } catch (e) {
          if (e instanceof HttpError && e.status === 404) {
            return { items: [], next_cursor: null };
          }
          throw e;
        }
      }
      return fetchUsers({
        config,
        user: auth.user!,
        displayName,
        email,
        cursor: pagination.cursor,
      });
    },
  });

  return (
    <Page title={"Users"}>
      <Title>Users</Title>

      <Accordion variant={"separated"}>
        <Accordion.Item value={"search"}>
          <Accordion.Control icon={<IconSearch />}>Search</Accordion.Control>
          <Accordion.Panel>
            <form
              method={"GET"}
              onSubmit={form.onSubmit((values) => {
                pagination.reset();
                setSearchParams({
                  id: values.id.trim(),
                  displayName: values.displayName.trim(),
                  email: values.email.trim(),
                });
              })}
            >
              <Group grow align={"stretch"}>
                <TextInput
                  label={"Identifier"}
                  placeholder={"Exact user identifier"}
                  {...form.getInputProps("id")}
                />
                <TextInput
                  label={"Display name"}
                  placeholder={"Full or partial name"}
                  {...form.getInputProps("displayName")}
                />
                <TextInput
                  label={"Email"}
                  placeholder={"Exact email address"}
                  {...form.getInputProps("email")}
                />
              </Group>

              <Button type={"submit"} mt={"md"} size={"sm"}>
                Find users
              </Button>
            </form>
          </Accordion.Panel>
        </Accordion.Item>
      </Accordion>

      {isLoading ? (
        <Loader />
      ) : isError ? (
        <Alert color={"red"}>{error.message}</Alert>
      ) : isSuccess ? (
        <>
          <UserTable users={data.items} />

          <CursorPagination
            hasPrevious={pagination.hasPrevious}
            hasNext={!!data.next_cursor}
            onPrevious={pagination.goPrevious}
            onNext={() => pagination.goNext(data.next_cursor!)}
          />
        </>
      ) : null}
    </Page>
  );
}
