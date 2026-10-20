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

import React from "react";
import { IconClipboardList, IconFileUpload, IconFolderPlus, IconFolderUp } from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useContextMenu } from "mantine-contextmenu";
import { modals } from "@mantine/modals";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useFileUpload } from "@/hooks/useFileUpload";
import { useNewFolderMutation } from "@/hooks/useNewFolderMutation";
import { usePathClipboard } from "@/hooks/usePathClipboard";
import { contentMetadataQueryKey, getPathMetadata } from "@/hooks/usePathMetadata";
import { useServiceApi } from "@/hooks/useServiceApi";
import { Connection, rootFolderPath } from "@/state/slices/connectionSlice";

export function useCurrentFolderContextMenu(connection: Connection) {
  const contextMenu = useContextMenu();
  const clipboard = usePathClipboard(connection.endpoint);
  const queryClient = useQueryClient();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  const { uploadFiles, uploadFolder } = useFileUpload();
  const newFolderMutation = useNewFolderMutation();

  return async (event: React.MouseEvent) => {
    event.persist();
    event.preventDefault();
    event.stopPropagation();

    const isTopLevelAddress = connection.currentFolder === rootFolderPath;
    if (isTopLevelAddress) {
      return;
    }

    const metadata = await queryClient.fetchQuery({
      queryKey: contentMetadataQueryKey(connection.endpoint, connection.currentFolder),
      queryFn: () => getPathMetadata(connection.currentFolder, connection, { requestTimeout }, createServiceApi),
    });
    return contextMenu.showContextMenu([
      {
        key: "paste",
        icon: <IconClipboardList size={16} />,
        disabled: !metadata.acl.value.includes("write"),
        hidden: !clipboard.canPaste(connection.currentFolder),
        onClick: () => {
          modals.openContextModal({
            modal: "copy",
            title: "Paste",
            innerProps: {
              source: clipboard.items,
              destination: connection.currentFolder,
              operation: clipboard.operation,
              onConfirm: (destination, overwrite) => {
                clipboard.paste.mutate({ destination, overwrite });
              },
            },
          });
        },
      },
      {
        key: "new-folder",
        icon: <IconFolderPlus size={16} />,
        disabled: !metadata.acl.value.includes("write"),
        onClick: () => {
          modals.openContextModal({
            modal: "newFolder",
            title: "New Folder",
            innerProps: {
              onCreateFolder: newFolderMutation.mutateAsync,
            },
          });
        },
      },
      {
        key: "upload-folder",
        icon: <IconFolderUp size={16} />,
        disabled: !metadata.acl.value.includes("write"),
        onClick: () => {
          void showDirectoryPicker({ mode: "readwrite" }).then((directoryHandle) => uploadFolder(directoryHandle));
        },
      },
      {
        key: "upload-files",
        icon: <IconFileUpload size={16} />,
        disabled: !metadata.acl.value.includes("write"),
        onClick: () => {
          void showOpenFilePicker({ multiple: true }).then((fileHandles) => uploadFiles(fileHandles));
        },
      },
    ])(event);
  };
}
