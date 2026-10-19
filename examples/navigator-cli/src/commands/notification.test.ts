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
  describe,
  expect,
  it,
  vi,
  beforeEach,
  afterEach,
  type MockInstance,
} from "vitest";
import { create } from "@/services/api";
import { noopLogger } from "@/output";
import { formatNotificationEvent, runNotification } from "./notification";

vi.mock("@/services/notificationService", () => ({
  notificationService: vi.fn(() => ({
    createEventSource: vi.fn(
      (opts: {
        path: string;
        eventTypes: string[];
        onMessage?: (event: unknown) => void;
        onConnected?: () => void;
        onError?: (error: Error) => void;
      }) => {
        // Immediately fire onConnected and one onMessage, then stop
        setTimeout(() => {
          opts.onConnected?.();
          opts.onMessage?.({
            event_type: "omni.storage.created",
            principal_identity: "alice@example.com",
            occurred_at: "2026-04-20T10:00:00Z",
            published_at: "2026-04-20T10:00:01Z",
            message: { resource_address: "/bucket/folder/file.usd" },
          });
        }, 0);
        return { close: vi.fn() };
      },
    ),
  })),
}));

const api = create("https://storage.example.com", {
  accessToken: "test-token",
});

describe("formatNotificationEvent", () => {
  it("formats event as human-readable line with resourceAddress", () => {
    const event = {
      event_type: "omni.storage.created",
      principal_identity: "alice@example.com",
      occurred_at: "2026-04-20T10:00:00Z",
      message: { resource_address: "/bucket/folder/file.usd" },
    };
    const result = formatNotificationEvent(event, false);
    expect(result).toContain("[omni.storage.created]");
    expect(result).toContain("/bucket/folder/file.usd");
    expect(result).toContain("alice@example.com");
    expect(result).toContain("2026-04-20T10:00:00Z");
  });

  it("falls back to full message JSON when no resource_address", () => {
    const event = {
      event_type: "omni.thumbnail.associated",
      principal_identity: "bob@example.com",
      occurred_at: "2026-04-20T11:00:00Z",
      message: { thumbnail_id: "thumb-123", asset_id: "asset-456" },
    };
    const result = formatNotificationEvent(event, false);
    expect(result).toContain("[omni.thumbnail.associated]");
    expect(result).toContain("thumb-123");
  });

  it("formats event as one JSON line when json is true", () => {
    const event = {
      event_type: "omni.storage.deleted",
      principal_identity: "alice@example.com",
      occurred_at: "2026-04-20T10:00:00Z",
      message: { resource_address: "/bucket/folder/file.usd" },
    };
    const result = formatNotificationEvent(event, true);
    expect(result).not.toContain("\n");
    const parsed: { event_type: string } = JSON.parse(result) as {
      event_type: string;
    };
    expect(parsed.event_type).toBe("omni.storage.deleted");
  });
});

describe("runNotification", () => {
  let stdoutSpy: MockInstance<typeof process.stdout.write>;
  let stderrSpy: MockInstance<typeof process.stderr.write>;

  beforeEach(() => {
    stdoutSpy = vi
      .spyOn(process.stdout, "write")
      .mockImplementation(() => true);
    stderrSpy = vi
      .spyOn(process.stderr, "write")
      .mockImplementation(() => true);
  });

  afterEach(() => vi.restoreAllMocks());

  it("throws when cwd is undefined", async () => {
    await expect(
      runNotification(
        api,
        undefined,
        ["omni.storage.created"],
        { json: false },
        noopLogger(),
      ),
    ).rejects.toThrow("no current folder set");
  });

  it("writes status line to stderr on connect then event to stdout, then resolves on abort", async () => {
    const controller = new AbortController();

    const promise = runNotification(
      api,
      "/bucket/folder/",
      ["omni.storage.created"],
      { json: false },
      noopLogger(),
      controller.signal,
    );

    await new Promise((r) => setTimeout(r, 10));
    controller.abort();
    await promise;

    const stderrOutput = stderrSpy.mock.calls.map((c) => c[0]).join("");
    expect(stderrOutput).toContain("Watching /bucket/folder/");
    expect(stderrOutput).toContain("omni.storage.created");

    const stdoutOutput = stdoutSpy.mock.calls.map((c) => c[0]).join("");
    expect(stdoutOutput).toContain("[omni.storage.created]");
    expect(stdoutOutput).toContain("/bucket/folder/file.usd");
  });

  it("writes NDJSON to stdout when json is true", async () => {
    const controller = new AbortController();

    const promise = runNotification(
      api,
      "/bucket/folder/",
      ["omni.storage.created"],
      { json: true },
      noopLogger(),
      controller.signal,
    );

    await new Promise((r) => setTimeout(r, 10));
    controller.abort();
    await promise;

    const stdoutOutput = stdoutSpy.mock.calls.map((c) => c[0]).join("");
    const parsed: { event_type: string } = JSON.parse(stdoutOutput.trim()) as {
      event_type: string;
    };
    expect(parsed.event_type).toBe("omni.storage.created");

    const stderrOutput = stderrSpy.mock.calls.map((c) => c[0]).join("");
    expect(stderrOutput).not.toContain("Watching /bucket/folder/");
  });
});
