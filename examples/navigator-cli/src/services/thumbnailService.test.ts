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
} from "vitest";
import { create } from "@/services/api";
import { thumbnailService } from "./thumbnailService";

const resourceAddress = "https://storage.example.com/objects/a.png";

const entry = {
  target_type: "FILE",
  association_target: { resource_address: resourceAddress },
  thumbnail: {
    resource_address: "https://storage.example.com/thumbnails/a-256.png",
    resource_identity:
      "https://storage.example.com/thumbnails/a-256.png?version-id=thumb",
  },
  specification: {
    type: "IMAGE_TO_IMAGE",
    width: 256,
    height: 256,
    quality: null,
  },
  create_time: "2026-04-01T12:00:00Z",
};

let requests: URL[] = [];
let body: unknown = { thumbnails: [] };
let status = 200;

const server = setupServer(
  http.get("https://thumb.example.com/v1alpha/thumbnails", ({ request }) => {
    requests.push(new URL(request.url));
    return HttpResponse.json(body as Record<string, unknown>, { status });
  }),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("thumbnailService", () => {
  const service = thumbnailService(create("https://thumb.example.com"));

  beforeEach(() => {
    requests = [];
    body = { thumbnails: [] };
    status = 200;
  });

  describe("getThumbnails", () => {
    it("requests thumbnails with association_target_resource_address only", async () => {
      await service.getThumbnails(resourceAddress);

      expect(requests).toHaveLength(1);
      expect(Object.fromEntries(requests[0].searchParams)).toEqual({
        association_target_resource_address: resourceAddress,
      });
    });

    it("adds thumbnail filters to the query", async () => {
      await service.getThumbnails(resourceAddress, {
        width: 256,
        height: 256,
        encodingFormat: "JPEG",
        transformType: "SQUARE",
        cropStrategy: "CENTER",
        specification: "IMAGE_TO_IMAGE",
      });

      expect(Object.fromEntries(requests[0].searchParams)).toEqual({
        association_target_resource_address: resourceAddress,
        width: "256",
        height: "256",
        encoding_format: "JPEG",
        transform_type: "SQUARE",
        crop_strategy: "CENTER",
        specification: "IMAGE_TO_IMAGE",
      });
    });

    it("parses snake_case JSON into camelCase thumbnail entries", async () => {
      body = { thumbnails: [entry] };

      const result = await service.getThumbnails(resourceAddress);

      expect(result.thumbnails).toEqual([
        {
          thumbnail: {
            resourceAddress: "https://storage.example.com/thumbnails/a-256.png",
            resourceIdentity:
              "https://storage.example.com/thumbnails/a-256.png?version-id=thumb",
          },
          // Passed through verbatim, including its nullable members.
          specification: entry.specification,
          createTime: "2026-04-01T12:00:00Z",
        },
      ]);
    });

    it("accepts entries without a specification or create time", async () => {
      body = {
        thumbnails: [
          {
            target_type: "FILE",
            thumbnail: entry.thumbnail,
            specification: null,
          },
        ],
      };

      const result = await service.getThumbnails(resourceAddress);

      expect(result.thumbnails).toHaveLength(1);
      expect(result.thumbnails[0].thumbnail.resourceIdentity).toBe(
        entry.thumbnail.resource_identity,
      );
    });

    it("propagates HTTP errors", async () => {
      status = 400;
      body = { source: "thumbnail", error_message: "bad request" };

      await expect(service.getThumbnails(resourceAddress)).rejects.toThrow();
    });

    it("rejects responses that fail schema validation", async () => {
      body = {
        thumbnails: [{ thumbnail: { resource_address: "only-one-field" } }],
      };

      await expect(service.getThumbnails(resourceAddress)).rejects.toThrow();
    });
  });
});
