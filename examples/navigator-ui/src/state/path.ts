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

import { ListItem } from "@/services/storage";
import { formatDate } from "@/util/date";
import { formatBytes } from "@/util/file";
import { getFileExtension, getPathName } from "@/util/url";

export const enum PathType {
  File = "File",
  Folder = "Folder",
  TopLevelAddress = "Top-level address",
}

export type Path = TopLevelAddressItem | FolderListItem | FileListItem;
export interface PathShortInfo {
  type: PathType;
  resourceAddress: string;
}
export type TopLevelAddressItem = Omit<ListItem, "metadata" | "resourceIdentity"> & {
  name: string;
  type: PathType.TopLevelAddress;
  displayedType: "Bucket";
  isFolderLike: true;
};

export type FolderListItem = Omit<ListItem, "metadata" | "resourceIdentity"> & {
  name: string;
  type: PathType.Folder;
  isFolderLike: true;
  displayedType: "Folder";
};
export type FileListItem = ListItem & {
  name: string;
  type: PathType.File;
  displayedType: string;
  isFolderLike: false;
  bytes: number;
  size: string;
  modified: string;
  resourceIdentity: string;
};

export function createFileListItem(file: ListItem): FileListItem {
  return {
    name: getPathName(file.resourceAddress),
    type: PathType.File,
    isFolderLike: false,
    bytes: file.metadata.dataObjectSize,
    size: formatBytes(file.metadata.dataObjectSize),
    displayedType: getFileExtension(file.resourceAddress),
    modified: formatDate(file.metadata.lastModifiedTimestamp),
    ...file,
  };
}

export function createFolderListItem(resourceAddress: string): FolderListItem {
  return {
    name: getPathName(resourceAddress),
    type: PathType.Folder,
    displayedType: "Folder",
    isFolderLike: true,
    resourceAddress,
  };
}

export function createTopLevelAddressListItem(resourceAddress: string): TopLevelAddressItem {
  return {
    name: getPathName(resourceAddress),
    type: PathType.TopLevelAddress,
    displayedType: "Bucket",
    isFolderLike: true,
    resourceAddress,
  };
}
