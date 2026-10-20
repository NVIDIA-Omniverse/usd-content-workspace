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
import { IconCopy, IconDownload, IconPencil, IconScissors, IconTrash } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { ContextMenuItem } from "mantine-contextmenu";
import { Loader } from "@mantine/core";
import { modals } from "@mantine/modals";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useNotification } from "@/hooks/useNotification";
import { usePathClipboard } from "@/hooks/usePathClipboard";
import { getPathMetadata } from "@/hooks/usePathMetadata";
import { CreateServiceApi, useServiceApi } from "@/hooks/useServiceApi";
import { PathMetadata } from "@/services/storage/metadataService";
import { Connection } from "@/state/appStore";
import { Path, PathType } from "@/state/path";
import { Batch } from "@/util/batch";

export function PathContextMenuContent({
  paths,
  connection,
  clipboard,
  onClose,
  onDownload,
  onDelete,
  onRename,
}: PathContextMenuContentProps) {
  const notifications = useNotification();
  const { requestTimeout, batchSize } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  const { data: result, isLoading } = useQuery({
    queryKey: [connection.endpoint, connection.currentFolder, paths, "context-menu-metadata"],
    queryFn: async () => {
      try {
        return await fetchPathsMetadata(paths, connection, requestTimeout, batchSize, createServiceApi);
      } catch (e) {
        onClose();
        notifications.show({
          type: "error",
          title: "Error loading permissions",
          message: e instanceof Error ? e.message : "Failed to load permissions.",
        });
        return null;
      }
    },
  });

  if (isLoading) {
    return (
      <ContextMenuItem
        onHide={onClose}
        submenuProps={{}}
        title="Checking permissions…"
        icon={<Loader size={16} />}
        disabled
      />
    );
  }

  if (!result) {
    return null;
  }

  const { paths: resultPaths, metadata } = result;
  const hasNonTopLevelAddress = resultPaths.some((path) => path.type !== PathType.TopLevelAddress);

  return (
    <>
      <ContextMenuItem
        onHide={onClose}
        submenuProps={{}}
        title="Download"
        icon={<IconDownload size={16} />}
        disabled={metadata.some((meta) => !meta?.acl.value.includes("read"))}
        onClick={() => void onDownload(resultPaths)}
      />
      <ContextMenuItem
        onHide={onClose}
        submenuProps={{}}
        title="Copy"
        icon={<IconCopy size={16} />}
        disabled={metadata.some((meta) => !meta?.acl.value.includes("read"))}
        onClick={() => clipboard.copy(resultPaths)}
      />
      {onRename && resultPaths.length === 1 && resultPaths[0].type !== PathType.TopLevelAddress && (
        <ContextMenuItem
          onHide={onClose}
          submenuProps={{}}
          title="Rename"
          icon={<IconPencil size={16} />}
          disabled={metadata.some((meta) => !meta?.acl.value.includes("write"))}
          onClick={() => onRename(resultPaths[0])}
        />
      )}
      {hasNonTopLevelAddress && (
        <ContextMenuItem
          onHide={onClose}
          submenuProps={{}}
          title="Cut"
          icon={<IconScissors size={16} />}
          disabled={metadata.some((meta) => !meta?.acl.value.includes("write"))}
          onClick={() => clipboard.cut(resultPaths)}
        />
      )}
      {hasNonTopLevelAddress && (
        <ContextMenuItem
          onHide={onClose}
          submenuProps={{}}
          title="Delete"
          icon={<IconTrash size={16} />}
          disabled={metadata.some((meta) => !meta?.acl.value.includes("write"))}
          onClick={() => {
            modals.openContextModal({
              modal: "delete",
              title: "Delete",
              innerProps: {
                paths: resultPaths,
                onConfirm: () => onDelete(resultPaths),
              },
            });
          }}
        />
      )}
    </>
  );
}

export interface PathContextMenuContentProps {
  paths: Path[];
  connection: Connection;
  clipboard: ReturnType<typeof usePathClipboard>;
  onClose: () => void;
  onDownload: (paths: Path[]) => void;
  onDelete: (paths: Path[]) => void;
  onRename?: (path: Path) => void;
}

export interface GetMetadataResult {
  paths: Path[];
  metadata: PathMetadata[];
}

async function fetchPathsMetadata(
  paths: Path[],
  connection: Connection,
  requestTimeout: number,
  batchSize: number,
  createServiceApi: CreateServiceApi
): Promise<GetMetadataResult> {
  if (paths.length === 1) {
    const metadata = await getPathMetadata(paths[0].resourceAddress, connection, { requestTimeout }, createServiceApi);
    return { paths, metadata: [metadata] };
  }

  const resultsMap = new Map<string, PathMetadata>();
  let rejectBatch: (reason: unknown) => void;
  const batchErrorPromise = new Promise<GetMetadataResult>((_, reject) => {
    rejectBatch = reject;
  });
  const batch = new Batch<Path, PathMetadata>(
    async (path) => getPathMetadata(path.resourceAddress, connection, { requestTimeout }, createServiceApi),
    {
      batchSize,
      onItemSuccess: (item, result) => {
        if (result) {
          resultsMap.set(item.resourceAddress, result);
        }
      },
      onItemError: (_item, error) => {
        rejectBatch(error);
      },
    }
  );
  await batch.add(paths);
  await Promise.race([batch.flush(), batchErrorPromise]);
  return {
    paths,
    metadata: paths.map((p) => resultsMap.get(p.resourceAddress)!),
  };
}
