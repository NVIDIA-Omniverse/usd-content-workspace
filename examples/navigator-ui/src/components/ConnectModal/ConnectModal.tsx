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

import { useState } from "react";
import { Button, Group, TextInput } from "@mantine/core";
import { useForm } from "@mantine/form";
import { ContextModalProps } from "@mantine/modals";

export type ConnectModalProps = ContextModalProps<{
  onAddConnection: (hostname: string) => Promise<void>;
}>;

export function ConnectModal({ innerProps: { onAddConnection } }: ConnectModalProps) {
  const [loading, setLoading] = useState(false);

  const form = useForm({
    mode: "uncontrolled",
    initialValues: {
      hostname: "",
    },
  });

  return (
    <form
      onSubmit={form.onSubmit(async (values) => {
        setLoading(true);
        try {
          await onAddConnection(values.hostname.trim());
        } catch (error) {
          if (error instanceof Error) {
            form.setFieldError("hostname", error.message);
          } else {
            form.setFieldError("hostname", String(error));
          }
        } finally {
          setLoading(false);
        }
      })}
    >
      <TextInput
        {...form.getInputProps("hostname")}
        aria-label="Discovery Hostname"
        data-autofocus
        withAsterisk
        label="Discovery Hostname"
        key={form.key("hostname")}
        disabled={loading}
      />

      <Group justify="flex-end" mt="md">
        <Button loading={loading} type="submit">
          Connect
        </Button>
      </Group>
    </form>
  );
}
