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

import { enableMapSet } from "immer";
import superjson from "superjson";
import { create } from "zustand";
import { devtools, persist, PersistStorage } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { Path } from "@/state/path";
import { ApplicationState, createApplicationSlice } from "@/state/slices/applicationSlice";
import { ClipboardSlice, createClipboardSlice } from "@/state/slices/clipboardSlice";
import { Connection, ConnectionSlice, createConnectionSlice } from "@/state/slices/connectionSlice";
import { createDiscoverySlice, DiscoverySlice } from "@/state/slices/discoverySlice";
import { createPathTreeSlice, PathTreeSlice } from "@/state/slices/pathTreeSlice";

export type { Connection, ConnectionInfo } from "./slices/connectionSlice";
export { rootFolderPath } from "./slices/connectionSlice";

export type AppStore = ConnectionSlice & ApplicationState & ClipboardSlice & PathTreeSlice & DiscoverySlice;

type AppPersistentState = Pick<
  AppStore,
  "endpoints" | "connections" | "treePanelWidth" | "detailsPanel" | "detailsPanelWidth"
>;

enableMapSet();

const storage: PersistStorage<AppPersistentState> = {
  getItem: (name) => {
    const str = localStorage.getItem(name);
    if (!str) {
      return null;
    }
    return superjson.parse(str);
  },
  setItem: (name, value) => {
    localStorage.setItem(name, superjson.stringify(value));
  },
  removeItem: (name) => localStorage.removeItem(name),
};

// Create the combined store with middleware
export const useAppStore = create<AppStore>()(
  devtools(
    persist(
      immer((...state) => ({
        ...createConnectionSlice(...state),
        ...createApplicationSlice(...state),
        ...createClipboardSlice(...state),
        ...createPathTreeSlice(...state),
        ...createDiscoverySlice(...state),
      })),
      {
        name: "storage-navigator",
        storage,
        version: 1,
        // v0 stored panel widths as unitless numbers; Mantine's Splitter now reads bare numbers as
        // percentages, so restore them as fixed pixel sizes.
        migrate: (persisted, version) => {
          if (persisted === null || typeof persisted !== "object" || Array.isArray(persisted)) {
            return {} as AppPersistentState;
          }
          const state = persisted as AppPersistentState;
          if (version < 1) {
            if (typeof state.treePanelWidth === "number") {
              state.treePanelWidth = `${state.treePanelWidth}px`;
            }
            if (typeof state.detailsPanelWidth === "number") {
              state.detailsPanelWidth = `${state.detailsPanelWidth}px`;
            }
          }
          return state;
        },
        partialize: (state) => ({
          endpoints: state.endpoints,
          connections: Object.fromEntries(
            Object.entries(state.connections).map(([key, conn]) => [
              key,
              {
                ...conn,
                currentFolder: "",
                selectedPaths: new Set<Path>(),
              },
            ])
          ),

          detailsPanel: state.detailsPanel,
          detailsPanelWidth: state.detailsPanelWidth,
          treePanelWidth: state.treePanelWidth,
          viewType: state.viewType,
        }),
      }
    )
  )
);

// Connection Slice Hooks
export const useConnection = (endpoint: string) => useAppStore((state) => state.connections[endpoint]);
export const useActiveConnection = (): Connection | null => {
  const connections = useAppStore((state) => state.connections);
  const activeConnection = useAppStore((state) => state.activeConnection);
  return connections[activeConnection] ?? null;
};
