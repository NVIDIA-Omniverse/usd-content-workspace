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

import { useCallback, useEffect, useMemo, useRef } from "react";
import { useAppConfig } from "@/hooks/useAppConfig";
import { useServiceApi } from "@/hooks/useServiceApi";
import { create, getApiOptions } from "@/services/api";
import { notificationService, OmniEvent } from "@/services/notificationService";
import { Connection } from "@/state/appStore";
import { Path } from "@/state/path";

type NotificationEventHandler = (event: OmniEvent) => void;

export interface PathSubscription {
  onEvent: (onNotificationEvent: NotificationEventHandler) => () => void;
}

export function usePathSubscription(_connection: Connection): PathSubscription {
  const handlersRef = useRef(new Set<NotificationEventHandler>());
  const { requestTimeout } = useAppConfig();
  const { getApi } = useServiceApi();

  // remove selected paths from the connection object to prevent reconnects to notifications when the user selects items
  const connection = useMemo(
    () => ({
      endpoint: _connection.endpoint,
      info: _connection.info,
      currentFolder: _connection.currentFolder,
      selectedPaths: new Set<Path>(),
    }),
    [_connection.endpoint, _connection.info, _connection.currentFolder]
  );

  useEffect(() => {
    let eventSource: { close: () => void } | undefined;
    let isActive = true;

    const setupSubscription = async () => {
      const options = getApiOptions(connection, { requestTimeout });
      const notifications = await getApi(connection.info.discovery, "notifications-consumer", options);

      if (!isActive || !notifications || connection.currentFolder === "") {
        return;
      }

      const api = create(notifications.url, {
        ...options,
        retry: 1,
      });

      eventSource = notificationService(api).createEventSource({
        path: connection.currentFolder,
        onMessage: (event) => {
          for (const handler of handlersRef.current) {
            try {
              const result = handler(event);
              if ((result as unknown) instanceof Promise) {
                (result as unknown as Promise<unknown>).catch(() => undefined);
              }
            } catch {
              // swallow — handlers are responsible for handling errors
            }
          }
        },
      });

      if (!isActive) {
        eventSource.close();
      }
    };

    void setupSubscription().catch(() => undefined);

    return () => {
      isActive = false;
      eventSource?.close();
    };
  }, [connection, requestTimeout, getApi]);

  const onEvent = useCallback((onNotificationEvent: NotificationEventHandler) => {
    handlersRef.current.add(onNotificationEvent);

    return () => {
      handlersRef.current.delete(onNotificationEvent);
    };
  }, []);

  return useMemo(() => ({ onEvent }), [onEvent]);
}
