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
  Button,
  Fieldset,
  Group,
  Select,
  Stack,
  TextInput,
} from "@mantine/core";
import { useForm } from "@mantine/form";
import { useState } from "react";
import {
  PermissionForm,
  type PermissionFormProps,
} from "../components/PermissionForm.js";

import type { ResourceAddress } from "../types/ResourceAddress";

export function Example() {
  const params = new URLSearchParams(window.location.search);

  const form = useForm({
    initialValues: {
      server: params.get("server") ?? "",
      userInfoServer: params.get("userInfoServer") ?? "",
      accessToken: "",
      idToken: "",
      resourceType: params.get("resourceType") ?? "object",
      resourceAddress: params.get("resourceAddress") ?? "",
    },
    validateInputOnBlur: true,
    validate: {
      server: (value) => validateUrl(value, { required: true }),
      userInfoServer: (value) => validateUrl(value, { required: false }),
      resourceAddress: (value) => validateUrl(value, { required: true }),
    },
  });

  const [props, setProps] = useState<PermissionFormProps | null>(null);
  if (props) {
    return (
      <Stack p={"md"} h={"100vh"}>
        <PermissionForm {...props} />

        <Group mt={"auto"}>
          <Button color={"cyan"} onClick={() => setProps(null)}>
            Back
          </Button>
        </Group>
      </Stack>
    );
  }

  return (
    <Stack p={"md"}>
      <form
        onSubmit={form.onSubmit((values) => {
          const url = new URL(window.location.toString());
          url.searchParams.set("server", values.server);
          url.searchParams.set("userInfoServer", values.userInfoServer);
          url.searchParams.set("resourceType", values.resourceType);
          url.searchParams.set("resourceAddress", values.resourceAddress);
          history.pushState({}, "", url);

          setProps({
            auth: {
              accessToken: values.accessToken,
              idToken: values.idToken,
            },
            config: {
              url: values.server,
              userInfoUrl: values.userInfoServer || undefined,
            },
            resource: {
              type: values.resourceType as ResourceAddress["type"],
              address: values.resourceAddress,
            },
          });
        })}
      >
        <Fieldset legend={"Configuration"}>
          <Stack>
            <TextInput
              label={"Server"}
              description={"Base URL of the storage policy service."}
              placeholder={"https://api.example.com"}
              {...form.getInputProps("server")}
            />
            <TextInput
              label={"User info service (optional)"}
              description={
                "Base URL of the user directory service. Leave empty to resolve identity from the ID token."
              }
              placeholder={"https://user-service.example.com"}
              {...form.getInputProps("userInfoServer")}
            />
            <TextInput
              label={"Access token"}
              {...form.getInputProps("accessToken")}
            />
            <TextInput label={"ID token"} {...form.getInputProps("idToken")} />
            <Select
              checkIconPosition={"right"}
              data={["object", "folder"]}
              label={"Resource type"}
              {...form.getInputProps("resourceType")}
            />
            <TextInput
              label={"Resource address"}
              description={
                "Full URL identifying the resource, including scheme and path."
              }
              placeholder={"https://api.example.com/buckets/my-bucket/file.txt"}
              {...form.getInputProps("resourceAddress")}
            />
          </Stack>
          <Button type={"submit"} color={"cyan"} mt={"md"}>
            Load
          </Button>
        </Fieldset>
      </form>
    </Stack>
  );
}

function validateUrl(
  value: string,
  options: { required: boolean },
): string | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return options.required ? "This field is required." : null;
  }

  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return "Enter a full URL, e.g. https://api.example.com";
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return "URL must start with http:// or https://";
  }

  return null;
}
