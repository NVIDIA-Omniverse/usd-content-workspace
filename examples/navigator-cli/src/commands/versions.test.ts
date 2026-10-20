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
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { noopLogger } from "@/output";
import { create } from "@/services/api";
import { runVersionsList, runVersionsRestore } from "./versions";

const fileAddress = "omniverse://nucleus/Library/scene.usd";
const fileEncoded = encodeURIComponent(fileAddress);
const identity = "omniverse://nucleus/Library/scene.usd?version=2";
const identityEncoded = encodeURIComponent(identity);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortFile = `${topLevelAddress}Library/scene.usd`;
const shortFileEncoded = encodeURIComponent(shortFile);

const versionItems = [
  {
    resource_info: {
      resource_identity: identity,
      metadata: {
        data_object_size: 1258291,
        last_modified_timestamp: "2026-04-10T09:15:00Z",
      },
    },
  },
];

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
  http.get(
    `https://storage.example.com/v1alpha/versioning/${fileEncoded}/versions`,
    () =>
      HttpResponse.json({
        items: versionItems,
        next_continuation_handle: null,
      }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/versioning/${shortFileEncoded}/versions`,
    () =>
      HttpResponse.json({
        items: versionItems,
        next_continuation_handle: null,
      }),
  ),
  http.post(
    `https://storage.example.com/v1alpha/fileobject/by-identity/${identityEncoded}/copy`,
    () => HttpResponse.json({ resource_identity: "new-identity" }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());
afterEach(() => {
  vi.restoreAllMocks();
});

describe("runVersionsList", () => {
  const api = create("https://storage.example.com");

  it("prints version identity, size, and date", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runVersionsList(api, fileAddress, { json: false });
    const out = lines.join("");
    expect(out).toContain(identity);
    expect(out).toContain("1.2 MB");
    expect(out).toContain("2026-04-10");
  });

  it("outputs JSON array with resourceIdentity, dataObjectSize, lastModifiedTimestamp", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runVersionsList(api, fileAddress, { json: true });
    const parsed = JSON.parse(lines.join("")) as unknown[];
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed[0]).toMatchObject({
      resourceIdentity: identity,
      dataObjectSize: 1258291,
      lastModifiedTimestamp: "2026-04-10T09:15:00Z",
    });
  });

  it("resolves short path", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runVersionsList(api, "mybucket/Library/scene.usd", { json: false });
    expect(lines.join("")).toContain(identity);
  });

  it("resolves bare name relative to cwd", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    await runVersionsList(api, "scene.usd", { json: false }, noopLogger(), cwd);
    expect(lines.join("")).toContain(identity);
  });
});

describe("runVersionsRestore", () => {
  const api = create("https://storage.example.com");

  it("restores a version and prints success message", async () => {
    let postCalled = false;
    server.use(
      http.post(
        `https://storage.example.com/v1alpha/fileobject/by-identity/${identityEncoded}/copy`,
        () => {
          postCalled = true;
          return HttpResponse.json({ resource_identity: "new-identity" });
        },
      ),
    );
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runVersionsRestore(api, fileAddress, identity);
    const out = lines.join("");
    expect(postCalled).toBe(true);
    expect(out).toContain("Restored");
    expect(out).toContain(identity);
    expect(out).toContain(fileAddress);
  });

  it("resolves short path before restoring", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runVersionsRestore(api, "mybucket/Library/scene.usd", identity);
    const out = lines.join("");
    expect(out).toContain("Restored");
    expect(out).toContain(shortFile);
  });
});
