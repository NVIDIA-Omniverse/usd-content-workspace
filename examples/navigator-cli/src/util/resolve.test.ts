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
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { create } from "@/services/api";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { resolveStoragePath } from "@/util/resolve";

const topLevel = "https://mybucket.blob.core.windows.net/mybucket/";
const cwd = `${topLevel}Library/`;

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

describe("resolveStoragePath", () => {
  const api = create("https://storage.example.com");

  it("returns full URL unchanged (absolute — contains ://)", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(
      caps,
      "omniverse://nucleus/Library/scene.usd",
    );
    expect(result).toBe("omniverse://nucleus/Library/scene.usd");
  });

  it("resolves bucket name to full URL (absolute — bucket lookup)", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(caps, "mybucket/Library/scene.usd");
    expect(result).toBe(`${topLevel}Library/scene.usd`);
  });

  it("resolves bare bucket name to bucket root", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(caps, "mybucket");
    expect(result).toBe(topLevel);
  });

  it("resolves ./ path relative to cwd", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(caps, "./scene.usd", cwd);
    expect(result).toBe(`${cwd}scene.usd`);
  });

  it("throws when ./ path used with no cwd", async () => {
    const caps = capabilitiesService(api);
    await expect(resolveStoragePath(caps, "./scene.usd")).rejects.toThrow(
      "No current folder set",
    );
  });

  it("resolves bare name relative to cwd when cwd is set", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(caps, "scene.usd", cwd);
    expect(result).toBe(`${cwd}scene.usd`);
  });

  it("throws unknown path error when bare name used with no cwd", async () => {
    const caps = capabilitiesService(api);
    await expect(resolveStoragePath(caps, "scene.usd")).rejects.toThrow(
      'Unknown path "scene.usd"',
    );
  });

  it("resolves .. relative to cwd", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(
      caps,
      "..",
      `${topLevel}Library/Assets/`,
    );
    expect(result).toBe(`${topLevel}Library/`);
  });

  it("resolves ../sibling relative to cwd", async () => {
    const caps = capabilitiesService(api);
    const result = await resolveStoragePath(
      caps,
      "../Other",
      `${topLevel}Library/Assets/`,
    );
    expect(result).toBe(`${topLevel}Library/Other`);
  });
});
