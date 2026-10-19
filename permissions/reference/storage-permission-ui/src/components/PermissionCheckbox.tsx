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

import type { CSSProperties } from "react";
import { Checkbox } from "@mantine/core";
import { IconCheck, IconX } from "@tabler/icons-react";
import type { PermissionEffect } from "../api/policies.js";

export interface PermissionCheckboxProps {
  disabled?: boolean;
  effect: PermissionEffect;
  title?: string;
  ariaLabel?: string;
  onChange: (effect: PermissionEffect) => void;
}

export function PermissionCheckbox({
  disabled,
  effect,
  title,
  ariaLabel,
  onChange,
}: PermissionCheckboxProps) {
  return (
    <Checkbox
      color={effect === "deny" ? "red" : undefined}
      checked={effect === "allow"}
      disabled={disabled}
      icon={PermissionCheckboxIcon}
      indeterminate={effect === "deny"}
      styles={disabled && effect !== "unset" ? DISABLED_STYLES : undefined}
      title={title}
      aria-label={ariaLabel}
      onChange={() => onChange(NEXT_EFFECT[effect])}
    />
  );
}

const DISABLED_OPACITY = 0.7;

const DISABLED_STYLES = {
  root: { filter: `opacity(${DISABLED_OPACITY})` },
  input: {
    backgroundColor: "var(--checkbox-color)",
    borderColor: "var(--checkbox-color)",
  },
  icon: { color: "var(--checkbox-icon-color)" },
};

const NEXT_EFFECT: Record<PermissionEffect, PermissionEffect> = {
  unset: "allow",
  allow: "deny",
  deny: "unset",
};

interface PermissionCheckboxIconProps {
  indeterminate: boolean | undefined;
  className: string;
  style?: CSSProperties;
}

function PermissionCheckboxIcon({
  indeterminate,
  className,
  style,
}: PermissionCheckboxIconProps) {
  return indeterminate ? (
    <IconX className={className} style={style} />
  ) : (
    <IconCheck className={className} style={style} />
  );
}
