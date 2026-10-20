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

import { type PreviewOperation } from "@/output";
import { fileFolderService } from "@/services/storage/fileFolderService";

export async function planFolderTransfer(
  folderSvc: ReturnType<typeof fileFolderService>,
  source: string,
  destination: string,
  action: "copy" | "move",
): Promise<PreviewOperation[]> {
  const listing = await folderSvc.listAll(source);
  const operations: PreviewOperation[] = listing.subResourceAddresses.map(
    (file) => ({
      action,
      source: file,
      destination: `${destination}${file.slice(source.length)}`,
    }),
  );
  operations.unshift({ action: "create-folder", source: destination });
  for (const subfolder of listing.subfolderAddresses) {
    operations.push(
      ...(await planFolderTransfer(
        folderSvc,
        subfolder,
        `${destination}${subfolder.slice(source.length)}`,
        action,
      )),
    );
  }
  if (action === "move") operations.push({ action: "delete-folder", source });
  return operations;
}
