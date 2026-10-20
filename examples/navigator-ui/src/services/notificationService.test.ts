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

import { createEventSource, type EventSourceMessage, type EventSourceOptions } from "eventsource-client";
import { HTTPError } from "ky";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiOptions, create } from "./api";
import { notificationService, OmniEvent } from "./notificationService";

let connectErrorResponseCode: number | null = null;

vi.mock("eventsource-client", () => ({
  createEventSource: vi.fn(({ url: _url, onMessage, onConnect, onDisconnect: _onDisconnect }: EventSourceOptions) => {
    const client = {
      close: vi.fn(() => {
        _onDisconnect?.();
      }),
    };

    if (connectErrorResponseCode !== null) {
      const response = new Response(null, { status: connectErrorResponseCode });
      // Construct full URL by combining the test base URL with the relative URL
      const fullUrl = new URL(_url as string, "http://test.com").toString();
      const request = new Request(fullUrl);
      throw new HTTPError(response, request, {
        method: "GET",
        retry: {
          limit: 0,
        },
        prefixUrl: "http://test.com",
        onDownloadProgress: () => vi.fn(),
        onUploadProgress: () => vi.fn(),
      });
    }

    setTimeout(() => {
      onConnect?.();
      onMessage?.({
        event: "connected",
        data: JSON.stringify({
          message: "Connected",
          reconnect_token: "test-reconnect-token",
        }),
      } as EventSourceMessage);
    }, 0);

    return client;
  }),
}));

const mockCreateEventSource = vi.mocked(createEventSource);

