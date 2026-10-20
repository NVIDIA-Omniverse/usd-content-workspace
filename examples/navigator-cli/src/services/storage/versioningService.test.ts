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
import { versioningService } from "./versioningService";

const fileAddress = "omniverse://nucleus/Library/scene.usd";
const encoded = encodeURIComponent(fileAddress);

const item1 = {
  resource_info: {
    resource_identity: "omniverse://nucleus/Library/scene.usd?version=2",
    metadata: {
      data_object_size: 1258291,
      last_modified_timestamp: "2026-04-10T09:15:00Z",
    },
  },
};
const item2 = {
  resource_info: {
    resource_identity: "omniverse://nucleus/Library/scene.usd?version=1",
    metadata: {
      data_object_size: 980000,
      last_modified_timestamp: "2026-03-22T14:30:00Z",
    },
  },
};

const server = setupServer();

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("versioningService", () => {
  const api = create("https://storage.example.com");
  const service = versioningService(api);

  it("lists a single page of versions", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/versioning/${encoded}/versions`,
        () =>
          HttpResponse.json({
            items: [item1, item2],
            next_continuation_handle: null,
          }),
      ),
    );
    const page = await service.list(fileAddress, null);
    expect(page.items).toHaveLength(2);
    expect(page.items[0].resourceInfo.resourceIdentity).toBe(
      "omniverse://nucleus/Library/scene.usd?version=2",
    );
    expect(page.nextContinuationHandle).toBeNull();
  });

  it("accumulates all pages in listAll", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/versioning/${encoded}/versions`,
        ({ request }) => {
          const handle = new URL(request.url).searchParams.get(
            "continuation_handle",
          );
          if (!handle) {
            return HttpResponse.json({
              items: [item1],
              next_continuation_handle: "page2",
            });
          }
          return HttpResponse.json({
            items: [item2],
            next_continuation_handle: null,
          });
        },
      ),
    );
    const items = await service.listAll(fileAddress);
    expect(items).toHaveLength(2);
    expect(items[0].resourceInfo.resourceIdentity).toBe(
      "omniverse://nucleus/Library/scene.usd?version=2",
    );
    expect(items[1].resourceInfo.resourceIdentity).toBe(
      "omniverse://nucleus/Library/scene.usd?version=1",
    );
  });
});
