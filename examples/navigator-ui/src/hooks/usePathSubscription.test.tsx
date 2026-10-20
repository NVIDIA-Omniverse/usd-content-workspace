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

import { ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { OmniEvent } from "@/services/notificationService";
import { Connection } from "@/state/appStore";
import { createTestAuth, createTestQueryClient } from "@/test-utils/utils";
import { usePathSubscription } from "./usePathSubscription";

const mockEvent: OmniEvent = {
  eventType: "omni.storage.created",
  principalIdentity: "test-user",
  occurredAt: "2024-03-20T12:00:00Z",
  publishedAt: "2024-03-20T12:00:01Z",
  message: {
    resourceIdentity: "test-resource",
    resourceAddress: "/test/path/file.txt",
  },
};

const endpoint = "http://storage.local";
const currentFolder = "file-storage://fileservice/test-folder";
const notificationEndpoint = "http://notification.local";

const mockAuth = createTestAuth();

const activeConnection: Connection = {
  endpoint,
  currentFolder,
  selectedPaths: new Set(),
  info: {
    auth: mockAuth,
    discovery: "http://discovery.local",
  },
};

const mockUseActiveConnection = vi.fn().mockReturnValue(activeConnection);

vi.mock("@/state/appStore", async () => {
  const actual = await vi.importActual("@/state/appStore");
  return {
    ...actual,
    useActiveConnection: () => mockUseActiveConnection() as Connection | undefined,
  };
});

vi.mock("@/hooks/useServiceApi", () => ({
  useServiceApi: () => ({
    getApi: vi.fn().mockResolvedValue({
      url: `${notificationEndpoint}/api/v1beta`,
      baseUrl: notificationEndpoint,
      versionPath: "/api/v1beta",
    }),
    createServiceApi: vi.fn(),
  }),
}));

class MockEventSourceClient {
  private onMessage: ((event: OmniEvent) => void) | undefined;
  private onConnectionStateChange: ((state: any) => void) | undefined;
  private closed = false;

  constructor(options: { onMessage?: (event: OmniEvent) => void; onConnectionStateChange?: (state: any) => void }) {
    this.onMessage = options.onMessage;
    this.onConnectionStateChange = options.onConnectionStateChange;

    setTimeout(() => {
      if (this.onConnectionStateChange) {
        this.onConnectionStateChange("Connected");
      }
    }, 0);
  }

  connect(_path: string) {
    this.closed = false;
    if (this.onConnectionStateChange) {
      this.onConnectionStateChange("Connected");
    }
  }

  close() {
    this.closed = true;
    if (this.onConnectionStateChange) {
      this.onConnectionStateChange("Disconnected");
    }
  }

  simulateEvent(event: OmniEvent) {
    if (!this.closed && this.onMessage) {
      this.onMessage(event);
    }
  }

  simulateConnectionStateChange(state: "Connected" | "Disconnected") {
    if (this.onConnectionStateChange) {
      this.onConnectionStateChange(state);
    }
  }

  updateHandler(handler?: (event: OmniEvent) => void) {
    this.onMessage = handler;
  }
}

const mockNotificationService = {
  createEventSource: vi
    .fn()
    .mockImplementation(
      (options: { onMessage?: (event: OmniEvent) => void; onConnectionStateChange?: (state: any) => void }) => {
        const client = new MockEventSourceClient(options);
        mockClients.push(client);
        return client;
      }
    ),
};

const mockClients: MockEventSourceClient[] = [];

vi.mock("@/services/api", () => ({
  create: vi.fn().mockReturnValue({}),
  getApiOptions: vi.fn().mockReturnValue({}),
}));

vi.mock("@/services/notificationService", async () => {
  const actual = await vi.importActual("@/services/notificationService");
  return {
    ...actual,
    notificationService: () => mockNotificationService,
  };
});

const wrapper = ({ children }: { children: ReactNode }) => {
  return <QueryClientProvider client={createTestQueryClient()}>{children}</QueryClientProvider>;
};

describe("usePathSubscription", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockClients.length = 0;
  });

  it("should handle multiple events through onEvent callbacks on one connection", async () => {
    const { result } = renderHook(() => usePathSubscription(activeConnection), { wrapper });

    await waitFor(() => {
      expect(mockNotificationService.createEventSource).toHaveBeenCalledTimes(1);
    });

    const events = [
      {
        ...mockEvent,
        eventType: "omni.storage.created",
        message: { ...mockEvent.message, resourceAddress: "/test/path/file1.txt" },
      },
      {
        ...mockEvent,
        eventType: "omni.storage.updated",
        message: { ...mockEvent.message, resourceAddress: "/test/path/file2.txt" },
      },
      {
        ...mockEvent,
        eventType: "omni.storage.deleted",
        message: { ...mockEvent.message, resourceAddress: "/test/path/file3.txt" },
      },
    ];

    const eventHandler = vi.fn();
    const secondEventHandler = vi.fn();
    result.current.onEvent(eventHandler);
    result.current.onEvent(secondEventHandler);

    for (const event of events) {
      mockClients[0].simulateEvent(event as OmniEvent);
    }

    await waitFor(() => {
      expect(eventHandler).toHaveBeenCalledTimes(events.length);
      expect(secondEventHandler).toHaveBeenCalledTimes(events.length);
      events.forEach((event, index) => {
        expect(eventHandler).toHaveBeenNthCalledWith(index + 1, expect.objectContaining(event));
        expect(secondEventHandler).toHaveBeenNthCalledWith(index + 1, expect.objectContaining(event));
      });
    });
  });

  it("should reconnect when path changes", async () => {
    const initialConnection = activeConnection;
    const { result, rerender } = renderHook(
      ({ connection }: { connection: Connection }) => usePathSubscription(connection),
      {
        wrapper,
        initialProps: { connection: initialConnection },
      }
    );

    const eventHandler = vi.fn();
    result.current.onEvent(eventHandler);

    const newPath = "file-storage://fileservice/new-folder";
    const newConnection = {
      ...activeConnection,
      currentFolder: newPath,
    };

    rerender({ connection: newConnection });

    await waitFor(() => {
      expect(mockNotificationService.createEventSource).toHaveBeenCalledWith(
        expect.objectContaining({
          path: newPath,
        })
      );
    });
  });

  it("should not handle events after unsubscribing", async () => {
    const { result } = renderHook(() => usePathSubscription(activeConnection), { wrapper });

    await waitFor(() => {
      expect(mockNotificationService.createEventSource).toHaveBeenCalledTimes(1);
    });

    const eventHandler = vi.fn();
    const unsubscribe = result.current.onEvent(eventHandler);

    unsubscribe();
    mockClients[0].simulateEvent(mockEvent);

    expect(eventHandler).not.toHaveBeenCalled();
  });

  it("should emit connection state changes", async () => {
    const onConnectionStateChange = vi.fn();
    const mockClientInstance = new MockEventSourceClient({
      onMessage: vi.fn(),
      onConnectionStateChange,
    });

    await waitFor(() => {
      expect(onConnectionStateChange).toHaveBeenCalledWith("Connected");
    });

    mockClientInstance.simulateConnectionStateChange("Disconnected");
    expect(onConnectionStateChange).toHaveBeenCalledWith("Disconnected");

    mockClientInstance.connect("/new/path");
    expect(onConnectionStateChange).toHaveBeenCalledWith("Connected");

    mockClientInstance.close();
    expect(onConnectionStateChange).toHaveBeenCalledWith("Disconnected");
  });
});