describe("notificationService", () => {
  // Create API instance with prefixUrl set to http://test.com
  const apiOptions: ApiOptions = {
    discovery: "test.com",
    auth: {
      get: () => null,
      set: () => undefined,
    },
  };
  const api = create("http://test.com", apiOptions);
  const service = notificationService(api);

  beforeEach(() => {
    vi.clearAllMocks();
    connectErrorResponseCode = null;
  });

  it("should create an event source client", () => {
    const onMessage = vi.fn();
    const client = service.createEventSource({
      path: "/test",
      onMessage,
    });

    expect(client).toBeDefined();
    expect(client.connect).toBeDefined();
    expect(client.close).toBeDefined();
  });

  it("should subscribe to storage path events and thumbnail association events", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const url = new URL(`http://test.com/${options.url}`);
    const filterGroups = JSON.parse(url.searchParams.get("filter_groups") ?? "[]") as {
      event_type: string;
      filters: { filter_type: string; resource_id: string }[];
    }[];

    expect(filterGroups).toEqual(
      expect.arrayContaining([
        {
          event_type: "omni.storage.created",
          filters: [{ filter_type: "STARTS_WITH_LAZY", resource_id: "/test/" }],
        },
        {
          event_type: "omni.thumbnail.associated",
          filters: [{ filter_type: "STARTS_WITH_LAZY", resource_id: "/test/" }],
        },
      ])
    );
    expect(filterGroups.filter(({ event_type }) => event_type === "omni.thumbnail.associated")).toEqual([
      {
        event_type: "omni.thumbnail.associated",
        filters: [{ filter_type: "STARTS_WITH_LAZY", resource_id: "/test/" }],
      },
    ]);
  });

  it("should handle storage events", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const mockEvent: OmniEvent = {
      eventType: "omni.storage.created",
      principalIdentity: "test-user",
      occurredAt: new Date().toISOString(),
      publishedAt: new Date().toISOString(),
      message: {
        resourceIdentity: "test-resource",
        resourceAddress: "/test/path",
      },
    };

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(mockEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(mockEvent);
  });

  it("should handle thumbnail association events", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    const thumbnailAssociatedEvent = {
      event_type: "omni.thumbnail.associated",
      message: {
        association_target: {
          resource_address: "https://storage.example.com/folder/image.png",
          resource_identity: "https://storage.example.com/folder/image.png?version-id=target",
        },
        specification: {
          image_to_image: {
            height: 512,
            width: 512,
          },
        },
        target_type: "FILE",
        thumbnail: {
          resource_address: "https://storage.example.com/thumbnails/thumb.png",
          resource_identity: "https://storage.example.com/thumbnails/thumb.png?version-id=thumbnail",
        },
      },
      occurred_at: new Date().toISOString(),
      principal_identity: "test-user",
      published_at: new Date().toISOString(),
      resource_id: "https://storage.example.com/thumbnails/thumb.png?version-id=thumbnail",
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(thumbnailAssociatedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.thumbnail.associated",
        principalIdentity: "test-user",
        message: {
          associationTarget: {
            resourceAddress: "https://storage.example.com/folder/image.png",
            resourceIdentity: "https://storage.example.com/folder/image.png?version-id=target",
          },
          specification: {
            image_to_image: {
              height: 512,
              width: 512,
            },
          },
          targetType: "FILE",
          thumbnail: {
            resourceAddress: "https://storage.example.com/thumbnails/thumb.png",
            resourceIdentity: "https://storage.example.com/thumbnails/thumb.png?version-id=thumbnail",
          },
        },
      })
    );
  });

  it("should handle different types of storage events", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    const deletedEvent = {
      event_type: "omni.storage.deleted",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_identity: "test-resource",
        resource_address: "/test/path",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(deletedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.storage.deleted",
        principalIdentity: "test-user",
        message: {
          resourceIdentity: "test-resource",
          resourceAddress: "/test/path",
        },
      })
    );

    const dirCreatedEvent = {
      event_type: "omni.storage.dir_created",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_identity: "test-resource",
        resource_address: "/test/dir/path",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(dirCreatedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.storage.dir_created",
        principalIdentity: "test-user",
        message: {
          resourceIdentity: "test-resource",
          resourceAddress: "/test/dir/path",
        },
      })
    );

    const dirDeletedEvent = {
      event_type: "omni.storage.dir_deleted",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_identity: "test-resource",
        resource_address: "/test/dir/path",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(dirDeletedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.storage.dir_deleted",
        principalIdentity: "test-user",
        message: {
          resourceIdentity: "test-resource",
          resourceAddress: "/test/dir/path",
        },
      })
    );
  });

  it("should handle malformed storage events", () => {
    const onMessage = vi.fn();
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);

    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    // Test with invalid JSON - should throw
    expect(() => {
      onMessageHandler?.({
        event: "message",
        data: "invalid-json",
      } as EventSourceMessage);
    }).toThrow("Failed to parse event message");

    expect(onMessage).not.toHaveBeenCalled();

    // Test with valid JSON but invalid schema - should be skipped
    const invalidEvent = {
      event_type: "omni.storage.created",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_identity: "test-resource",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(invalidEvent),
    } as EventSourceMessage);

    expect(onMessage).not.toHaveBeenCalled();

    const unknownEvent = {
      event_type: "unknown.event.type",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_address: "/test/path",
        resource_identity: "test-resource",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(unknownEvent),
    } as EventSourceMessage);

    expect(onMessage).not.toHaveBeenCalled();

    consoleErrorSpy.mockRestore();
  });

  it("should handle permission events", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    // Test with permission created event
    const permissionCreatedEvent = {
      event_type: "omni.permission.created",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_address: "/test/path",
        policy: "read-only",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(permissionCreatedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.permission.created",
        principalIdentity: "test-user",
        message: {
          resourceAddress: "/test/path",
          policy: "read-only",
        },
      })
    );

    // Test with permission deleted event
    const permissionDeletedEvent = {
      event_type: "omni.permission.deleted",
      principal_identity: "test-user",
      occurred_at: new Date().toISOString(),
      published_at: new Date().toISOString(),
      message: {
        resource_address: "/test/path",
        policy: "read-only",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(permissionDeletedEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        eventType: "omni.permission.deleted",
        principalIdentity: "test-user",
        message: {
          resourceAddress: "/test/path",
          policy: "read-only",
        },
      })
    );
  });

  it("should handle events with different message payloads", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    // Test with minimal message payload
    const minimalEvent: OmniEvent = {
      eventType: "omni.storage.created",
      principalIdentity: "test-user",
      occurredAt: new Date().toISOString(),
      publishedAt: new Date().toISOString(),
      message: {
        resourceIdentity: "test-resource",
        resourceAddress: "/test/path",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(minimalEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(minimalEvent);

    // Test with permission event payload
    const permissionEvent: OmniEvent = {
      eventType: "omni.permission.created",
      principalIdentity: "test-user",
      occurredAt: new Date().toISOString(),
      publishedAt: new Date().toISOString(),
      message: {
        resourceAddress: "/test/path",
        policy: "read-only",
      },
    };

    onMessageHandler?.({
      event: "message",
      data: JSON.stringify(permissionEvent),
    } as EventSourceMessage);

    expect(onMessage).toHaveBeenCalledWith(permissionEvent);
  });

  it("should handle reconnection with valid token", () => {
    const onMessage = vi.fn();
    const client = service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    onMessageHandler?.({
      event: "connected",
      data: JSON.stringify({
        message: "Connected",
        reconnect_token: "test-reconnect-token",
      }),
    } as EventSourceMessage);

    // Close and reconnect
    client.close();
    client.connect("/test");

    const url = new URL(`http://test.com/${(mockCreateEventSource.mock.calls[1][0] as EventSourceOptions).url}`);
    expect(url.searchParams.get("reconnect_token")).toBe("test-reconnect-token");
  });

  it("should handle expired token", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    // Simulate error event with expired token
    onMessageHandler?.({
      event: "error",
      data: JSON.stringify({
        status_code: 401,
      }),
    } as EventSourceMessage);

    // Verify that the client was recreated
    expect(mockCreateEventSource).toHaveBeenCalledTimes(2);
  });

  it("should handle invalid reconnect token", () => {
    const onMessage = vi.fn();
    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    onMessageHandler?.({
      event: "error",
      data: JSON.stringify({
        status_code: 404,
      }),
    } as EventSourceMessage);

    // Verify that the client was recreated without reconnect token
    expect(mockCreateEventSource).toHaveBeenCalledTimes(2);
    const url = new URL(`http://test.com/${(mockCreateEventSource.mock.calls[1][0] as EventSourceOptions).url}`);
    expect(url.searchParams.has("reconnect_token")).toBe(false);
  });

  it("should handle authentication error events", () => {
    const onMessage = vi.fn();
    const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => undefined);

    service.createEventSource({
      path: "/test",
      onMessage,
    });

    const options = mockCreateEventSource.mock.calls[0][0] as EventSourceOptions;
    const onMessageHandler = options.onMessage;

    onMessageHandler?.({
      event: "error",
      data: JSON.stringify({
        status_code: 401,
      }),
    } as EventSourceMessage);

    expect(mockCreateEventSource).toHaveBeenCalledTimes(2);

    onMessageHandler?.({
      event: "error",
      data: JSON.stringify({
        status_code: 403,
      }),
    } as EventSourceMessage);

    expect(mockCreateEventSource).toHaveBeenCalledTimes(3);

    consoleLogSpy.mockRestore();
  });

  it("should handle token expired error", () => {
    const onMessage = vi.fn();
    const onDisconnect = vi.fn();

    connectErrorResponseCode = 401;

    expect(() => {
      service.createEventSource({
        path: "/test",
        onMessage,
        onDisconnect,
      });
    }).toThrow("Reconnect attempts exceeded. Please refresh the page.");
  });

  it("should handle stale reconnect token error", () => {
    const onMessage = vi.fn();
    const onDisconnect = vi.fn();

    connectErrorResponseCode = 404;

    expect(() => {
      service.createEventSource({
        path: "/test",
        onMessage,
        onDisconnect,
      });
    }).toThrow("Reconnect attempts exceeded. Please refresh the page.");
  });
});
