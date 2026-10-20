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

import { createEventSource, EventSourceMessage, EventSourceOptions } from "eventsource-client";
import { z, ZodEnum, ZodType } from "zod";
import { fromSnakeCaseSchema } from "@/util/schemas";
import { ApiInstance, isHttpError } from "./api";
import { ResourceRefSchema } from "./thumbnailService";

export function notificationService(api: ApiInstance) {
  return {
    createEventSource: createTypedEventSource(api),
  };
}

const createTypedEventSource =
  (api: ApiInstance) =>
  (
    options: Omit<EventSourceOptions, "onMessage" | "fetch" | "url"> & {
      path: string;
      onMessage?: (event: OmniEvent) => void;
    }
  ): EventSourceClient => {
    let reconnectToken: string | null = null;
    let connectAttempts = 0;

    const handleEventSourceMessage = (event: EventSourceMessage) => {
      let data;
      try {
        data = JSON.parse(event.data);
      } catch (error: unknown) {
        if (error instanceof Error) {
          throw new Error(`Failed to parse event message: ${error.message}`);
        }
        throw new Error("Failed to parse event message: Unknown error");
      }

      try {
        if (event.event === "connected") {
          const { success, data: connectData } = ConnectEventSchema.safeParse(data);
          if (success) {
            reconnectToken = connectData.reconnectToken ?? null;
            connectAttempts = 0;
            return;
          }
        }
        if (event.event === "error") {
          const { success, data: errorData } = ErrorEventSchema.safeParse(data);
          // server detected that the auth token has expired. ky will handle the token refresh
          if (success && (errorData.statusCode === 401 || errorData.statusCode === 403)) {
            handleClose(true);
            return;
          }
          // attempted to reconnect with a stale reconnect token
          if (success && errorData.statusCode === 404) {
            reconnectToken = null;
            handleClose(true);
            return;
          }
        }

        const typedEvent = OmniEventSchema.parse(data);
        options.onMessage?.(typedEvent);
      } catch (error) {
        if (error instanceof z.ZodError) {
          // Skip unknown message types
          return;
        }
        throw error;
      }
    };

    const create = (path: string) => {
      connectAttempts++;
      if (connectAttempts > 3) {
        throw new Error("Reconnect attempts exceeded. Please refresh the page.");
      }

      const filterGroups: FilterGroup[] = [
        ...StorageEventTypes.map((eventType) => ({
          event_type: eventType,
          filters: [{ filter_type: "STARTS_WITH_LAZY", resource_id: path.endsWith("/") ? path : `${path}/` }],
        })),
        ...ThumbnailEventTypes.map((eventType) => ({
          event_type: eventType,
          filters: [{ filter_type: "STARTS_WITH_LAZY", resource_id: path.endsWith("/") ? path : `${path}/` }],
        })),
      ];

      const params = new URLSearchParams();
      if (reconnectToken) {
        params.set("reconnect_token", reconnectToken);
        reconnectToken = null;
      } else {
        params.set("filter_groups", JSON.stringify(filterGroups));
      }

      let client: EventSourceClient | undefined;

      try {
        client = createEventSource({
          ...options,
          url: `events/stream?${params}`,
          fetch: api.instance,
          onMessage: (event) => handleEventSourceMessage(event),
          onConnect: () => {
            connectAttempts = 0;
          },
          onScheduleReconnect: () => {
            // createEventSource does not throw an error in the case of a timeout or other network error
            // onScheduleReconnect is called when the underlying fetch call encounters an error before receiving a response
            connectAttempts++;
            if (connectAttempts > 3) {
              client!.close();
            }
          },
        });
      } catch (error) {
        if (isHttpError(error)) {
          const { response } = error;

          if (response.status === 401 || response.status === 403) {
            // Auth token expired or invalid - ky will handle token refresh
            handleClose(true);
            return;
          }

          if (response.status === 404) {
            // Stale reconnect token
            reconnectToken = null;
            handleClose(true);
            return;
          }
        }

        handleClose(false);
        return;
      }

      if (!client) {
        throw new Error("Failed to create event source client");
      }

      return client;
    };

    const handleClose = (reconnect: boolean) => {
      if (reconnect) {
        client = create(options.path);
      }
    };

    let client = create(options.path);

    return {
      connect: (path: string) => {
        client?.close();
        client = create(path);
      },
      close: () => {
        client?.close();
      },
    };
  };

export const enum ConnectionState {
  Connected = "Connected",
  Disconnected = "Disconnected",
}

export interface EventSourceClient {
  connect: (path: string) => void;
  close: () => void;
}

export interface FilterGroup {
  event_type: string;
  filters: Filter[];
}

export interface Filter {
  filter_type: string;
  resource_id: string;
}

const ConnectEventSchema = fromSnakeCaseSchema(
  z.object({
    message: z.string(),
    reconnectToken: z.string(),
  })
);

const ErrorEventSchema = fromSnakeCaseSchema(
  z.object({
    statusCode: z.number(),
  })
);

// base event schema for all notifications
const EventSchema = <K extends ZodEnum<[string, ...string[]]>, T extends ZodType>(eventType: K, messageSchema: T) =>
  fromSnakeCaseSchema(
    z.object({
      eventType,
      principalIdentity: z.string(),
      occurredAt: z.string(),
      publishedAt: z.string(),
      message: messageSchema,
    })
  );

export const StorageEventTypes = [
  "omni.storage.created",
  "omni.storage.deleted",
  "omni.storage.dir_created",
  "omni.storage.dir_deleted",
] as const;

const StorageEventSchema = EventSchema(
  z.enum(StorageEventTypes),
  fromSnakeCaseSchema(
    z.object({
      resourceIdentity: z.optional(z.string()),
      resourceAddress: z.string(),
    })
  )
);
export type StorageEvent = z.infer<typeof StorageEventSchema>;

export const ThumbnailEventTypes = ["omni.thumbnail.associated"] as const;

const ThumbnailAssociatedEventSchema = EventSchema(
  z.enum(ThumbnailEventTypes),
  fromSnakeCaseSchema(
    z.object({
      associationTarget: ResourceRefSchema,
      specification: z.unknown().optional(),
      targetType: z.string(),
      thumbnail: ResourceRefSchema,
    })
  )
);
export type ThumbnailAssociatedEvent = z.infer<typeof ThumbnailAssociatedEventSchema>;

// placeholder for permission event types to test OmniEventSchema parsing
export const PermissionEventTypes = ["omni.permission.created", "omni.permission.deleted"] as const;

// placeholder for permission message schema
const PermissionEventSchema = EventSchema(
  z.enum(PermissionEventTypes),
  fromSnakeCaseSchema(
    z.object({
      resourceAddress: z.string(),
      policy: z.string(),
    })
  )
);
export type PermissionEvent = z.infer<typeof PermissionEventSchema>;

const OmniEventSchema = z.union([StorageEventSchema, ThumbnailAssociatedEventSchema, PermissionEventSchema]);

export type OmniEvent = z.infer<typeof OmniEventSchema>;
