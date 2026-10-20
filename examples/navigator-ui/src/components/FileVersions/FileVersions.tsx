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

import React, { useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Alert, Progress } from "@mantine/core";
import { FileVersionItem } from "@/components/FileVersions/FileVersionItem";
import { VirtualContainer } from "@/components/VirtualContainer";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { versioningService, type VersionPage } from "@/services/storage/versioningService";
import { Connection } from "@/state/appStore";
import { FileListItem } from "@/state/path";
import { parseStorageTimestamp } from "@/util/date";

export interface FileVersionsProps {
  file: FileListItem;
  connection: Connection;
}

export function FileVersions({ file, connection }: FileVersionsProps) {
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();
  const query = useQuery({
    queryKey: [connection.endpoint, file.resourceAddress, "version", "list"],
    queryFn: async () => {
      const options = getApiOptions(connection, { requestTimeout });
      const api = await createServiceApi(connection.info.discovery, "versioning", options);
      if (!api) {
        throw new Error("Storage versioning API is not available.");
      }
      const versioning = versioningService(api);

      const page = await versioning.listAll(file.resourceAddress);
      if (page.versionsOrder === "newest_first") {
        return page;
      }

      return {
        ...page,
        items: sortVersionsNewestFirst([...page.items]),
      };
    },
    refetchOnWindowFocus: false,
  });
  const { data, error, isError, isLoading } = query;
  const rows = data?.items ?? [];

  const virtualContainer = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => virtualContainer.current,
    estimateSize: () => 70,
    gap: 15,
    overscan: 5,
  });

  return (
    <VirtualContainer aria-label="Versions" containerRef={virtualContainer} mih={70}>
      {isLoading && <Progress aria-label="Loading versions" color="gray" size="xs" value={100} striped animated />}
      {isError && (
        <Alert color="red" title="Error" pos="absolute">
          {error.toString()}
        </Alert>
      )}

      {virtualizer.getVirtualItems().map((item) => {
        const row = rows[item.index];
        return (
          <FileVersionItem
            key={row.resourceInfo.resourceIdentity}
            resourceAddress={file.resourceAddress}
            resource={row.resourceInfo}
            connection={connection}
          />
        );
      })}
    </VirtualContainer>
  );
}

type VersionItem = VersionPage["items"][number];

function sortVersionsNewestFirst(items: VersionItem[]): VersionItem[] {
  return items.sort((left, right) => {
    const leftTime = getVersionTimestamp(left);
    const rightTime = getVersionTimestamp(right);
    if (leftTime === rightTime) {
      return 0;
    }
    return rightTime > leftTime ? 1 : -1;
  });
}

function getVersionTimestamp(version: VersionItem): number {
  const timestamp = version.resourceInfo.metadata.lastModifiedTimestamp;
  if (!timestamp) {
    return Number.NEGATIVE_INFINITY;
  }

  return parseStorageTimestamp(timestamp)?.getTime() ?? Number.NEGATIVE_INFINITY;
}
