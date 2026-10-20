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
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { create } from "@/services/api";
import { runCd } from "./cd";

// configState is captured by reference in vi.mock — mutations in tests are visible to the mock
const configState = {
  config: {
    activeHost: "test",
    hosts: {
      test: {
        discoveryUrl: "https://discovery.example.com",
        storageUrl: "https://storage.example.com",
        authUrl: "https://auth.example.com",
        cwd: undefined as string | undefined,
        accessToken: "old",
        refreshToken: "old-refresh",
      },
    },
  },
};

vi.mock("@/config", () => ({
  readConfig: vi.fn(() => structuredClone(configState.config)),
  updateConfig: vi.fn((update: (config: typeof configState.config) => void) => {
    update(configState.config);
    return Promise.resolve();
  }),
  getActiveHost: vi.fn(() => configState.config.hosts.test),
}));

const topLevel = "https://mybucket.blob.core.windows.net/mybucket/";

const server = setupServer(
  http.get(
    "https://storage.example.com/v1alpha/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [
          {
            top_level_address:
              "https://mybucket.blob.core.windows.net/mybucket",
          },
        ],
      }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());
beforeEach(() => {
  configState.config.hosts.test.cwd = undefined;
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
});

describe("runCd", () => {
  it("retains credentials refreshed while resolving the folder", async () => {
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/capabilities/top-level-addresses",
        () => {
          configState.config.hosts.test.accessToken = "refreshed";
          configState.config.hosts.test.refreshToken = "rotated";
          return HttpResponse.json({
            items: [{ top_level_address: topLevel }],
          });
        },
      ),
    );
    await runCd(
      create("https://storage.example.com"),
      "mybucket/Library/",
      "test",
    );
    expect(configState.config.hosts.test).toMatchObject({
      accessToken: "refreshed",
      refreshToken: "rotated",
      cwd: `${topLevel}Library/`,
    });
  });

  const api = create("https://storage.example.com");

  it("sets cwd to resolved bucket sub-path", async () => {
    await runCd(api, "mybucket/Library/", "test");
    expect(configState.config.hosts.test.cwd).toBe(`${topLevel}Library/`);
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "Now in mybucket/Library/",
    );
  });

  it("sets cwd to bucket root when no sub-path given", async () => {
    await runCd(api, "mybucket", "test");
    expect(configState.config.hosts.test.cwd).toBe(topLevel);
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "Now in mybucket/",
    );
  });

  it("clears cwd when called with no path argument", async () => {
    configState.config.hosts.test.cwd = `${topLevel}Library/`;
    await runCd(api, undefined, "test");
    expect(configState.config.hosts.test.cwd).toBeUndefined();
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "now at root",
    );
  });

  it("prints a message when no cwd is set and clearing is attempted", async () => {
    await runCd(api, undefined, "test");
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "No current folder set",
    );
  });

  it("clears cwd when called with '/'", async () => {
    configState.config.hosts.test.cwd = `${topLevel}Library/`;
    await runCd(api, "/", "test");
    expect(configState.config.hosts.test.cwd).toBeUndefined();
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "now at root",
    );
  });

  it("throws for an unknown bucket name", async () => {
    await expect(runCd(api, "nonexistent", "test")).rejects.toThrow(
      'Unknown path "nonexistent"',
    );
  });

  it("resolves bare folder name relative to current cwd", async () => {
    configState.config.hosts.test.cwd = `${topLevel}Library/`;
    await runCd(api, "Assets", "test");
    expect(configState.config.hosts.test.cwd).toBe(
      `${topLevel}Library/Assets/`,
    );
    expect(vi.mocked(process.stdout.write).mock.calls.join("")).toContain(
      "Now in mybucket/Library/Assets/",
    );
  });
});
