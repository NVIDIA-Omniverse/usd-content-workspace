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
import { NavLink } from "@mantine/core";
import type { Icon } from "@tabler/icons-react/dist/esm/types";
import { NavLink as RouterNavLink } from "react-router-dom";
import styles from "./NavbarLink.module.css";

export interface NavbarLinkProps {
  label: string;
  icon: Icon;
  to: string;
  disabled?: boolean;
}

export function NavbarLink({ label, icon: Icon, to, disabled }: NavbarLinkProps) {
  if (disabled) {
    return (
      <NavLink
        className={styles.navbarLink}
        label={label}
        leftSection={<Icon />}
        disabled
      />
    );
  }

  return (
    <NavLink
      className={styles.navbarLink}
      component={RouterNavLink}
      label={label}
      leftSection={<Icon />}
      to={to}
    />
  );
}
