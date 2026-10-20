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

import React from "react";
import { IconCircleXFilled, IconPlus } from "@tabler/icons-react";
import { ActionIcon, Alert, Button, Divider, Group, Stack, Table, TextInput } from "@mantine/core";
import { useForm } from "@mantine/form";
import { notifications } from "@mantine/notifications";
import { useSetTagsMutation } from "@/hooks/useSetTagsMutation";
import { PathShortInfo } from "@/state/path";
import { Connection } from "@/state/slices/connectionSlice";
import styles from "./TagFormTable.module.css";

export interface TagFormTableProps {
  connection: Connection;
  path: PathShortInfo;
  tags: Record<string, string>;
  etag: string;
  onReset: () => void;
}

interface FormValues {
  tags: [string, string][];
  newKey: string;
  newValue: string;
}

const tagValidationRegex = /^[\w\p{L} ]*$/u;

const invalidTagMessage = "contains invalid characters. Only letters, numbers, underscores, and spaces are allowed.";

export function TagFormTable({ connection, path, tags, etag, onReset }: TagFormTableProps) {
  const mutation = useSetTagsMutation(connection);

  const form = useForm<FormValues>({
    initialValues: {
      // Create a new array instance so that later mutations do not modify the original reference.
      // This allows Mantine Form's dirty state to be calculated correctly when items are added/removed.
      tags: [...Object.entries(tags)],
      newKey: "",
      newValue: "",
    },
  });

  const newKeyRef = React.useRef<HTMLInputElement>(null);

  function validateKey(key: string, currentIndex?: number): string | null {
    const trimmed = key.trim();

    if (!trimmed) {
      return "Key cannot be empty";
    }

    if (!tagValidationRegex.test(trimmed)) {
      return `Key ${invalidTagMessage}`;
    }

    const duplicateFound = form.values.tags.some(([existingKey], idx) => {
      if (currentIndex !== undefined && idx === currentIndex) {
        return false; // Skip the row currently being edited
      }
      return existingKey.trim() === trimmed;
    });

    if (duplicateFound) {
      return "Key must be unique";
    }

    return null;
  }

  function validateValue(value: string): string | null {
    if (!tagValidationRegex.test(value.trim())) {
      return `Value ${invalidTagMessage}`;
    }

    return null;
  }

  function addNewTag() {
    const keyError = validateKey(form.values.newKey);
    const valueError = validateValue(form.values.newValue);

    if (keyError) {
      form.setFieldError("newKey", keyError);
    }
    if (valueError) {
      form.setFieldError("newValue", valueError);
    }

    if (keyError || valueError) {
      return;
    }

    const newTag: [string, string] = [form.values.newKey.trim(), form.values.newValue.trim()];
    // Use immutable update so that form.initialValues.tags reference is not mutated, allowing isDirty to behave correctly
    form.setFieldValue("tags", [...form.values.tags, newTag]);
    form.setFieldValue("newKey", "");
    form.setFieldValue("newValue", "");
    newKeyRef.current?.focus();
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      addNewTag();
    }
  }

  function handleAddClick(event: React.MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    addNewTag();
  }

  function handleSubmit(values: FormValues) {
    // If the user has typed a new tag but forgotten to press the add button,
    // automatically validate and include it.

    const trimmedKey = values.newKey.trim();
    const trimmedValue = values.newValue.trim();

    const tagsToSubmit: [string, string][] = [...values.tags];

    if (trimmedKey) {
      const keyError = validateKey(trimmedKey);
      const valueError = validateValue(trimmedValue);

      if (keyError) {
        form.setFieldError("newKey", keyError);
      }
      if (valueError) {
        form.setFieldError("newValue", valueError);
      }
      if (keyError || valueError) {
        return;
      }

      tagsToSubmit.push([trimmedKey, trimmedValue]);
    }

    mutation.mutate(
      {
        resourceAddress: path.resourceAddress,
        metadata: Object.fromEntries(tagsToSubmit),
        etag,
      },
      {
        onSuccess: () => {
          notifications.show({
            title: "Success",
            message: "Tags updated successfully.",
            color: "green",
          });
          // Update form values to reflect what was successfully submitted and reset dirty state
          form.setValues({
            tags: tagsToSubmit,
            newKey: "",
            newValue: "",
          });

          form.resetDirty();
          onReset();
        },
        onError: () => {
          notifications.show({
            title: "Failed to submit tag changes",
            message: "Please try again.",
            color: "red",
          });
        },
      }
    );
  }

  function handleDelete(index: number) {
    form.setFieldValue(
      "tags",
      form.values.tags.filter((_, i) => i !== index)
    );
  }

  function createBlurHandler(fieldPath: string, validateFn: (value: string) => string | null) {
    return (event: React.FocusEvent<HTMLInputElement>) => {
      // Run Mantine's original onBlur logic for this field (marks field as touched, etc.)
      const originalOnBlur = form.getInputProps(fieldPath).onBlur as
        | ((e: React.FocusEvent<HTMLInputElement>) => void)
        | undefined;
      originalOnBlur?.(event);

      const error = validateFn(event.currentTarget.value);
      form.setFieldError(fieldPath, error);
    };
  }

  const submitDisabled = mutation.isPending || Object.values(form.errors).some(Boolean) || !form.isDirty();

  return (
    <form onSubmit={form.onSubmit(handleSubmit)}>
      {mutation.error && (
        <Alert color="red" title="Failed to submit tag changes">
          {mutation.error.message}
        </Alert>
      )}

      <Stack gap="md">
        <Stack gap={0}>
          <Table withRowBorders={false} className={styles.tagList}>
            <Table.Thead>
              <Table.Tr>
                <Table.Th className={styles.tagKey}>Key</Table.Th>
                <Table.Th className={styles.tagValue}>Value</Table.Th>
                <Table.Th className={styles.tagActions} />
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {form.values.tags.map(([key, _value], index) => {
                const disabled = mutation.isPending;
                return (
                  <Table.Tr key={index}>
                    <Table.Td className={styles.tagKey}>
                      <TextInput
                        aria-label="Key"
                        disabled={disabled}
                        placeholder="Key"
                        {...form.getInputProps(`tags.${index}.0`)}
                        onBlur={createBlurHandler(`tags.${index}.0`, (v) => validateKey(v, index))}
                      />
                    </Table.Td>
                    <Table.Td className={styles.tagValue}>
                      <TextInput
                        aria-label="Value"
                        disabled={disabled}
                        placeholder="No value"
                        {...form.getInputProps(`tags.${index}.1`)}
                        onBlur={createBlurHandler(`tags.${index}.1`, validateValue)}
                      />
                    </Table.Td>
                    <Table.Td className={styles.tagActions}>
                      <ActionIcon
                        color="red"
                        variant="transparent"
                        disabled={disabled}
                        title={`Remove ${key}`}
                        className={styles.actionIcon}
                        onClick={() => handleDelete(index)}
                      >
                        <IconCircleXFilled size={20} />
                      </ActionIcon>
                    </Table.Td>
                  </Table.Tr>
                );
              })}

              <Table.Tr className={styles.tagRow}>
                <Table.Td className={styles.tagKey}>
                  <TextInput
                    aria-label="Key"
                    placeholder="Key"
                    disabled={mutation.isPending}
                    onKeyDown={handleKeyDown}
                    ref={newKeyRef}
                    {...form.getInputProps("newKey")}
                  />
                </Table.Td>
                <Table.Td className={styles.tagValue}>
                  <TextInput
                    aria-label="Value"
                    placeholder="Value (optional)"
                    disabled={mutation.isPending}
                    onKeyDown={handleKeyDown}
                    {...form.getInputProps("newValue")}
                  />
                </Table.Td>
                <Table.Td className={styles.tagActions}>
                  <ActionIcon
                    color="green"
                    variant="transparent"
                    disabled={mutation.isPending}
                    title="Add tag"
                    onClick={handleAddClick}
                  >
                    <IconPlus size={20} className={styles.actionIcon} />
                  </ActionIcon>
                </Table.Td>
              </Table.Tr>
            </Table.Tbody>
          </Table>
        </Stack>

        <Divider />
        <Group>
          <Button disabled={submitDisabled} loading={mutation.isPending} type="submit">
            Submit
          </Button>
          <Button color="gray" disabled={mutation.isPending} variant="light" onClick={() => form.reset()}>
            Cancel
          </Button>
        </Group>
      </Stack>
    </form>
  );
}
