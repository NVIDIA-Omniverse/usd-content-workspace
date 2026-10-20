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
import { Loader, Tooltip } from "@mantine/core";
import { IconUsersGroup } from "@tabler/icons-react";
import { useGroupName } from "../hooks/useGroupName";

export interface GroupNameProps {
  id: string | undefined;
}

export function GroupName({ id }: GroupNameProps) {
  const { displayName, isLoading } = useGroupName(id);

  if (!id) {
    return null;
  }

  if (isLoading) {
    return <Loader size={"xs"} />;
  }

  const label = displayName && displayName !== id ? displayName : id;

  const content = (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        verticalAlign: "middle",
      }}
    >
      <IconUsersGroup size={16} style={{ flexShrink: 0 }} />
      <span>{label}</span>
    </span>
  );

  if (label !== id) {
    return (
      <Tooltip label={id} withArrow>
        {content}
      </Tooltip>
    );
  }

  return content;
}
