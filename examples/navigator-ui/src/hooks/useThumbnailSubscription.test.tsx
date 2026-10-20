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

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OmniEvent } from "@/services/notificationService";
import { Connection } from "@/state/appStore";
import { createTestAuth } from "@/test-utils/utils";
import { useThumbnailSubscription } from "./useThumbnailSubscription";

const mockPathSubscription = {
  onEvent: vi.fn((_onNotificationEvent: (event: OmniEvent) => void) => vi.fn()),
};

const mockSetQueryData = vi.fn();
const mockQueryClient = {
  setQueryData: mockSetQueryData,
};

vi.mock("@tanstack/react-query", async () => {
  const actual = await vi.importActual("@tanstack/react-query");
  return {
    ...actual,
    useQueryClient: () => mockQueryClient,
  };
});

const mockConnection: Connection = {
  endpoint: "https://test-endpoint.com",
  currentFolder: "/test-folder",
  selectedPaths: new Set(),
  info: {
    auth: createTestAuth(),
    discovery: "https://discovery.example.com",
  },
};

const createThumbnailAssociatedEvent = (resourceAddress: string, thumbnailResourceIdentity: string): OmniEvent => ({
  eventType: "omni.thumbnail.associated",
  principalIdentity: "test-user",
  occurredAt: "2023-01-01T00:00:00Z",
  publishedAt: "2023-01-01T00:00:00Z",
  message: {
    associationTarget: {
      resourceAddress,
      resourceIdentity: `${resourceAddress}?version-id=target`,
    },
    targetType: "FILE",
    thumbnail: {
      resourceAddress: "thumbnail-address",
      resourceIdentity: thumbnailResourceIdentity,
    },
  },
});

const createStorageEvent = (resourceAddress: string): OmniEvent => ({
  eventType: "omni.storage.created",
  principalIdentity: "test-user",
  occurredAt: "2023-01-01T00:00:00Z",
  publishedAt: "2023-01-01T00:00:00Z",
  message: {
    resourceIdentity: `${resourceAddress}?version-id=storage`,
    resourceAddress,
  },
});

describe("useThumbnailSubscription", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("updates the thumbnail resource query for the association target", () => {
    const thumbnailEvent = createThumbnailAssociatedEvent("test-resource-address", "thumbnail-resource-id");

    renderHook(() => useThumbnailSubscription(mockConnection, mockPathSubscription));

    const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
    act(() => {
      eventHandler(thumbnailEvent);
    });

    expect(mockSetQueryData).toHaveBeenCalledWith(
      ["https://test-endpoint.com", "test-resource-address", "thumbnail", "resource"],
      "thumbnail-resource-id"
    );
  });

  it("ignores storage events", () => {
    const storageEvent = createStorageEvent("test-resource-address");

    renderHook(() => useThumbnailSubscription(mockConnection, mockPathSubscription));

    const eventHandler = mockPathSubscription.onEvent.mock.calls[0][0] as (event: OmniEvent) => void;
    act(() => {
      eventHandler(storageEvent);
    });

    expect(mockSetQueryData).not.toHaveBeenCalled();
  });
});
