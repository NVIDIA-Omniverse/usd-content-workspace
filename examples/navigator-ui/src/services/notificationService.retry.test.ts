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

import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "./api";
import { notificationService } from "./notificationService";

// Ensure we use the real `eventsource-client` implementation for this test
vi.unmock("eventsource-client");

// Counter to track how many times the notifications endpoint was requested
let requestCount = 0;

// MSW server that always simulates a network timeout
const server = setupServer(
  http.get("http://test.com/api/v1beta/events/stream", () => {
    // Increment request counter for each connection attempt
    requestCount += 1;
    return HttpResponse.error();
  }),
  // Match any query-string variants of the same endpoint
  http.get("http://test.com/api/v1beta/events/stream*", () => {
    requestCount += 1;
    return HttpResponse.error();
  })
);

describe("notificationService – connection retry limits", () => {
  beforeAll(() => server.listen());

  afterAll(() => server.close());

  beforeEach(() => {
    server.resetHandlers();
    requestCount = 0;
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("should retry the connection only 3 times when a network timeout occurs", async () => {
    const api = create("http://test.com/api/v1beta");
    const service = notificationService(api);

    // The createEventSource call triggers the first (initial) request immediately.
    // Subsequent retries are scheduled by `eventsource-client` with a 2-second back-off.
    service.createEventSource({ path: "/test" });

    // Wrap advancing timers in an expectation that an error is eventually thrown

    // 4 reconnect windows of 2s each → 8s total should exceed the limit and trigger the throw
    await vi.advanceTimersByTimeAsync(2000 * 3 + 100);

    // Verify that the endpoint was requested 4 times (initial + 3 retries)
    expect(requestCount).toBe(3);
  });
});
