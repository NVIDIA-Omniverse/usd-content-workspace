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
import { IconBucket, IconFolder, IconFolderOpen } from "@tabler/icons-react";
import { PathTreeNode, PathTreeNodeProps } from "@/components/PathTree/PathTreeNode";
import { PathTreeFolderData, PathTreeNodeType, PathTreeTopLevelAddressData } from "@/components/PathTree/types";
import { usePathContextMenu } from "@/hooks/usePathContextMenu";
import { constructPathNavLink } from "@/hooks/usePathNavigation";
import { usePathTreeNodeQuery } from "@/hooks/usePathTreeNodeQuery";
import { useAppStore, useConnection } from "@/state/appStore";
import { createFolderListItem, createTopLevelAddressListItem } from "@/state/path";
import { getPathName } from "@/util/url";

export function PathTreeFolderLikeNode({
  node,
  item,
}: PathTreeNodeProps<PathTreeFolderData | PathTreeTopLevelAddressData>) {
  const query = usePathTreeNodeQuery(node);
  const name = getPathName(node.resourceAddress);

  const connection = useConnection(node.endpoint);
  const showPathContextMenu = usePathContextMenu(connection);
  const path = useMemo(
    () =>
      node.type === PathTreeNodeType.Folder
        ? createFolderListItem(node.resourceAddress)
        : createTopLevelAddressListItem(node.resourceAddress),
    [node.type, node.resourceAddress]
  );

  const highlighted = useAppStore((state) => state.highlightedNode === node.id);
  const setHighlightedNode = useAppStore((state) => state.setHighlightedNode);

  const handleContextMenu = useCallback(
    (event: React.MouseEvent) => {
      if (!connection) {
        return;
      }
      setHighlightedNode(node.id);
      void showPathContextMenu([path])(event);
    },
    [connection, node.id, setHighlightedNode, showPathContextMenu, path]
  );

  return (
    <PathTreeNode
      node={node}
      item={item}
      data-context-menu-active={highlighted || undefined}
      icon={
        node.type === PathTreeNodeType.Folder ? node.expanded ? <IconFolderOpen /> : <IconFolder /> : <IconBucket />
      }
      iconColor="yellow.8"
      loading={query.isFetching || query.hasNextPage}
      error={query.error?.toString()}
      title={name}
      to={constructPathNavLink(node.endpoint, node.resourceAddress)}
      onContextMenu={handleContextMenu}
    >
      {name}
    </PathTreeNode>
  );
}
