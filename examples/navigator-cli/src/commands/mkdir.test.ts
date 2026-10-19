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
import { noopLogger } from "@/output";
import { create } from "@/services/api";
import { runMkdir } from "./mkdir";

const address = "omniverse://nucleus/Library/NewFolder/";
const encoded = encodeURIComponent(address);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortAddress = `${topLevelAddress}NewFolder/`;
const shortAddressEncoded = encodeURIComponent(shortAddress);

let created = false;

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
  http.put(`https://storage.example.com/v1alpha/filefolder/${encoded}`, () => {
    created = true;
    return new HttpResponse(null, { status: 204 });
  }),
  http.put(
    `https://storage.example.com/v1alpha/filefolder/${shortAddressEncoded}`,
    () => {
      created = true;
      return new HttpResponse(null, { status: 204 });
    },
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  created = false;
});

describe("runMkdir", () => {
  const api = create("https://storage.example.com");

  it("createFolder calls PUT", async () => {
    await runMkdir(api, address);
    expect(created).toBe(true);
  });

  it("resolves a short name to its full address", async () => {
    await runMkdir(api, "mybucket/NewFolder/");
    expect(created).toBe(true);
  });

  it("resolves bare name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/";
    await runMkdir(api, "NewFolder/", noopLogger(), cwd);
    expect(created).toBe(true);
  });
});
