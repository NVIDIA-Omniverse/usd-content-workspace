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

import { Slice } from "@/state/slices/types";
import { join } from "@/util/url";

export type PathTreeNodes = Map<string, Set<string>>;
export type PathTreeState = Map<string, PathTreeNodeState>;

export interface PathTreeNodeState {
  expanded: boolean;
  data?: {
    resourceAddress: string;
  };
}

export const PathTreeRoot = "ROOT";

export interface PathTreeSlice {
  tree: PathTreeNodes;
  treeState: PathTreeState;
  selectedNode: string;
  highlightedNode: string | null;

  setTreeNodes: (node: string, children: string[]) => void;
  addTreeNodes: (node: string, children: string[]) => void;

  selectTreeNode: (node: string) => void;
  toggleTreeNode: (node: string, toggled?: boolean) => void;
  setTreeState: (updates: Record<string, Partial<PathTreeNodeState>>) => void;
  setTreeNodeState: (node: string, nodeState: Partial<PathTreeNodeState>) => void;
  setHighlightedNode: (node: string | null) => void;
}

export const createPathTreeSlice: Slice<PathTreeSlice> = (set, _get, store) => {
  const tree: PathTreeNodes = new Map();
  tree.set(PathTreeRoot, new Set());

  const treeState: PathTreeState = new Map();
  treeState.set(PathTreeRoot, { expanded: true });

  store.subscribe((next, prev) => {
    if (!prev || next.endpoints !== prev.endpoints) {
      set((state) => {
        state.tree.set(PathTreeRoot, new Set([...next.endpoints]));
      });
    }
  });

  return {
    tree,
    treeState,
    selectedNode: PathTreeRoot,
    highlightedNode: null,

    setTreeNodes: (node, children) => {
      return set((state) => {
        state.tree.set(node, new Set(children));
        return state;
      });
    },

    addTreeNodes: (node, children) => {
      return set((state) => {
        const nodes = state.tree.get(node) ?? new Set();
        state.tree.set(node, new Set([...nodes, ...children]));
        return state;
      });
    },

    selectTreeNode: (node) => {
      return set((state) => {
        state.selectedNode = node;
        return state;
      });
    },

    toggleTreeNode: (node, toggled) => {
      return set((state) => {
        const nodeState = state.treeState.get(node) ?? { expanded: false };
        nodeState.expanded = typeof toggled === "undefined" ? !nodeState.expanded : toggled;

        state.treeState.set(node, nodeState);
        return state;
      });
    },

    setTreeState: (updates) => {
      return set((state) => {
        for (const [node, newState] of Object.entries(updates)) {
          const currentState = state.treeState.get(node) ?? { expanded: false };
          state.treeState.set(node, { ...currentState, ...newState });
        }
        return state;
      });
    },

    setTreeNodeState: (node, newState) => {
      return set((state) => {
        const currentState = state.treeState.get(node) ?? { expanded: false };
        state.treeState.set(node, { ...currentState, ...newState });
        return state;
      });
    },

    setHighlightedNode: (node) => {
      return set((state) => {
        state.highlightedNode = node;
        return state;
      });
    },
  };
};

export function folderLikeTreeNode(parent: string, resourceAddress: string) {
  if (resourceAddress) {
    return join(parent, resourceAddress);
  }
  return parent;
}
