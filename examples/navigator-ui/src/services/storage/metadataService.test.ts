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

import { beforeEach, describe, expect, it, vi } from "vitest";
import { MetadataApi, metadataService, PathMetadata } from "@/services/storage/metadataService";

describe("metadataService", () => {
  const resourceAddress = "http://test-server/bucket1/asset.usd";
  const encodedAddress = encodeURIComponent(resourceAddress);

  interface RequestOptions {
    body: string;
    headers?: Headers;
  }
  type MockRequest = (path: string, options: RequestOptions) => Promise<{ json: () => Promise<unknown> }>;

  const mockPost = vi.fn<MockRequest>();
  const mockPut = vi.fn<MockRequest>();

  const parseBody = (body: string) => JSON.parse(body) as { requests: Record<string, unknown>[] };

  const createApi = (apiVersion?: string): MetadataApi =>
    ({ serviceUrl: "http://test-service", apiVersion, post: mockPost, put: mockPut }) as unknown as MetadataApi;

  const respondWith = (body: unknown) => ({ json: () => Promise.resolve(body) });

  // The same resource as each API version reports it.
  const v1alphaBody = {
    acl: { etag: "acl-etag", value: ["read", "write"] },
    tags: { etag: "tags-etag", value: { project: "demo" } },
    thumbnail_task: { etag: "thumbnail-etag", value: { state: "done" } },
  };

  const v1betaBody = {
    results: [
      { user_metadata_key: "acl", user_metadata: { etag: "acl-etag", value: ["read", "write"] } },
      { user_metadata_key: "tags", user_metadata: { etag: "tags-etag", value: { project: "demo" } } },
      { user_metadata_key: "thumbnail_task", user_metadata: { etag: "thumbnail-etag", value: { state: "done" } } },
    ],
    next_continuation_handle: null,
  };

  const expectedMetadata: PathMetadata = {
    acl: { etag: "acl-etag", value: ["read", "write"] },
    tags: { etag: "tags-etag", value: { project: "demo" } },
    thumbnails: [{ state: "done" }],
  };

  beforeEach(() => {
    mockPost.mockReset();
    mockPut.mockReset();
  });

  describe("v1alpha", () => {
    it("should read every key through the single-resource route", async () => {
      mockPost.mockResolvedValueOnce(respondWith(v1alphaBody));

      await expect(metadataService(createApi("v1alpha")).get(resourceAddress)).resolves.toEqual(expectedMetadata);
      expect(mockPost).toHaveBeenCalledWith(`metadata/${encodedAddress}`, { body: "[]" });
    });

    it("should default to v1alpha when the api instance carries no version", async () => {
      mockPost.mockResolvedValueOnce(respondWith(v1alphaBody));

      await metadataService(createApi()).get(resourceAddress);
      expect(mockPost).toHaveBeenCalledWith(`metadata/${encodedAddress}`, { body: "[]" });
    });

    it("should update tags through the single-key route with an If-Match header", async () => {
      mockPut.mockResolvedValueOnce(respondWith({}));

      await metadataService(createApi("v1alpha")).setTags(resourceAddress, { project: "demo" }, "tags-etag");

      const [path, options] = mockPut.mock.calls[0];
      expect(path).toBe(`metadata/${encodedAddress}/tags`);
      expect(options.body).toBe(JSON.stringify({ project: "demo" }));
      expect(options.headers?.get("If-Match")).toBe("tags-etag");
    });

    it("should omit the If-Match header when no etag is known", async () => {
      mockPut.mockResolvedValueOnce(respondWith({}));

      await metadataService(createApi("v1alpha")).setTags(resourceAddress, { project: "demo" }, "");

      expect(mockPut.mock.calls[0][1].headers?.get("If-Match")).toBeNull();
    });
  });

  describe("v1beta", () => {
    it("should read every key through the query route", async () => {
      mockPost.mockResolvedValueOnce(respondWith(v1betaBody));

      await expect(metadataService(createApi("v1beta")).get(resourceAddress)).resolves.toEqual(expectedMetadata);
      expect(mockPost).toHaveBeenCalledWith("metadata/query", {
        body: JSON.stringify({ resource_location: { resource_address: resourceAddress } }),
      });
    });

    it("should follow the continuation handle until the query is exhausted", async () => {
      mockPost
        .mockResolvedValueOnce(respondWith({ results: [v1betaBody.results[0]], next_continuation_handle: "next-page" }))
        .mockResolvedValueOnce(respondWith({ results: v1betaBody.results.slice(1), next_continuation_handle: null }));

      await expect(metadataService(createApi("v1beta")).get(resourceAddress)).resolves.toEqual(expectedMetadata);
      expect(mockPost).toHaveBeenCalledTimes(2);
      expect(mockPost.mock.calls[1][1].body).toBe(
        JSON.stringify({
          resource_location: { resource_address: resourceAddress },
          continuation_handle: "next-page",
        })
      );
    });

    it("should skip entries the service returns without a value", async () => {
      mockPost.mockResolvedValueOnce(
        respondWith({ results: [{ user_metadata_key: "tags", user_metadata: null }], next_continuation_handle: null })
      );

      await expect(metadataService(createApi("v1beta")).get(resourceAddress)).resolves.toEqual({
        acl: { etag: "", value: ["read", "write"] },
        tags: { etag: "", value: {} },
        thumbnails: [],
      });
    });

    it("should update tags through the batch route with the expected etag in the entry", async () => {
      mockPut.mockResolvedValueOnce(
        respondWith({ results: [{ request_index: 0, outcome: { Success: { etag: "new-etag" } } }] })
      );

      await metadataService(createApi("v1beta")).setTags(resourceAddress, { project: "demo" }, "tags-etag");

      const [path, options] = mockPut.mock.calls[0];
      expect(path).toBe("metadata");
      expect(parseBody(options.body)).toEqual({
        requests: [
          {
            resource_location: { resource_address: resourceAddress },
            user_metadata_key: "tags",
            user_metadata: { project: "demo" },
            expected_etag: "tags-etag",
          },
        ],
      });
    });

    it("should omit the expected etag when none is known", async () => {
      mockPut.mockResolvedValueOnce(respondWith({ results: [] }));

      await metadataService(createApi("v1beta")).setTags(resourceAddress, { project: "demo" }, "");

      expect(parseBody(mockPut.mock.calls[0][1].body).requests[0]).not.toHaveProperty("expected_etag");
    });

    // The batch route answers HTTP 200 for a conflict, so the per-entry error has to be raised.
    it("should reject when the batch entry fails", async () => {
      mockPut.mockResolvedValueOnce(
        respondWith({
          results: [
            {
              request_index: 0,
              outcome: { Error: { code: "failed_precondition", message: "metadata was modified" } },
            },
          ],
        })
      );

      await expect(
        metadataService(createApi("v1beta")).setTags(resourceAddress, { project: "demo" }, "stale-etag")
      ).rejects.toThrow("failed_precondition - metadata was modified");
    });
  });

  it("should parse both API versions into the same result", async () => {
    mockPost.mockResolvedValueOnce(respondWith(v1alphaBody));
    const fromV1Alpha = await metadataService(createApi("v1alpha")).get(resourceAddress);

    mockPost.mockResolvedValueOnce(respondWith(v1betaBody));
    const fromV1Beta = await metadataService(createApi("v1beta")).get(resourceAddress);

    expect(fromV1Beta).toEqual(fromV1Alpha);
  });
});
