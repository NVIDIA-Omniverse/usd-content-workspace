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

import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer, VirtualItem } from "@tanstack/react-virtual";
import { useContextMenu } from "mantine-contextmenu";
import { Box } from "@mantine/core";
import { PathTreeAddConnectionNode } from "@/components/PathTree/PathTreeAddConnectionNode";
import { PathTreeConnectionNode } from "@/components/PathTree/PathTreeConnectionNode";
import { PathTreeFolderLikeNode } from "@/components/PathTree/PathTreeFolderLikeNode";
import { PathTreeRootNode } from "@/components/PathTree/PathTreeRootNode";
import { PathTreeNodeData, PathTreeNodeType } from "@/components/PathTree/types";
import { VirtualContainer } from "@/components/VirtualContainer";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useAppStore } from "@/state/appStore";
import { PathTreeNodes, PathTreeRoot, PathTreeState } from "@/state/slices/pathTreeSlice";
import styles from "./PathTree.module.css";

export function PathTree() {
  const tree = useAppStore((state) => state.tree);
  const treeState = useAppStore((state) => state.treeState);
  const { defaultDiscovery } = useAppConfig();
  const isDefaultServerMode = !!defaultDiscovery;

  const { isContextMenuVisible } = useContextMenu();
  const setHighlightedNode = useAppStore((state) => state.setHighlightedNode);
  useEffect(() => {
    if (!isContextMenuVisible) {
      setHighlightedNode(null);
    }
  }, [isContextMenuVisible, setHighlightedNode]);
  const treeNodes = useMemo((): PathTreeNodeData[] => {
    return Array.from(buildTreeNodes(tree, treeState, { isDefaultServerMode }));
  }, [tree, treeState, isDefaultServerMode]);

  const virtualContainer = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: treeNodes.length,
    getScrollElement: () => virtualContainer.current,
    estimateSize,
    overscan: 25,
    gap: 0,
  });

  return (
    <VirtualContainer role="tree" aria-label="Path tree" className={styles.pathTree} containerRef={virtualContainer}>
      <Box className={styles.pathTreeNodes} h={virtualizer.getTotalSize()}>
        {virtualizer.getVirtualItems().map((item) => {
          const node = treeNodes[item.index];
          return renderTreeNode(node, item);
        })}
      </Box>
    </VirtualContainer>
  );
}

function estimateSize(): number {
  return 32;
}

interface BuildTreeNodesOptions {
  isDefaultServerMode: boolean;
}

function* buildTreeNodes(
  tree: PathTreeNodes,
  treeState: PathTreeState,
  options: BuildTreeNodesOptions,
  node: string = PathTreeRoot,
  level = 0,
  endpoint = ""
): Generator<PathTreeNodeData> {
  if (node === PathTreeRoot) {
    const state = treeState.get(PathTreeRoot);
    yield { id: PathTreeRoot, type: PathTreeNodeType.Root, level: 0, expanded: state?.expanded };
  }

  const nodeState = treeState.get(node);
  if (nodeState?.expanded) {
    const children = tree.get(node) ?? [];

    let childEndpoint = endpoint;
    for (const node of children) {
      const state = treeState.get(node);
      if (level === 0) {
        childEndpoint = node;
        yield {
          id: node,
          type: PathTreeNodeType.Connection,
          level: level + 1,
          expanded: state?.expanded,
        };
      } else if (level === 1) {
        yield {
          id: node,
          type: PathTreeNodeType.TopLevelAddress,
          endpoint,
          resourceAddress: state?.data?.resourceAddress ?? "",
          level: level + 1,
          expanded: state?.expanded,
        };
      } else {
        yield {
          id: node,
          type: PathTreeNodeType.Folder,
          endpoint,
          resourceAddress: state?.data?.resourceAddress ?? "",
          level: level + 1,
          expanded: state?.expanded,
        };
      }

      if (state?.expanded) {
        yield* buildTreeNodes(tree, treeState, options, node, level + 1, childEndpoint);
      }
    }
  }

  // The "Add Connection" affordance is hidden in default-server mode: the connection set
  // is owned by the Helm-configured discovery service and users cannot add their own.
  if (node === PathTreeRoot && !options.isDefaultServerMode) {
    yield { id: "CONNECTION", type: PathTreeNodeType.AddConnection, level: 0 };
  }
}

function renderTreeNode(node: PathTreeNodeData, item: VirtualItem) {
  switch (node.type) {
    case PathTreeNodeType.Root:
      return <PathTreeRootNode key={node.id} node={node} item={item} />;
    case PathTreeNodeType.AddConnection:
      return <PathTreeAddConnectionNode key={node.id} node={node} item={item} />;
    case PathTreeNodeType.Connection:
      return <PathTreeConnectionNode key={node.id} node={node} item={item} />;
    case PathTreeNodeType.TopLevelAddress:
    case PathTreeNodeType.Folder:
      return <PathTreeFolderLikeNode key={node.id} node={node} item={item} />;
    default:
      return null;
  }
}
