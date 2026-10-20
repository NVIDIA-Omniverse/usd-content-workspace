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

import { IconCategoryFilled } from "@tabler/icons-react";
import { ActionIcon, Menu } from "@mantine/core";
import { useAppConfig } from "@/hooks/useAppConfig";

export function ReferencesMenu() {
  const { references } = useAppConfig();

  if (references.length === 0) {
    return null;
  }

  return (
    <Menu position="bottom-end" shadow="md" width={200}>
      <Menu.Target>
        <ActionIcon variant="subtle" color="gray" size="lg" aria-label="Reference links">
          <IconCategoryFilled size={20} />
        </ActionIcon>
      </Menu.Target>

      <Menu.Dropdown>
        {references.map((reference) => (
          <Menu.Item
            key={reference.url}
            component="a"
            href={reference.url}
            target="_blank"
            rel="noopener noreferrer"
            leftSection={
              reference.icon ? (
                <i className={`ti ti-${reference.icon}`} style={{ display: "block", fontSize: "14pt" }} aria-hidden />
              ) : undefined
            }
          >
            {reference.label}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}
