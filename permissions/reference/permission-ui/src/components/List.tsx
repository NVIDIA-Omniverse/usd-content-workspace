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
  Group,
  Stack,
  Table,
  Text,
  TextInput,
} from "@mantine/core";
import { useInputState } from "@mantine/hooks";
import { IconCopyPlusFilled, IconTrashFilled } from "@tabler/icons-react";
import clsx from "clsx";
import * as React from "react";

import styles from "./List.module.css";

export interface ListProps {
  description?: React.ReactNode;
  disabled?: boolean;
  error?: React.ReactNode;
  inputPlaceholder?: string;
  label?: React.ReactNode;
  placeholder?: React.ReactNode;
  renderHeaders?: () => React.ReactNode;
  renderItem?: (value: string, index: number) => React.ReactNode;
  value?: string[];
  onAdd?: (value: string) => void;
  onChange?: (value: string[]) => void;
  onRemove?: (value: string, index: number) => void;
}

export function List({
  description,
  disabled,
  error,
  label,
  inputPlaceholder,
  placeholder = "No items added.",
  renderHeaders = () => null,
  renderItem = (value: string) => <Table.Td>{value}</Table.Td>,
  value = [],
  onAdd,
  onChange,
  onRemove,
}: ListProps) {
  const [newValue, setNewValue] = useInputState("");

  function handleInputKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      return handleSubmit(event);
    }
  }

  function handleSubmit(event: React.SyntheticEvent) {
    if (!newValue) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    onAdd?.(newValue);
    onChange?.(Array.from(new Set([...value, newValue])));
    setNewValue("");
  }

  function deleteItem(item: string, index: number) {
    onRemove?.(item, index);
    onChange?.(value.filter((it) => it !== item));
  }

  return (
    <Stack gap={0}>
      {label && (
        <Text size={"sm"} fw={500}>
          {label}
        </Text>
      )}
      {description && (
        <Text size={"xs"} c={"dimmed"}>
          {description}
        </Text>
      )}

      {value.length > 0 ? (
        <Table
          className={clsx(styles.values, error && styles.error)}
          withTableBorder
        >
          {renderHeaders()}
          <Table.Tbody>
            {value.map((value, index) => (
              <Table.Tr key={value}>
                {renderItem(value, index)}
                <Table.Td w={30} miw={30}>
                  <ActionIcon
                    title={`Remove ${value}`}
                    color={"red"}
                    disabled={disabled}
                    variant={"light"}
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      deleteItem(value, index);
                    }}
                  >
                    <IconTrashFilled size={16} />
                  </ActionIcon>
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      ) : (
        <Text mt={"xs"} size={"sm"}>
          {placeholder}
        </Text>
      )}

      <Group align={"center"} my={"xs"}>
        <TextInput
          flex={1}
          disabled={disabled}
          placeholder={inputPlaceholder}
          value={newValue}
          onKeyDown={handleInputKeyDown}
          onChange={setNewValue}
        />
        <ActionIcon
          title={`Add ${newValue}`}
          color={"green"}
          disabled={disabled}
          variant={"light"}
          mr={"sm"}
          onClick={handleSubmit}
        >
          <IconCopyPlusFilled size={16} />
        </ActionIcon>
      </Group>

      {error && (
        <Text className={styles.error} size={"xs"}>
          {error}
        </Text>
      )}
    </Stack>
  );
}
