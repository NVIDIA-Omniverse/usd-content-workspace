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
import type { ApiInstance } from "../api";
import { versioningService } from "./versioningService";

describe("versioningService", () => {
  const mockGet = vi.fn();
  const mockApi: ApiInstance = {
    get: mockGet,
  } as unknown as ApiInstance;

  beforeEach(() => {
    mockGet.mockReset();
  });

  describe("listAll", () => {
    it("fetches every version page", async () => {
      mockGet
        .mockResolvedValueOnce({
          json: () =>
            Promise.resolve({
              items: [
                {
                  resource_info: {
                    resource_identity: "1.dat",
                    metadata: {
                      data_object_size: 1,
                      last_modified_timestamp: "2024-05-15T14:00:00Z",
                    },
                  },
                },
              ],
              next_continuation_handle: "page2",
              versions_order: "oldest_first",
            }),
        })
        .mockResolvedValueOnce({
          json: () =>
            Promise.resolve({
              items: [
                {
                  resource_info: {
                    resource_identity: "2.dat",
                    metadata: {
                      data_object_size: 2,
                      last_modified_timestamp: "2024-05-15T15:00:00Z",
                    },
                  },
                },
              ],
              next_continuation_handle: null,
              versions_order: "oldest_first",
            }),
        });

      const service = versioningService(mockApi);
      const result = await service.listAll("test folder");

      expect(mockGet).toHaveBeenCalledTimes(2);
      expect(mockGet).toHaveBeenNthCalledWith(1, "versioning/test%20folder/versions?max_page_size=1000");
      expect(mockGet).toHaveBeenNthCalledWith(
        2,
        "versioning/test%20folder/versions?max_page_size=1000&continuation_handle=page2"
      );
      expect(result).toEqual({
        items: [
          {
            resourceInfo: {
              resourceIdentity: "1.dat",
              metadata: {
                dataObjectSize: 1,
                lastModifiedTimestamp: "2024-05-15T14:00:00Z",
              },
            },
          },
          {
            resourceInfo: {
              resourceIdentity: "2.dat",
              metadata: {
                dataObjectSize: 2,
                lastModifiedTimestamp: "2024-05-15T15:00:00Z",
              },
            },
          },
        ],
        nextContinuationHandle: null,
        versionsOrder: "oldest_first",
      });
    });
  });
});
