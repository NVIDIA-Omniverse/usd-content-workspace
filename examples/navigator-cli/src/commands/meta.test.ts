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
import { runMeta } from "./meta";

const address = "omniverse://nucleus/Library/scene.usd";
const encoded = encodeURIComponent(address);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortAddress = `${topLevelAddress}Library/scene.usd`;
const shortAddressEncoded = encodeURIComponent(shortAddress);

const statHeaders = {
  "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
    data_object_size: 4200000,
    last_modified_timestamp: "2026-04-10T09:15:00Z",
  }),
  "X-Nvidia-Omniverse-Storage-Resource-Identity": "identity-abc",
};

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
  http.head(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    () => new HttpResponse(null, { status: 204, headers: statHeaders }),
  ),
  http.head(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortAddressEncoded}`,
    () => new HttpResponse(null, { status: 204, headers: statHeaders }),
  ),
  http.post("https://storage.example.com/v1alpha/metadata/:address", () =>
    HttpResponse.json({
      acl: { etag: "acl-1", value: ["read", "write"] },
      tags: { etag: "tags-1", value: { project: "demo" } },
    }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runMeta", () => {
  const api = create("https://storage.example.com");

  it("prints stat info to stdout", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runMeta(api, address, { json: false });
    const out = lines.join("");
    expect(out).toContain("identity-abc");
    expect(out).toContain("scene.usd");
  });

  it("outputs JSON when json flag set", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runMeta(api, address, { json: true });
    const parsed = JSON.parse(lines.join("")) as {
      resourceIdentity: string;
      tags: { value: Record<string, string> };
    };
    expect(parsed.resourceIdentity).toBe("identity-abc");
    expect(parsed.tags.value.project).toBe("demo");
  });

  it("resolves a short name to its full address", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runMeta(api, "mybucket/Library/scene.usd", { json: false });
    expect(lines.join("")).toContain("identity-abc");
  });

  it("resolves bare name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runMeta(api, "scene.usd", { json: false }, cwd);
    expect(lines.join("")).toContain("identity-abc");
  });

  it("shows folder metadata without a file stat", async () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    await runMeta(api, "mybucket/Library/", { json: true });
    const parsed = JSON.parse(lines.join("")) as {
      type: string;
      size: number | null;
      tags: { value: Record<string, string> };
    };
    expect(parsed.type).toBe("folder");
    expect(parsed.size).toBeNull();
    expect(parsed.tags.value.project).toBe("demo");
  });
});
