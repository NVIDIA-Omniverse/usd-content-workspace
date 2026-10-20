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
import { capabilitiesService } from "./capabilitiesService";

const server = setupServer(
  http.get(
    "https://storage.example.com/v1alpha/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [
          { top_level_address: "omniverse://nucleus" },
          { top_level_address: "omniverse://backup" },
        ],
      }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("capabilitiesService", () => {
  const api = create("https://storage.example.com");
  const service = capabilitiesService(api);

  it("listTopLevelAddresses returns folder addresses with trailing slash", async () => {
    const result = await service.listTopLevelAddresses();
    expect(result.subfolderAddresses).toContain("omniverse://nucleus/");
    expect(result.subfolderAddresses).toContain("omniverse://backup/");
    expect(result.isRoot).toBe(true);
    expect(result.folderAddress).toBe("/");
    expect(result.entries).toEqual([]);
    expect(result.nextContinuationHandle).toBeNull();
  });
});
