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

import { useCallback, useEffect, useMemo, useState } from "react";
import { IconCancel, IconCheck, IconExclamationCircleFilled } from "@tabler/icons-react";
import { ActionIcon, Anchor, Text } from "@mantine/core";
import { NotificationMessage } from "@/components/NotificationMessage";
import { create, getApiOptions } from "@/services/api";
import { archiveService, isArchiveServiceUsable, toArchiveItems, triggerFormDownload } from "@/services/archiveService";
import { Connection } from "@/state/appStore";
import { Path } from "@/state/path";
import { join } from "@/util/url";
import { useAppConfig } from "./useAppConfig";
import { useNotification } from "./useNotification";
import { DiscoveredApi, useServiceApi } from "./useServiceApi";

export interface ArchiveDownload {
  available: boolean;
  download: (paths: Path[]) => void;
}

/**
 * Downloads paths as a single ZIP via the archive service instead of the client-side FileSystem
 * Access flow. `available` is false unless discovery advertises a same-site archive service, so
 * callers can fall back to the FileSystem Access download.
 */
export function useArchiveDownload(connection: Connection | undefined): ArchiveDownload {
  const notifications = useNotification();
  const { getApi } = useServiceApi();
  const { requestTimeout } = useAppConfig();
  const [archiveApi, setArchiveApi] = useState<DiscoveredApi | null>(null);

  useEffect(() => {
    if (!connection) {
      setArchiveApi(null);
      return;
    }

    let cancelled = false;
    const options = getApiOptions(connection, { requestTimeout });
    void getApi(connection.info.discovery, "archive", options)
      .then((api) => {
        if (!cancelled) {
          setArchiveApi(api && isArchiveServiceUsable(api.baseUrl) ? api : null);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setArchiveApi(null);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [connection, getApi, requestTimeout]);

  const download = useCallback(
    (paths: Path[]) => {
      if (!archiveApi || !connection) {
        return;
      }

      const items = toArchiveItems(paths);
      const archiveName = paths.length === 1 ? paths[0].name : "download";
      const title = `Download ${paths.length === 1 ? paths[0].name : `${paths.length} items`}`;
      const api = create(archiveApi.url, getApiOptions(connection, { requestTimeout }));

      const abortController = new AbortController();

      function handleCancel() {
        abortController.abort();
        notifications.update({
          id,
          title,
          loading: false,
          color: "dark",
          icon: <IconCancel size={22} />,
          message: "Cancelled.",
          autoClose: 5000,
          withCloseButton: true,
        });
      }

      const id = notifications.show({
        title,
        message: (
          <NotificationMessage onCancel={handleCancel}>
            <Text size="xs" c="dimmed">
              Preparing archive…
            </Text>
          </NotificationMessage>
        ),
        loading: true,
        autoClose: false,
        withCloseButton: false,
      });

      const startDownload = (action: string) => {
        triggerFormDownload({ action, items, archiveName });
        notifications.update({
          id,
          title,
          loading: false,
          color: "green",
          icon: <IconCheck size={22} />,
          message: (
            <Text size="xs">
              Download started. If it does not start automatically,{" "}
              <Anchor
                component="button"
                type="button"
                size="xs"
                onClick={() => triggerFormDownload({ action, items, archiveName })}
              >
                click here
              </Anchor>{" "}
              to start manually.
            </Text>
          ),
          autoClose: DOWNLOAD_NOTIFICATION_AUTO_CLOSE_MS,
          withCloseButton: true,
        });
      };

      void (async () => {
        try {
          const { downloadAction } = await archiveService(api).prepare(items, archiveName, abortController.signal);
          if (abortController.signal.aborted) {
            return;
          }

          startDownload(join(archiveApi.baseUrl, downloadAction));
        } catch (error) {
          if (abortController.signal.aborted) {
            return;
          }
          notifications.update({
            id,
            title,
            loading: false,
            color: "red",
            icon: (
              <ActionIcon c="red" variant="transparent">
                <IconExclamationCircleFilled size={22} />
              </ActionIcon>
            ),
            message: <Text size="xs">{error instanceof Error ? error.message : "Failed to prepare archive."}</Text>,
            autoClose: DOWNLOAD_NOTIFICATION_AUTO_CLOSE_MS,
            withCloseButton: true,
          });
        }
      })();
    },
    [archiveApi, connection, requestTimeout, notifications]
  );

  return useMemo(() => ({ available: archiveApi != null, download }), [archiveApi, download]);
}

const DOWNLOAD_NOTIFICATION_AUTO_CLOSE_MS = 15_000;
