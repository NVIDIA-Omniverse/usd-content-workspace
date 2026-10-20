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
import { sanitizePathName } from "@/util/url";

export type NewFolderModalProps = ContextModalProps<{
  onCreateFolder: (name: string) => Promise<void>;
}>;

export function NewFolderModal({ id, context, innerProps: { onCreateFolder } }: NewFolderModalProps) {
  const [loading, setLoading] = useState(false);

  const form = useForm({
    mode: "uncontrolled",
    initialValues: {
      name: "",
    },

    validate: {
      name: (value: string) => {
        if (!value.trim()) {
          return "Folder name must not be blank.";
        }

        if (sanitizePathName(value) !== value) {
          return "Folder name must not contain system characters such as slashes, colons, question marks and quotes.";
        }
      },
    },
  });

  function close() {
    form.reset();
    context.closeModal(id);
  }

  return (
    <form
      onSubmit={form.onSubmit(async (values) => {
        setLoading(true);
        try {
          await onCreateFolder(values.name.trim());
          close();
        } catch (error) {
          if (error instanceof Error) {
            form.setErrors({ name: error.message });
          } else {
            form.setErrors({ name: "An unexpected error occurred" });
          }
        } finally {
          setLoading(false);
        }
      })}
    >
      <TextInput {...form.getInputProps("name")} data-autofocus disabled={loading} label="Folder name" withAsterisk />
      <Group justify="flex-end" mt="md">
        <Button loading={loading} type="submit">
          Create
        </Button>
        <Button variant="default" onClick={() => context.closeModal(id)}>
          Cancel
        </Button>
      </Group>
    </form>
  );
}
