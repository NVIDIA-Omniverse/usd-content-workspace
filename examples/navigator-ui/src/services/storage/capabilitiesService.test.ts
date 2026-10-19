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
import { rootFolderPath } from "@/state/appStore";
import type { ApiInstance } from "../api";
import { capabilitiesService } from "./capabilitiesService";

describe("capabilitiesService", () => {
  const mockGet = vi.fn();
  const mockApi: ApiInstance = {
    get: mockGet,
    serviceUrl: "http://test-service",
  } as unknown as ApiInstance;

  beforeEach(() => {
    mockGet.mockReset();
  });

  describe("listTopLevelAddresses", () => {
    it("should fetch and transform top level addresses", async () => {
      const mockResponse = {
        items: [
          { top_level_address: "address1" },
          { top_level_address: "address2" },
          { top_level_address: "address3" },
        ],
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = capabilitiesService(mockApi);
      const result = await service.listTopLevelAddresses();

      expect(mockGet).toHaveBeenCalledWith("capabilities/top-level-addresses");
      expect(result).toEqual({
        folderAddress: rootFolderPath,
        isRoot: true,
        subfolderAddresses: ["address1/", "address2/", "address3/"],
        entries: [],
        nextContinuationHandle: null,
      });
    });

    it("should handle empty items array", async () => {
      const mockResponse = {
        items: [],
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = capabilitiesService(mockApi);
      const result = await service.listTopLevelAddresses();

      expect(mockGet).toHaveBeenCalledWith("capabilities/top-level-addresses");
      expect(result).toEqual({
        folderAddress: rootFolderPath,
        isRoot: true,
        subfolderAddresses: [],
        entries: [],
        nextContinuationHandle: null,
      });
    });

    it("should handle API errors", async () => {
      mockGet.mockRejectedValueOnce(new Error("API Error"));

      const service = capabilitiesService(mockApi);
      await expect(service.listTopLevelAddresses()).rejects.toThrow("API Error");
    });

    it("should handle invalid response data", async () => {
      const mockResponse = {
        // Missing 'items' property
        somethingElse: [],
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = capabilitiesService(mockApi);
      await expect(service.listTopLevelAddresses()).rejects.toThrow();
    });

    it("should handle invalid item format", async () => {
      const mockResponse = {
        items: [
          { wrongProperty: "address1" }, // Missing topLevelAddress
          { topLevelAddress: "address2" },
        ],
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = capabilitiesService(mockApi);
      await expect(service.listTopLevelAddresses()).rejects.toThrow();
    });

    it("should handle non-string top_level_address", async () => {
      const mockResponse = {
        items: [
          { topLevelAddress: 123 }, // Not a string
          { topLevelAddress: "address2" },
        ],
      };

      mockGet.mockResolvedValueOnce({
        json: () => Promise.resolve(mockResponse),
      });

      const service = capabilitiesService(mockApi);
      await expect(service.listTopLevelAddresses()).rejects.toThrow();
    });
  });
});
