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

import React, { useEffect } from "react";
import { IconFolderOff } from "@tabler/icons-react";
import { Box, Progress } from "@mantine/core";
import { LoaderError } from "@/components/LoaderError";
import { PathContentGrid } from "@/components/PathContentGrid";
import { PathContentTable } from "@/components/PathContentTable";
import { useCurrentFolderContextMenu } from "@/hooks/useCurrentFolderContextMenu";
import { useFolderContentData, useFolderContentQuery } from "@/hooks/useFolderContentQuery";
import { usePathSubscription } from "@/hooks/usePathSubscription";
import { useStorageSubscription } from "@/hooks/useStorageSubscription";
import { useThumbnailSubscription } from "@/hooks/useThumbnailSubscription";
import { isHttpError } from "@/services/api";
import { Connection, useActiveConnection, useAppStore } from "@/state/appStore";
import { PathContentView } from "@/state/slices/applicationSlice";
import { getPathName } from "@/util/url";
import { PathContentBar } from "./PathContentBar";
import styles from "./PathContent.module.css";

export function PathContent() {
  const connection = useActiveConnection();
  if (!connection) {
    return <Box className={styles.root}>No active connection.</Box>;
  }

  return <PathContentBody connection={connection} />;
}

function PathContentBody({ connection }: { connection: Connection }) {
  const view = useAppStore((state) => state.viewType);
  const query = useFolderContentQuery(connection.endpoint, connection.currentFolder);
  const data = useFolderContentData(query);
  const pathSubscription = usePathSubscription(connection);

  useStorageSubscription(connection, pathSubscription);
  useThumbnailSubscription(connection, pathSubscription);

  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  useEffect(() => {
    // Reset the current selection every time the current path changes
    setSelectedPaths(connection.endpoint, []);
  }, [connection.endpoint, connection.currentFolder, setSelectedPaths]);

  const folderName = getPathName(connection.currentFolder || connection.endpoint);
  const handleContextMenu = useCurrentFolderContextMenu(connection);
  return (
    <Box
      aria-label={`${folderName} content`}
      className={styles.view}
      onContextMenu={(event) => void handleContextMenu(event)}
    >
      <Progress
        aria-label={`Loading ${folderName} content`}
        className={`${styles.progress} ${query.isFetching || query.hasNextPage ? styles.progressVisible : styles.progressHidden}`}
        size="xs"
        value={100}
        animated
      />

      {query.isError ? (
        isHttpError(query.error) && query.error.response.status === 404 ? (
          <LoaderError color="gray" icon={<IconFolderOff />} variant="default" title="Folder not found">
            {folderName} no longer exists or has been removed.
          </LoaderError>
        ) : (
          <LoaderError title={`Failed to load ${folderName} content`}>{query.error.toString()}</LoaderError>
        )
      ) : view === PathContentView.Table ? (
        <PathContentTable connection={connection} data={data} />
      ) : (
        <PathContentGrid connection={connection} data={data} />
      )}

      <PathContentBar itemCount={data.length} />
    </Box>
  );
}
