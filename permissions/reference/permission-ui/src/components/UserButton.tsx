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
import { Group, Menu, Stack, Text, UnstyledButton } from "@mantine/core";
import { IconChevronRight, IconLogout } from "@tabler/icons-react";
import { useAuth } from "react-oidc-context";
import styles from "./UserButton.module.css";

export function UserButton() {
  const auth = useAuth();
  return (
    <Menu>
      <Menu.Target>
        <UnstyledButton className={styles.userButton}>
          <Group p={"md"} wrap={"nowrap"}>
            <Stack gap={0} flex={1}>
              <Text size={"sm"} fw={"500"}>
                {auth.user?.profile?.name}
              </Text>
              <Text c={"dimmed"} size={"xs"}>
                {auth.user?.profile?.email}
              </Text>
            </Stack>

            <IconChevronRight />
          </Group>
        </UnstyledButton>
      </Menu.Target>

      <Menu.Dropdown>
        <Menu.Item
          leftSection={<IconLogout />}
          onClick={() => auth.removeUser()}
        >
          Log out
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
}
