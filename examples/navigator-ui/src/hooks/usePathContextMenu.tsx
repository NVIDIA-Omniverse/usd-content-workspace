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

import React, { useCallback } from "react";
import { useContextMenu } from "mantine-contextmenu";
import { modals } from "@mantine/modals";
import { PathContextMenuContent } from "@/components/PathContextMenu";
import { useArchiveDownload } from "@/hooks/useArchiveDownload";
import { useDeletePathMutation } from "@/hooks/useDeletePathMutation";
import { useFileDownload } from "@/hooks/useFileDownload";
import { usePathClipboard } from "@/hooks/usePathClipboard";
import { Connection } from "@/state/appStore";
import { Path } from "@/state/path";
import { getParentPath } from "@/util/url";

export function usePathContextMenu(connection: Connection | undefined) {
  const contextMenu = useContextMenu();
  const clipboard = usePathClipboard(connection?.endpoint ?? "");
  const { download } = useFileDownload(connection);
  const archive = useArchiveDownload(connection);
  const deleteMutation = useDeletePathMutation(connection);

  const handleDownload = useCallback(
    (paths: Path[]) => {
      if (archive.available) {
        archive.download(paths);
        return;
      }
      void showDirectoryPicker({ mode: "readwrite" }).then((directoryHandle) => {
        void download(paths, directoryHandle);
      });
    },
    [archive, download]
  );

  const onRename = useCallback(
    (path: Path) => {
      modals.openContextModal({
        modal: "rename",
        title: "Rename",
        innerProps: {
          initial: path.name,
          onSubmit: (newName: string) => {
            clipboard.cut([path]);
            clipboard.paste.mutate({ destination: getParentPath(path.resourceAddress), newName });
          },
        },
      });
    },
    [clipboard]
  );

  return useCallback(
    (paths: Path[]) => (event: React.MouseEvent) => {
      if (!connection || !paths || paths.length === 0) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();

      void contextMenu.showContextMenu((close) => {
        return (
          <PathContextMenuContent
            paths={paths}
            connection={connection}
            clipboard={clipboard}
            onClose={close}
            onDownload={handleDownload}
            onDelete={(p: Path[]) => deleteMutation.mutate(p)}
            onRename={onRename}
          />
        );
      })(event);
    },
    [connection, contextMenu, deleteMutation, handleDownload, onRename, clipboard]
  );
}
