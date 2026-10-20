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

import {
  createEventSource,
  type EventSourceClient,
  type EventSourceMessage,
  type EventSourceOptions,
} from "eventsource-client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "@/services/api";
import { type FilterGroup } from "./notificationService";
import { notificationService } from "./notificationService";

vi.mock("eventsource-client", () => ({
  createEventSource: vi.fn((options: EventSourceOptions) => {
    const client = { close: vi.fn() };
    setTimeout(() => {
      options.onMessage?.({
        event: "connected",
        data: JSON.stringify({
          message: "Connected",
          reconnect_token: "tok-abc",
        }),
      } as EventSourceMessage);
    }, 0);
    return client;
  }),
}));

const mockCreate = vi.mocked(createEventSource);

function getOptionsAt(index = 0): EventSourceOptions {
  const call = mockCreate.mock.calls[index];
  if (!call) {
    throw new Error(`Expected createEventSource call at index ${index}`);
  }
  return call[0] as EventSourceOptions;
}

describe("notificationService", () => {
  const api = create("https://storage.example.com", {
    accessToken: "test-token",
  });
  const service = notificationService(api);

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns a client with a close method", () => {
    const client = service.createEventSource({
      path: "/bucket/folder/",
      eventTypes: ["omni.storage.created"],
    });
    expect(client.close).toBeDefined();
    client.close();
  });

  it("builds filter_groups from user-specified event types", () => {
    service.createEventSource({
      path: "/bucket/folder",
      eventTypes: ["omni.storage.created", "omni.thumbnail.associated"],
    });
    const url = getOptionsAt(0).url;
    const params = new URLSearchParams(String(url).split("?")[1] ?? "");
    const groups: FilterGroup[] = JSON.parse(
      params.get("filter_groups") ?? "[]",
    ) as FilterGroup[];
    expect(groups).toHaveLength(2);
    expect(groups[0].event_type).toBe("omni.storage.created");
    expect(groups[1].event_type).toBe("omni.thumbnail.associated");
    expect(groups[0].filters[0].filter_type).toBe("STARTS_WITH_LAZY");
    expect(groups[0].filters[0].resource_id).toBe("/bucket/folder/");
  });

  it("ensures path ends with / in filter resource_id", () => {
    service.createEventSource({
      path: "/bucket/folder",
      eventTypes: ["omni.storage.created"],
    });
    const url = getOptionsAt(0).url;
    const params = new URLSearchParams(String(url).split("?")[1] ?? "");
    const groups: FilterGroup[] = JSON.parse(
      params.get("filter_groups") ?? "[]",
    ) as FilterGroup[];
    expect(groups[0].filters[0].resource_id).toBe("/bucket/folder/");
  });

  it("calls onConnected when connected event arrives", async () => {
    const onConnected = vi.fn();
    service.createEventSource({
      path: "/bucket/",
      eventTypes: ["omni.storage.created"],
      onConnected,
    });
    await new Promise((r) => setTimeout(r, 10));
    expect(onConnected).toHaveBeenCalledOnce();
  });

  it("calls onMessage with parsed event data", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/bucket/",
      eventTypes: ["omni.storage.created"],
      onMessage,
    });

    getOptionsAt(0).onMessage?.({
      event: "message",
      data: JSON.stringify({
        event_type: "omni.storage.created",
        principal_identity: "user@example.com",
        occurred_at: "2026-04-20T10:00:00Z",
        published_at: "2026-04-20T10:00:01Z",
        message: { resource_address: "/bucket/file.usd" },
      }),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledOnce();
    expect(onMessage.mock.calls[0][0]).toMatchObject({
      event_type: "omni.storage.created",
      message: { resource_address: "/bucket/file.usd" },
    });
  });

  it("uses reconnect_token on subsequent connections after connected event", async () => {
    const client = service.createEventSource({
      path: "/bucket/",
      eventTypes: ["omni.storage.created"],
    });
    await new Promise((r) => setTimeout(r, 10));

    // Trigger a reconnect via a 404 error event
    getOptionsAt(0).onMessage?.({
      event: "error",
      data: JSON.stringify({ status_code: 404 }),
    } as EventSourceMessage);

    // Second connection should NOT use reconnect_token (cleared on 404)
    const url2 = getOptionsAt(1).url;
    const params2 = new URLSearchParams(String(url2).split("?")[1] ?? "");
    expect(params2.has("reconnect_token")).toBe(false);
    expect(params2.has("filter_groups")).toBe(true);

    client.close();
  });

  it("calls onError after 3 failed reconnect attempts via onScheduleReconnect", async () => {
    vi.mocked(createEventSource).mockImplementation((optionsOrUrl) => {
      const { onScheduleReconnect } = optionsOrUrl as EventSourceOptions;
      const client = { close: vi.fn() };
      setTimeout(() => {
        onScheduleReconnect?.({ delay: 0 });
        onScheduleReconnect?.({ delay: 0 });
        onScheduleReconnect?.({ delay: 0 });
        onScheduleReconnect?.({ delay: 0 });
      }, 0);
      return client as unknown as EventSourceClient;
    });

    const onError = vi.fn();
    service.createEventSource({
      path: "/bucket/",
      eventTypes: ["omni.storage.created"],
      onError,
    });

    await new Promise((r) => setTimeout(r, 10));
    expect(onError).toHaveBeenCalledOnce();
    const firstError = onError.mock.calls[0]?.[0];
    expect(firstError).toBeInstanceOf(Error);
    expect((firstError as Error).message).toContain(
      "Reconnect attempts exceeded",
    );
  });
});
