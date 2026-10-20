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

import { useEffect, useRef } from "react";
import { useParams } from "react-router-dom";
import { FolderQuery, useFolderContentQuery } from "@/hooks/useFolderContentQuery";
import { useNotification } from "@/hooks/useNotification";
import { rootFolderPath, useAppStore } from "@/state/appStore";
import { folderLikeTreeNode } from "@/state/slices/pathTreeSlice";
import { ensureFolderPath, getParents } from "@/util/url";

export function usePathNavigation() {
  const notifications = useNotification();
  const { endpoint = "", path = rootFolderPath } = useParams<{ endpoint: string; path: string }>();

  const endpoints = useAppStore((state) => state.endpoints);
  const endpointsRef = useRef(endpoints);
  endpointsRef.current = endpoints;

  const query = useFolderContentQuery(endpoint, rootFolderPath, !!endpoint);

  useEffect(() => {
    if (!endpoint) {
      return;
    }

    if (!endpointsRef.current.has(endpoint)) {
      notifications.show({
        type: "error",
        title: `Cannot connect to ${endpoint}`,
        message: `${endpoint} is not added to the application.`,
      });
      return;
    }

    if (!query.data) {
      return;
    }

    const store = useAppStore.getState();
    store.setActiveConnection(endpoint);
    store.setCurrentFolder(endpoint, path);

    syncTreeSelection(store, endpoint, path, collectTopLevelAddresses(query.data));
  }, [endpoint, notifications, path, query.data]);
}

export function constructPathNavLink(endpoint: string, resourceAddress = ""): string {
  const path = encodeURIComponent(resourceAddress ? ensureFolderPath(resourceAddress) : resourceAddress);
  return `/${encodeURIComponent(endpoint)}/path/${path}`;
}

type NavigationStore = ReturnType<typeof useAppStore.getState>;

interface TreeSelection {
  node: string;
  address: string;
  inConnection: boolean;
}

function syncTreeSelection(store: NavigationStore, endpoint: string, path: string, topLevelAddresses: string[]) {
  const connectionNode = folderLikeTreeNode(endpoint, rootFolderPath);
  const selection = getConnectionSelection(store, connectionNode);

  if (selection.inConnection && isSameFolder(selection.address, path)) {
    return;
  }

  if (path === rootFolderPath) {
    selectConnectionRoot(store, connectionNode);
    return;
  }

  if (isPathWithinSelection(selection, connectionNode, path)) {
    revealPathWithinSelection(store, selection, path);
    return;
  }

  revealPathUnderTopLevelAddress(store, connectionNode, topLevelAddresses, path);
}

function getConnectionSelection(store: NavigationStore, connectionNode: string): TreeSelection {
  const { selectedNode, treeState } = store;
  return {
    node: selectedNode,
    address: treeState.get(selectedNode)?.data?.resourceAddress ?? "",
    inConnection: selectedNode === connectionNode || selectedNode.startsWith(ensureFolderPath(connectionNode)),
  };
}

function isSameFolder(a: string, b: string): boolean {
  return ensureFolderPath(a) === ensureFolderPath(b);
}

function selectConnectionRoot(store: NavigationStore, connectionNode: string) {
  store.selectTreeNode(connectionNode);
  store.setTreeNodeState(connectionNode, { expanded: true, data: { resourceAddress: rootFolderPath } });
}

function isPathWithinSelection(selection: TreeSelection, connectionNode: string, path: string): boolean {
  return (
    selection.inConnection &&
    selection.node !== connectionNode &&
    !!selection.address &&
    isWithinFolder(path, selection.address)
  );
}

function isWithinFolder(path: string, ancestor: string): boolean {
  return ensureFolderPath(path).startsWith(ensureFolderPath(ancestor));
}

function revealPathWithinSelection(store: NavigationStore, selection: TreeSelection, path: string) {
  store.setTreeNodeState(selection.node, { expanded: true });
  attachChain(store, selection.node, [...intermediateFolders(selection.address, path), path]);
}

function attachChain(store: NavigationStore, root: string, addresses: string[]) {
  let parent = root;
  for (const address of addresses) {
    const child = folderLikeTreeNode(parent, address);
    store.addTreeNodes(parent, [child]);
    store.setTreeNodeState(child, { expanded: true, data: { resourceAddress: address } });
    parent = child;
  }
  store.selectTreeNode(parent);
}

function intermediateFolders(ancestor: string, path: string): string[] {
  return getParents(path).filter((parent) => isWithinFolder(parent, ancestor) && !isSameFolder(parent, ancestor));
}

function revealPathUnderTopLevelAddress(
  store: NavigationStore,
  connectionNode: string,
  topLevelAddresses: string[],
  path: string
) {
  const topLevelAddress = mostSpecificAncestor(topLevelAddresses, path);
  const folders = topLevelAddress ? foldersFromTopLevelAddress(topLevelAddress, path) : [];

  store.setTreeNodeState(connectionNode, { expanded: true, data: { resourceAddress: rootFolderPath } });
  attachChain(store, connectionNode, [...folders, path]);
}

function mostSpecificAncestor(topLevelAddresses: string[], path: string): string | undefined {
  return topLevelAddresses.filter((address) => isWithinFolder(path, address)).sort((a, b) => b.length - a.length)[0];
}

function foldersFromTopLevelAddress(topLevelAddress: string, path: string): string[] {
  const parents = getParents(path);
  const index = parents.findIndex((parent) => isSameFolder(parent, topLevelAddress));
  return index === -1 ? [] : parents.slice(index);
}

function collectTopLevelAddresses(data: NonNullable<FolderQuery["data"]>): string[] {
  return data.pages.flatMap((page) => Array.from(page.folders.values())).map((folder) => folder.resourceAddress);
}
