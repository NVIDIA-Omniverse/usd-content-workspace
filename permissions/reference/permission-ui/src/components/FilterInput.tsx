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
  Combobox,
  CloseButton,
  InputBase,
  TextInput,
  useCombobox,
} from "@mantine/core";
import type { ReactNode } from "react";

export interface FilterInputProps {
  data?: string[];
  error?: ReactNode;
  label?: ReactNode;
  name?: string;
  placeholder?: string;
  value?: string;
  unscoped?: ReactNode;
  leftSection?: ReactNode;
  leftSectionWidth?: number;
  onChange?: (value: string) => void;
}

export function FilterInput({
  data = [],
  error,
  label,
  name,
  placeholder,
  value,
  unscoped,
  leftSection,
  leftSectionWidth,
  onChange,
}: FilterInputProps) {
  const combobox = useCombobox({
    onDropdownClose: () => combobox.resetSelectedOption(),
  });

  const isUnscoped = value === "NULL" && !!unscoped;

  return (
    <Combobox
      store={combobox}
      withinPortal={false}
      onOptionSubmit={(value) => {
        onChange?.(value);
        combobox.closeDropdown();
      }}
    >
      <Combobox.Target>
        {isUnscoped ? (
          <InputBase
            component="button"
            type="button"
            pointer
            error={error}
            label={label}
            name={name}
            leftSection={leftSection}
            leftSectionWidth={leftSectionWidth}
            leftSectionPointerEvents={leftSection ? "all" : undefined}
            rightSection={
              <CloseButton
                aria-label={"Remove filter"}
                size={"sm"}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => onChange?.("")}
              />
            }
            onClick={() => combobox.openDropdown()}
            onFocus={() => combobox.openDropdown()}
            onBlur={() => combobox.closeDropdown()}
          >
            {unscoped}
          </InputBase>
        ) : (
          <TextInput
            error={error}
            label={label}
            name={name}
            placeholder={placeholder}
            leftSection={leftSection}
            leftSectionWidth={leftSectionWidth}
            leftSectionPointerEvents={leftSection ? "all" : undefined}
            rightSection={
              value && (
                <CloseButton
                  aria-label={"Remove filter"}
                  size={"sm"}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => onChange?.("")}
                />
              )
            }
            value={value}
            onChange={(event) => {
              onChange?.(event.currentTarget.value);
              combobox.openDropdown();
              combobox.updateSelectedOptionIndex();
            }}
            onClick={() => combobox.openDropdown()}
            onFocus={() => combobox.openDropdown()}
            onBlur={() => combobox.closeDropdown()}
          />
        )}
      </Combobox.Target>

      <Combobox.Dropdown>
        <Combobox.Options>
          <Combobox.Option value={"NULL"}>{unscoped}</Combobox.Option>
          {data.map((option) => (
            <Combobox.Option key={option} value={option}>
              {option}
            </Combobox.Option>
          ))}
        </Combobox.Options>
      </Combobox.Dropdown>
    </Combobox>
  );
}
