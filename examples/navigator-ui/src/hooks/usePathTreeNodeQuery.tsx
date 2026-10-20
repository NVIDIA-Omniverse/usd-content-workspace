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

import { useEffect } from "react";
import {
  PathTreeConnectionData,
  PathTreeFolderData,
  PathTreeNodeType,
  PathTreeTopLevelAddressData,
} from "@/components/PathTree/types";
import { useFolderContentQuery } from "@/hooks/useFolderContentQuery";
import { rootFolderPath, useAppStore } from "@/state/appStore";
import { folderLikeTreeNode, PathTreeNodeState } from "@/state/slices/pathTreeSlice";

export function usePathTreeNodeQuery(node: PathTreeTopLevelAddressData | PathTreeFolderData | PathTreeConnectionData) {
  const selected = useAppStore((state) => state.selectedNode === node.id);
  const setTreeNodes = useAppStore((state) => state.setTreeNodes);
  const setTreeState = useAppStore((state) => state.setTreeState);

  const endpoint = node.type === PathTreeNodeType.Connection ? node.id : node.endpoint;
  const resourceAddress = node.type === PathTreeNodeType.Connection ? rootFolderPath : node.resourceAddress;
  const query = useFolderContentQuery(endpoint, resourceAddress, selected || !!node.expanded);

  useEffect(() => {
    if (!query.data?.pages.length) {
      return;
    }

    const updates: Record<string, Partial<PathTreeNodeState>> = {};
    const children = [];
    for (const page of query.data.pages) {
      for (const folder of page.folders.values()) {
        const child = folderLikeTreeNode(node.id, folder.resourceAddress);
        children.push(child);
        updates[child] = { data: { resourceAddress: folder.resourceAddress } };
      }
    }

    setTreeNodes(node.id, children);
    setTreeState(updates);
  }, [node.id, endpoint, query.data, setTreeNodes, setTreeState]);

  return query;
}
