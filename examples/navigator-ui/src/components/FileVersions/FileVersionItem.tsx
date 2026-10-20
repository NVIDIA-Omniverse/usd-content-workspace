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
import { IconRestore } from "@tabler/icons-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContextMenu } from "mantine-contextmenu";
import { Burger, Card, Group, Loader, Text } from "@mantine/core";
import { PathText } from "@/components/PathText";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useNotification } from "@/hooks/useNotification";
import { useServiceApi } from "@/hooks/useServiceApi";
import { getApiOptions } from "@/services/api";
import { fileObjectService, ResourceInfo } from "@/services/storage";
import { Connection } from "@/state/slices/connectionSlice";
import { formatDate } from "@/util/date";
import { formatBytes } from "@/util/file";

export interface FileVersionItemProps {
  connection: Connection;
  resourceAddress: string;
  resource: ResourceInfo;
}

export function FileVersionItem({ connection, resourceAddress, resource }: FileVersionItemProps) {
  const notification = useNotification();
  const { requestTimeout } = useAppConfig();
  const { createServiceApi } = useServiceApi();

  const queryClient = useQueryClient();
  const restore = useMutation({
    mutationFn: async () => {
      const options = getApiOptions(connection, { requestTimeout });
      const api = await createServiceApi(connection.info.discovery, "fileobject", options);
      if (!api) {
        throw new Error("Storage fileobject API is not available.");
      }
      const fileObject = fileObjectService(api);
      return await fileObject.copy(resource.resourceIdentity, resourceAddress);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: [connection.endpoint, resourceAddress, "version"],
      });

      notification.show({
        type: "success",
        title: "Version restored successfully.",
        message: <PathText title={resource.resourceIdentity}>{resource.resourceIdentity}</PathText>,
      });
    },
    onError: (error) => {
      notification.show({
        type: "error",
        title: "Version restore failed.",
        message: (
          <Text size="xs" title={error.toString()}>
            {error.toString()}
          </Text>
        ),
      });
    },
  });

  const { showContextMenu } = useContextMenu();
  const handleContextMenu = useCallback(
    (event: React.MouseEvent) => {
      showContextMenu([
        {
          key: "restore",
          icon: <IconRestore size={16} />,
          onClick: () => restore.mutate(),
          disabled: restore.isPending,
        },
      ])(event);
    },
    [showContextMenu, restore]
  );

  const date = formatDate(resource.metadata.lastModifiedTimestamp);
  return (
    <Card key={resource.resourceIdentity} aria-label={date} mb={15} h={70} onContextMenu={handleContextMenu}>
      <Group justify="space-between">
        <Text>{date}</Text>
        {restore.isPending ? <Loader size="xs" /> : <Burger size="xs" onClick={handleContextMenu} />}
      </Group>

      <Text size="xs" c="dimmed">
        {formatBytes(resource.metadata.dataObjectSize)}
      </Text>
    </Card>
  );
}
