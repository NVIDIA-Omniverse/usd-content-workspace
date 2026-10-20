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
import type { ApiInstance } from "@/services/api";
import { thumbnailService } from "@/services/thumbnailService";

describe("thumbnailService", () => {
  const mockGet = vi.fn();
  const mockApi: ApiInstance = {
    get: mockGet,
    serviceUrl: "http://thumbnail-service",
  } as unknown as ApiInstance;

  beforeEach(() => {
    mockGet.mockReset();
  });

  describe("getThumbnails", () => {
    it("requests thumbnails with association_target_resource_address", async () => {
      mockGet.mockResolvedValueOnce({
        json: () =>
          Promise.resolve({
            thumbnails: [],
          }),
      });

      const service = thumbnailService(mockApi);
      await service.getThumbnails("https://storage.example.com/objects/a.png");

      expect(mockGet).toHaveBeenCalledTimes(1);
      expect(mockGet).toHaveBeenCalledWith(
        "thumbnails?association_target_resource_address=https%3A%2F%2Fstorage.example.com%2Fobjects%2Fa.png",
        expect.objectContaining({ signal: undefined })
      );
    });

    it("adds thumbnail specification filters to the query", async () => {
      mockGet.mockResolvedValueOnce({
        json: () =>
          Promise.resolve({
            thumbnails: [],
          }),
      });
      const signal = new AbortController().signal;

      const service = thumbnailService(mockApi);
      await service.getThumbnails("https://storage.example.com/objects/a.png", {
        signal,
        width: 256,
        height: 256,
        encodingFormat: "JPEG",
        transformType: "SQUARE",
        cropDirection: "CENTER",
        specification: "IMAGE_TO_IMAGE",
      });

      const path = mockGet.mock.calls[0][0] as string;
      const url = new URL(path, "http://thumbnail-service");

      expect(url.pathname).toBe("/thumbnails");
      expect(url.searchParams.get("association_target_resource_address")).toBe(
        "https://storage.example.com/objects/a.png"
      );
      expect(url.searchParams.get("width")).toBe("256");
      expect(url.searchParams.get("height")).toBe("256");
      expect(url.searchParams.get("encoding_format")).toBe("JPEG");
      expect(url.searchParams.get("transform_type")).toBe("SQUARE");
      expect(url.searchParams.get("crop_direction")).toBe("CENTER");
      expect(url.searchParams.get("specification")).toBe("IMAGE_TO_IMAGE");
      expect(mockGet).toHaveBeenCalledWith(path, expect.objectContaining({ signal }));
    });

    it("parses snake_case JSON into camelCase thumbnail entries", async () => {
      mockGet.mockResolvedValueOnce({
        json: () =>
          Promise.resolve({
            thumbnails: [
              {
                thumbnail: {
                  resource_address: "https://storage.example.com/thumbnails/a-256.png",
                  resource_identity: "https://storage.example.com/thumbnails/a-256.png?version-id=thumb",
                },
                specification: {
                  image_to_image: {
                    width: 256,
                    height: 256,
                  },
                },
              },
            ],
          }),
      });

      const service = thumbnailService(mockApi);
      const result = await service.getThumbnails("https://storage.example.com/objects/a.png");

      expect(result.thumbnails).toEqual([
        {
          thumbnail: {
            resourceAddress: "https://storage.example.com/thumbnails/a-256.png",
            resourceIdentity: "https://storage.example.com/thumbnails/a-256.png?version-id=thumb",
          },
          specification: {
            imageToImage: {
              width: 256,
              height: 256,
            },
          },
        },
      ]);
    });

    it("propagates HTTP client errors", async () => {
      mockGet.mockRejectedValueOnce(new Error("network failed"));

      const service = thumbnailService(mockApi);
      await expect(service.getThumbnails("x")).rejects.toThrow("network failed");
    });

    it("rejects responses that fail schema validation", async () => {
      mockGet.mockResolvedValueOnce({
        json: () =>
          Promise.resolve({
            thumbnails: [{ thumbnail: { resource_address: "only-one-field" } }],
          }),
      });

      const service = thumbnailService(mockApi);
      await expect(service.getThumbnails("x")).rejects.toThrow();
    });
  });
});
