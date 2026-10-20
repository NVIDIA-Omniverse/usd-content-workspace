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

import React, { useCallback, useMemo } from "react";
import { IconCircleXFilled, IconCloudFilled, IconLogout } from "@tabler/icons-react";
import { ContextMenuItemOptions, useContextMenu } from "mantine-contextmenu";
import { Text } from "@mantine/core";
import { PathTreeNode, PathTreeNodeProps } from "@/components/PathTree/PathTreeNode";
import { PathTreeConnectionData } from "@/components/PathTree/types";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useLogout } from "@/hooks/useLogout";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { usePathTreeNodeQuery } from "@/hooks/usePathTreeNodeQuery";
import { getUserEmail } from "@/services/auth";
import { useAppStore } from "@/state/appStore";
import { getPathName } from "@/util/url";

export function PathTreeConnectionNode({ node, item }: PathTreeNodeProps<PathTreeConnectionData>) {
  const removeConnection = useAppStore((state) => state.removeConnection);
  const logout = useLogout();
  const { defaultDiscovery } = useAppConfig();
  const isDefaultServerMode = !!defaultDiscovery;

  const query = usePathTreeNodeQuery(node);

  const idToken = useAppStore((state) => state.connections[node.id]?.info.auth?.idToken);
  const email = useMemo(() => (idToken ? getUserEmail(idToken) : undefined), [idToken]);

  const contextMenu = useContextMenu();
  const highlighted = useAppStore((state) => state.highlightedNode === node.id);
  const setHighlightedNode = useAppStore((state) => state.setHighlightedNode);

  const handleContextMenu = useCallback(
    (event: React.MouseEvent) => {
      const items: ContextMenuItemOptions[] = [];

      // In default-server mode the connection set is owned by Helm-configured discovery,
      // so removing a connection from the UI would just have it re-synced on the next boot.
      if (!isDefaultServerMode) {
        items.push({
          key: "delete",
          icon: <IconCircleXFilled size={16} />,
          onClick: () => {
            removeConnection(node.id);
          },
        });
      }

      if (idToken) {
        items.push({
          key: "logout",
          title: "Log Out",
          icon: <IconLogout size={16} />,
          onClick: () => {
            void logout(node.id);
          },
        });
      }

      if (items.length === 0) {
        return;
      }

      setHighlightedNode(node.id);
      return contextMenu.showContextMenu(items)(event);
    },
    [contextMenu, idToken, isDefaultServerMode, logout, node.id, removeConnection, setHighlightedNode]
  );

  const name = getPathName(node.id);
  return (
    <PathTreeNode
      node={node}
      item={item}
      data-context-menu-active={highlighted || undefined}
      icon={<IconCloudFilled />}
      loading={query.isFetching}
      title={email ? `${name} (${email})` : name}
      to={constructPathNavLink(node.id)}
      onContextMenu={handleContextMenu}
    >
      {name}
      {email && (
        <Text span c="dimmed" size="xs" ml={6}>
          ({email})
        </Text>
      )}
    </PathTreeNode>
  );
}
