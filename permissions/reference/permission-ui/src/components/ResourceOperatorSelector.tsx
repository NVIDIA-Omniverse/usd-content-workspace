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
import { Menu, Text, UnstyledButton } from "@mantine/core";
import { IconChevronDown } from "@tabler/icons-react";
import classes from "./ResourceOperatorSelector.module.css";

export type ResourceOperator = "==" | "in";

export const RESOURCE_OPERATOR_SECTION_WIDTH = 96;

export interface ResourceOperatorSelectorProps {
  value: ResourceOperator;
  disabled?: boolean;
  onChange: (operator: ResourceOperator) => void;
}

/**
 * Leading operator selector for the resource input. Switches the resource scope
 * between strict equality and the `in` (prefix) operator.
 */
export function ResourceOperatorSelector({
  value,
  disabled,
  onChange,
}: ResourceOperatorSelectorProps) {
  return (
    <Menu position={"bottom-start"} disabled={disabled}>
      <Menu.Target>
        <UnstyledButton
          type={"button"}
          disabled={disabled}
          aria-label={`Resource operator: ${value}`}
          className={classes.selector}
        >
          <Text size={"sm"} lh={1} className={classes.label}>
            {RESOURCE_OPERATOR_META[value].label}
          </Text>
          <span className={classes.right}>
            <IconChevronDown size={14} />
            <span className={classes.divider} />
          </span>
        </UnstyledButton>
      </Menu.Target>
      <Menu.Dropdown>
        {(Object.keys(RESOURCE_OPERATOR_META) as ResourceOperator[]).map(
          (operator) => {
            const meta = RESOURCE_OPERATOR_META[operator];
            return (
              <Menu.Item key={operator} onClick={() => onChange(operator)}>
                <Text size={"sm"}>{meta.label}</Text>
                <Text size={"xs"} c={"dimmed"}>
                  {meta.description}
                </Text>
              </Menu.Item>
            );
          },
        )}
      </Menu.Dropdown>
    </Menu>
  );
}

const RESOURCE_OPERATOR_META: Record<
  ResourceOperator,
  { label: string; description: string }
> = {
  "==": { label: "equals", description: "Match a single resource exactly." },
  in: { label: "in", description: "Match every resource under a prefix." },
};
