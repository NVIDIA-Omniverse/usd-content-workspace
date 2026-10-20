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

export const enum PathTreeNodeType {
  Root,
  Connection,
  TopLevelAddress,
  Folder,
  AddConnection,
}

export type PathTreeNodeData =
  | PathTreeRootData
  | PathTreeConnectionData
  | PathTreeTopLevelAddressData
  | PathTreeFolderData
  | PathTreeAddConnectionData;

interface PathTreeBaseNodeData {
  id: string;
  level: number;
  expanded?: boolean;
}

export interface PathTreeRootData extends PathTreeBaseNodeData {
  type: PathTreeNodeType.Root;
}

export interface PathTreeConnectionData extends PathTreeBaseNodeData {
  type: PathTreeNodeType.Connection;
}

export interface PathTreeTopLevelAddressData extends PathTreeBaseNodeData {
  type: PathTreeNodeType.TopLevelAddress;
  endpoint: string;
  resourceAddress: string;
}

export interface PathTreeFolderData extends PathTreeBaseNodeData {
  type: PathTreeNodeType.Folder;
  endpoint: string;
  resourceAddress: string;
}

export interface PathTreeAddConnectionData extends PathTreeBaseNodeData {
  type: PathTreeNodeType.AddConnection;
}
