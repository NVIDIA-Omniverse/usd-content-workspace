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

// src/services/notificationService.ts
import { createEventSource, type EventSourceMessage } from "eventsource-client";
import { type ApiInstance } from "@/services/api";

export interface NotificationClient {
  close(): void;
}

export interface FilterGroup {
  event_type: string;
  filters: Filter[];
}

export interface Filter {
  filter_type: string;
  resource_id: string;
}

export function notificationService(api: ApiInstance) {
  return {
    createEventSource: createNotificationEventSource(api),
  };
}

const createNotificationEventSource =
  (api: ApiInstance) =>
  (options: {
    path: string;
    eventTypes: string[];
    onMessage?: (event: unknown) => void;
    onConnected?: () => void;
    onError?: (error: Error) => void;
  }): NotificationClient => {
    let reconnectToken: string | null = null;
    let connectAttempts = 0;
    let innerClient: { close: () => void } | undefined;
    let closed = false;

    const resourceId = options.path.endsWith("/")
      ? options.path
      : `${options.path}/`;

    const buildUrl = (): string => {
      const params = new URLSearchParams();
      if (reconnectToken) {
        params.set("reconnect_token", reconnectToken);
        reconnectToken = null;
      } else {
        const filterGroups: FilterGroup[] = options.eventTypes.map(
          (eventType) => ({
            event_type: eventType,
            filters: [
              { filter_type: "STARTS_WITH_LAZY", resource_id: resourceId },
            ],
          }),
        );
        params.set("filter_groups", JSON.stringify(filterGroups));
      }
      return `api/v1beta/events/stream?${params.toString()}`;
    };

    const handleMessage = (event: EventSourceMessage) => {
      let data: unknown;
      try {
        data = JSON.parse(event.data);
      } catch {
        options.onError?.(new Error("Failed to parse event message"));
        return;
      }

      if (event.event === "connected") {
        const d = data as Record<string, unknown>;
        reconnectToken =
          typeof d.reconnect_token === "string" ? d.reconnect_token : null;
        connectAttempts = 0;
        options.onConnected?.();
        return;
      }

      if (event.event === "error") {
        const d = data as Record<string, unknown>;
        if (d.status_code === 401 || d.status_code === 403) {
          // Reconnect with existing token so the server can resume the stream after ky refreshes auth
          reconnect();
          return;
        }
        if (d.status_code === 404) {
          // Stale token — server cannot resume the stream; reconnect from scratch
          reconnectToken = null;
          reconnect();
          return;
        }
      }

      options.onMessage?.(data);
    };

    const connect = () => {
      if (closed) return;
      connectAttempts++;
      if (connectAttempts > 3) {
        closed = true;
        options.onError?.(new Error("Reconnect attempts exceeded."));
        return;
      }

      try {
        innerClient = createEventSource({
          url: buildUrl(),
          fetch: api.instance,
          onMessage: handleMessage,
          onScheduleReconnect: () => {
            if (closed) return;
            connectAttempts++; // fires after each network-level retry; onError after 3 callbacks
            if (connectAttempts > 3) {
              closed = true;
              innerClient?.close();
              options.onError?.(new Error("Reconnect attempts exceeded."));
            }
          },
        });
      } catch {
        reconnect();
      }
    };

    const reconnect = () => {
      innerClient?.close();
      connect();
    };

    connect();

    return {
      close: () => {
        closed = true;
        innerClient?.close();
      },
    };
  };
