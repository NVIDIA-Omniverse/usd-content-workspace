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

import { useEffect, useRef } from "react";
import { Button, Group, TextInput } from "@mantine/core";
import { useForm } from "@mantine/form";
import { ContextModalProps } from "@mantine/modals";
import { sanitizePathName } from "@/util/url";

export type RenameModalProps = ContextModalProps<{
  initial?: string;
  onSubmit: (name: string) => void;
}>;

export function RenameModal({ id, context, innerProps: { initial = "", onSubmit } }: RenameModalProps) {
  const form = useForm({
    mode: "controlled",

    initialValues: {
      newName: "",
    },

    validate: {
      newName: (value: string) => {
        if (!value.trim()) {
          return "Name must not be blank.";
        }

        if (sanitizePathName(value) !== value) {
          return "Name must not contain system characters such as slashes, colons, question marks and quotes.";
        }
      },
    },
  });

  const formRef = useRef(form);
  formRef.current = form;

  useEffect(() => {
    formRef.current.setInitialValues({ newName: initial });
    formRef.current.setValues({ newName: initial });
  }, [initial]);

  return (
    <form
      onSubmit={form.onSubmit((values) => {
        if (values.newName !== initial) {
          onSubmit(values.newName);
        }
        context.closeModal(id);
      })}
    >
      <TextInput name="newName" {...form.getInputProps("newName")} data-autofocus label="New name" withAsterisk />
      <Group justify="flex-end" mt="md">
        <Button color="green" type="submit">
          Rename
        </Button>
        <Button variant="default" onClick={() => context.closeModal(id)}>
          Cancel
        </Button>
      </Group>
    </form>
  );
}
