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

import type { SplitterPaneSize } from "@mantine/hooks";
import { Slice } from "./types";

export interface ApplicationState {
  detailsPanel: boolean;
  detailsPanelWidth: SplitterPaneSize;
  detailsPanelTab: DetailsPanelTab;
  treePanelWidth: SplitterPaneSize;
  viewType: PathContentView;
  setDetailsVisible: (visible: boolean) => void;
  setDetailsPanelWidth: (width: SplitterPaneSize) => void;
  setDetailsPanelTab: (tab: DetailsPanelTab) => void;
  setTreePanelWidth: (width: SplitterPaneSize) => void;
  setViewType: (viewType: PathContentView) => void;
}

export const enum PathContentView {
  Table = "table",
  Grid = "grid",
}

export type DetailsPanelTab = "details" | "permissions" | "tags";

export const createApplicationSlice: Slice<ApplicationState> = (set) => ({
  detailsPanel: true,
  detailsPanelWidth: "300px",
  detailsPanelTab: "details",

  treePanelWidth: "280px",
  viewType: PathContentView.Table,

  setTreePanelWidth: (width) =>
    set((state) => {
      state.treePanelWidth = width;
      return state;
    }),

  setViewType: (view) =>
    set((state) => {
      state.viewType = view;
      return state;
    }),

  setDetailsVisible: (visible) =>
    set((state) => {
      state.detailsPanel = visible;
      return state;
    }),

  setDetailsPanelWidth: (width) =>
    set((state) => {
      state.detailsPanelWidth = width;
      return state;
    }),

  setDetailsPanelTab: (tab) =>
    set((state) => {
      state.detailsPanelTab = tab;
      return state;
    }),
});
