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

import { Authentication } from "@/services/auth";
import { Path } from "@/state/path";
import { Slice } from "@/state/slices/types";

/**
 * Root folder path constant
 */
export const rootFolderPath = "";

/**
 * Connection information interface
 */
export interface ConnectionInfo {
  auth: Authentication | null;
  discovery: string;
}

/**
 * Connection interface representing a storage endpoint
 */
export interface Connection {
  endpoint: string;
  currentFolder: string;
  selectedPaths: Set<Path>;
  info: ConnectionInfo;
}

export interface ConnectionSlice {
  endpoints: Set<string>;
  connections: Record<string, Connection>;
  activeConnection: string;
  addConnection: (endpoint: string, info: ConnectionInfo) => Connection;
  removeConnection: (endpoint: string) => void;
  setActiveConnection: (endpoint: string) => void;
  setCurrentFolder: (endpoint: string, folderPath: string) => void;
  setSelectedPaths: (endpoint: string, paths: Path[]) => void;
  toggleSelectedPath: (endpoint: string, path: Path) => void;
  setConnectionAuth: (endpoint: string, auth: Authentication | null) => void;
  /**
   * Replaces the current connection set with exactly the storage endpoints advertised by
   * the given discovery URL. Used by default-server mode to keep the tree in sync with
   * what the Helm-configured discovery currently exposes:
   *   - Removes any existing connections that are not in `endpoints`.
   *   - Adds or updates each endpoint, preserving `currentFolder`/`selectedPaths` and any
   *     fresher `info.auth` already on a matching persisted connection.
   *   - Picks `endpoints[0]` as the active connection when the current active
   *     connection is empty or no longer in the set.
   */
  syncDefaultConnections: (discovery: string, endpoints: string[], auth: Authentication | null) => void;
}

export const createConnectionSlice: Slice<ConnectionSlice> = (set) => ({
  endpoints: new Set(),
  connections: {} as Record<string, Connection>,
  activeConnection: "",
  addConnection: (endpoint: string, info: ConnectionInfo) => {
    const connection: Connection = {
      endpoint,
      currentFolder: rootFolderPath,
      selectedPaths: new Set(),
      info,
    };
    set((state) => {
      state.connections[endpoint] = connection;
      state.endpoints.add(endpoint);
      return state;
    });
    return connection;
  },
  removeConnection: (endpoint: string) =>
    set((state) => {
      delete state.connections[endpoint];
      state.endpoints.delete(endpoint);
      return state;
    }),
  setActiveConnection: (endpoint: string) =>
    set((state) => {
      state.activeConnection = endpoint;
      return state;
    }),
  setCurrentFolder: (endpoint: string, folder: string) =>
    set((state) => {
      if (!state.connections[endpoint]) {
        return state;
      }
      state.connections[endpoint].currentFolder = folder;
      return state;
    }),
  setSelectedPaths: (endpoint: string, paths: Path[]) =>
    set((state) => {
      if (!state.connections[endpoint]) {
        return state;
      }
      state.connections[endpoint].selectedPaths = new Set(paths);
      return state;
    }),
  toggleSelectedPath: (endpoint: string, path: Path) => {
    set((state) => {
      if (!state.connections[endpoint]) {
        return state;
      }

      if (state.connections[endpoint].selectedPaths.has(path)) {
        state.connections[endpoint].selectedPaths.delete(path);
      } else {
        state.connections[endpoint].selectedPaths.add(path);
      }
      return state;
    });
  },
  setConnectionAuth: (endpoint: string, auth: Authentication | null) =>
    set((state) => {
      if (!state.connections[endpoint]) {
        return state;
      }
      state.connections[endpoint].info = { ...state.connections[endpoint].info, auth };
      return state;
    }),
  syncDefaultConnections: (discovery: string, endpoints: string[], auth: Authentication | null) =>
    set((state) => {
      const nextEndpoints = new Set(endpoints);

      for (const existing of Object.keys(state.connections)) {
        if (!nextEndpoints.has(existing)) {
          delete state.connections[existing];
        }
      }
      for (const existing of [...state.endpoints]) {
        if (!nextEndpoints.has(existing)) {
          state.endpoints.delete(existing);
        }
      }

      for (const endpoint of endpoints) {
        const previous = state.connections[endpoint];
        const nextAuth = auth ?? previous?.info.auth;
        const nextInfo: ConnectionInfo = {
          discovery,
          auth: nextAuth,
        };
        if (previous) {
          previous.info = nextInfo;
        } else {
          state.connections[endpoint] = {
            endpoint,
            currentFolder: rootFolderPath,
            selectedPaths: new Set(),
            info: nextInfo,
          };
        }
        state.endpoints.add(endpoint);
      }

      if (!state.activeConnection || !nextEndpoints.has(state.activeConnection)) {
        state.activeConnection = endpoints[0] ?? "";
      }
      return state;
    }),
});
