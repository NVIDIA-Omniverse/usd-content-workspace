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
import { metadataService } from "./metadataService";

const address = "omniverse://nucleus/Library/scene.usd";
const encoded = encodeURIComponent(address);

const server = setupServer(
  http.post(`https://storage.example.com/v1alpha/metadata/${encoded}`, () =>
    HttpResponse.json({
      acl: { etag: "etag-1", value: ["read", "write"] },
      tags: { etag: "etag-2", value: { author: "alice" } },
    }),
  ),
  http.put(
    `https://storage.example.com/v1alpha/metadata/${encoded}/tags`,
    () => new HttpResponse(null, { status: 204 }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("metadataService", () => {
  const api = create("https://storage.example.com");
  const service = metadataService(api);

  it("get returns acl and tags", async () => {
    const result = await service.get(address);
    expect(result.acl.etag).toBe("etag-1");
    expect(result.tags.value).toEqual({ author: "alice" });
  });

  it("setTags sends PUT without If-Match when etag is empty", async () => {
    let capturedEtag: string | null = null;
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/metadata/${encoded}/tags`,
        ({ request }) => {
          capturedEtag = request.headers.get("If-Match");
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    await service.setTags(address, { env: "prod" }, "");
    expect(capturedEtag).toBeNull();
  });

  it("setTags sends If-Match header when etag is provided", async () => {
    let capturedEtag: string | null = null;
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/metadata/${encoded}/tags`,
        ({ request }) => {
          capturedEtag = request.headers.get("If-Match");
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    await service.setTags(address, { env: "prod" }, "etag-2");
    expect(capturedEtag).toBe("etag-2");
  });
});
