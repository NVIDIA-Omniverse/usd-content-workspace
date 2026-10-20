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
import { create } from "@/services/api";
import { noopLogger, type Logger } from "@/output";
import { runLs } from "./ls";

const folderAddress = "omniverse://nucleus/Library/";
const encoded = encodeURIComponent(folderAddress);

// Top-level address used for short-name resolution tests
const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const topLevelEncoded = encodeURIComponent(topLevelAddress);

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
    `https://storage.example.com/v1alpha/filefolder/list/${encoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: ["omniverse://nucleus/Library/Sub/"],
        sub_resource_addresses: ["omniverse://nucleus/Library/scene.usd"],
        next_continuation_handle: null,
      }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/filefolder/list/${topLevelEncoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: [
          "https://mybucket.blob.core.windows.net/mybucket/Assets/",
        ],
        sub_resource_addresses: [],
        next_continuation_handle: null,
      }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/filefolder/liststat/${encoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: ["omniverse://nucleus/Library/Sub/"],
        entries: [
          {
            resource_address: "omniverse://nucleus/Library/scene.usd",
            resource_identity: "id1",
            metadata: {
              data_object_size: 1024 * 1024,
              last_modified_timestamp: "2026-04-10T09:15:00Z",
            },
          },
        ],
        next_continuation_handle: null,
      }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runLs", () => {
  const api = create("https://storage.example.com");

  it("prints folder and file names", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, folderAddress, { long: false, json: false });
    const out = lines.join("");
    expect(out).toContain("Sub/");
    expect(out).toContain("scene.usd");
  });

  it("prints long format with size and date", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, folderAddress, { long: true, json: false });
    const out = lines.join("");
    expect(out).toContain("1.0 MB");
    expect(out).toContain("scene.usd");
  });

  it("outputs JSON when json flag set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, folderAddress, { long: false, json: true });
    const parsed = JSON.parse(lines.join(""));
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed).toContainEqual({
      name: "Sub",
      type: "folder",
      resourceAddress: "omniverse://nucleus/Library/Sub/",
    });
    expect(parsed).toContainEqual({
      name: "scene.usd",
      type: "file",
      resourceAddress: "omniverse://nucleus/Library/scene.usd",
    });
  });

  it("lists top-level bucket names when no path and no cwd", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, undefined, { long: false, json: false });
    expect(lines.join("")).toContain("mybucket/");
  });

  it("outputs JSON array of bucket names when no path, no cwd, json flag set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, undefined, { long: false, json: true });
    const parsed = JSON.parse(lines.join(""));
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed).toContainEqual({
      name: "mybucket",
      type: "folder",
      resourceAddress: topLevelAddress,
    });
  });

  it("outputs JSON array with size and date when long and json flags set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runLs(api, folderAddress, { long: true, json: true });
    interface LsLongJsonEntry {
      name: string;
      type: "file" | "folder";
      size?: number;
      modifiedAt?: string;
    }
    const parsed = JSON.parse(lines.join("")) as LsLongJsonEntry[];
    expect(Array.isArray(parsed)).toBe(true);
    const file = parsed.find((e) => e.name === "scene.usd");
    if (!file) throw new Error("Expected scene.usd in listing");
    expect(file.type).toBe("file");
    expect(file.size).toBe(1024 * 1024);
    expect(parsed.find((entry) => entry.type === "folder")).toMatchObject({
      name: "Sub",
      type: "folder",
    });
  });

  it("resolves a short name to its full address", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    // "mybucket" is the short name for "https://mybucket.blob.core.windows.net/mybucket/"
    await runLs(api, "mybucket", { long: false, json: false });
    expect(lines.join("")).toContain("Assets/");
  });

  it("throws a helpful error for an unknown short name", async () => {
    await expect(
      runLs(api, "unknown-bucket", { long: false, json: false }),
    ).rejects.toThrow('Unknown path "unknown-bucket"');
  });

  it("lists cwd when no path given and cwd is set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    // topLevelAddress is already registered in the server — it returns Assets/
    await runLs(
      api,
      undefined,
      { long: false, json: false },
      noopLogger(),
      topLevelAddress,
    );
    expect(lines.join("")).toContain("Assets/");
  });

  it("resolves bare name relative to cwd", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/";
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${encodeURIComponent(`${cwd}Library/`)}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [
              "https://mybucket.blob.core.windows.net/mybucket/Library/Sub/",
            ],
            sub_resource_addresses: [
              "https://mybucket.blob.core.windows.net/mybucket/Library/scene.usd",
            ],
            next_continuation_handle: null,
          }),
      ),
    );
    await runLs(
      api,
      "Library/",
      { long: false, json: false },
      noopLogger(),
      cwd,
    );
    expect(lines.join("")).toContain("Sub/");
    expect(lines.join("")).toContain("scene.usd");
  });

  it("logs page fetch when logger provided", async () => {
    const logs: string[] = [];
    const log: Logger = (label, msg) => logs.push(`[${label}] ${msg}`);
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await runLs(api, folderAddress, { long: false, json: false }, log);
    expect(logs).toContain(`[ls] page 1 — ${folderAddress}`);
  });
});
