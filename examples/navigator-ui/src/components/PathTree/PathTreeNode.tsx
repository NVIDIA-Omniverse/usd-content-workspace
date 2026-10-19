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

import React, { ReactNode, useCallback, useRef } from "react";
import { IconChevronDown, IconChevronRight, IconExclamationCircle } from "@tabler/icons-react";
import { VirtualItem } from "@tanstack/react-virtual";
import { NavLink } from "react-router-dom";
import { ActionIcon, Box, Group, GroupProps, Loader, MantineColor, Text } from "@mantine/core";
import { PathTreeNodeData } from "@/components/PathTree/types";
import { useAppStore } from "@/state/appStore";
import styles from "./PathTreeNode.module.css";

export interface PathTreeNodeProps<T extends PathTreeNodeData = PathTreeNodeData> extends GroupProps {
  error?: string;
  expandable?: boolean;
  icon?: ReactNode;
  iconColor?: MantineColor;
  item: VirtualItem;
  loading?: boolean;
  node: T;
  title?: string;
  to?: string;
  children?: ReactNode;
}

export function PathTreeNode<T extends PathTreeNodeData>({
  error,
  expandable = true,
  icon,
  iconColor = "inherit",
  item,
  loading,
  node,
  title = "",
  to = "",
  children,
  onContextMenu,
  ...props
}: PathTreeNodeProps<T>) {
  const selected = useAppStore((state) => state.selectedNode === node.id);
  const select = useAppStore((state) => state.selectTreeNode);
  const toggle = useAppStore((state) => state.toggleTreeNode);

  const textRef = useRef<HTMLParagraphElement>(null);

  const handleClick = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      props?.onClick?.(event);
      if (!event.defaultPrevented) {
        select(node.id);
        toggle(node.id, true);
        textRef.current?.scrollIntoView({ block: "nearest", inline: "center" });
      }
    },
    [select, node.id, toggle, props]
  );

  const handleChevronClick = useCallback(
    (event: React.MouseEvent) => {
      if (node.expanded) {
        event.stopPropagation();
        event.preventDefault();
        toggle(node.id, false);
      }
    },
    [node.expanded, node.id, toggle]
  );

  const handleContextMenu = useCallback(
    (event: React.MouseEvent) => {
      onContextMenu?.(event as React.MouseEvent<HTMLDivElement>);
    },
    [onContextMenu]
  );

  return (
    <NavLink
      className={styles.node}
      id={node.id}
      style={{
        gridRow: item.index + 1,
      }}
      to={to}
      onContextMenu={handleContextMenu}
    >
      <Group
        role="treeitem"
        aria-expanded={node.expanded ?? false}
        aria-selected={selected}
        gap={10}
        pl={node.level * 30 + 10}
        title={title}
        wrap="nowrap"
        {...props}
        onClick={handleClick}
      >
        {expandable ? (
          <ActionIcon
            data-tree-chevron
            variant="subtle"
            color="gray"
            size="sm"
            title={node.expanded ? `Hide ${title}` : `Show ${title}`}
            onClick={handleChevronClick}
          >
            {node.expanded ? <IconChevronDown size={20} /> : <IconChevronRight size={20} />}
          </ActionIcon>
        ) : (
          <Box w={20} />
        )}

        {loading ? (
          <Loader size={20} />
        ) : (
          icon && (
            <ActionIcon className={styles.nodeIcon} variant="transparent" c={iconColor} size={20} autoContrast={false}>
              {icon}
              {error && <IconExclamationCircle className={styles.nodeErrorIcon} size={16} />}
            </ActionIcon>
          )
        )}
        <Text ref={textRef} truncate="end">
          {children}
        </Text>
      </Group>
    </NavLink>
  );
}
