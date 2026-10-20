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

import { Path, PathType } from "@/state/path";
import { formatBytes } from "@/util/file";
import { getFileExtension, getPathName } from "@/util/url";

export const createMockFile = (resourceAddress: string): Path => ({
  name: getPathName(resourceAddress),
  type: PathType.File,
  displayedType: getFileExtension(resourceAddress),
  isFolderLike: false,
  resourceAddress,
  resourceIdentity: `${resourceAddress}-identity`,
  bytes: 1500,
  size: formatBytes(1500),
  modified: "02/27/2024 12:00:00",
  metadata: {
    dataObjectSize: 1500,
    lastModifiedTimestamp: "2025-05-01T21:23:12.714Z",
  },
});

export const createMockFolder = (resourceAddress: string): Path => ({
  name: getPathName(resourceAddress),
  type: PathType.Folder,
  displayedType: "Folder",
  isFolderLike: true,
  resourceAddress,
});

export const createMockTopLevelAddress = (resourceAddress: string): Path => ({
  name: getPathName(resourceAddress),
  type: PathType.TopLevelAddress,
  displayedType: "Bucket",
  isFolderLike: true,
  resourceAddress,
});
