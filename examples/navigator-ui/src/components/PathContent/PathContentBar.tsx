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
import {
  IconLayoutGrid,
  IconLayoutSidebarRightFilled,
  IconLayoutSidebarRightInactive,
  IconList,
} from "@tabler/icons-react";
import { useShallow } from "zustand/react/shallow";
import { ActionIcon, Group, Text } from "@mantine/core";
import { useAppStore } from "@/state/appStore";
import { PathContentView } from "@/state/slices/applicationSlice";
import { humanizeNumber } from "@/util/number";
import styles from "./PathContentBar.module.css";

export interface PathContentBarProps {
  itemCount: number;
}

export function PathContentBar({ itemCount }: PathContentBarProps) {
  const [view, setView] = useAppStore(useShallow((state) => [state.viewType, state.setViewType]));
  const [details, setDetailsVisible] = useAppStore(
    useShallow((state) => [state.detailsPanel, state.setDetailsVisible])
  );

  const nextViewLayout = view === PathContentView.Grid ? PathContentView.Table : PathContentView.Grid;
  return (
    <Group className={styles.bar} justify="space-between" align="center" w="100%">
      <Text size="sm">
        {humanizeNumber(itemCount)} item{itemCount === 1 ? "" : "s"}
      </Text>

      <Group gap="xs">
        <ActionIcon
          size={18}
          variant="default"
          color="grey"
          title={`Change view layout to ${nextViewLayout}`}
          onClick={() => setView(nextViewLayout)}
        >
          {view === PathContentView.Table ? <IconList /> : <IconLayoutGrid />}
        </ActionIcon>

        <ActionIcon
          size={18}
          variant="default"
          color="grey"
          title={details ? "Hide details panel" : "Show details panel"}
          onClick={() => setDetailsVisible(!details)}
        >
          {details ? <IconLayoutSidebarRightFilled /> : <IconLayoutSidebarRightInactive />}
        </ActionIcon>
      </Group>
    </Group>
  );
}
