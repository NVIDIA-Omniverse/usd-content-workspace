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

import { HTTPError } from "ky";
import { http, HttpResponse } from "msw";
import { DiscoveryStackInfo } from "@/services/discoveryService";
import { CopyResponse, ListStatResult, TopLevelAddressEntry } from "@/services/storage";
import { VersionPage } from "@/services/storage/versioningService";
import { Connection, rootFolderPath } from "@/state/slices/connectionSlice";
import type { NormalizedOptions } from "ky/distribution/types/options";

export const testConnection: Connection = {
  endpoint: "http://test-endpoint",
  currentFolder: rootFolderPath,
  selectedPaths: new Set(),
  info: {
    auth: null,
    discovery: "http://discovery",
  },
};

export const testEndpoints = [
  http.get("http://*/v1alpha/filefolder/liststat/:resourceAddress", ({ params }) => {
    const resourceAddress = decodeURI(params.resourceAddress as string);
    if (resourceAddress === `http://test-server/bucket1/`) {
      const body = {
        folderAddress: resourceAddress,
        isRoot: true,
        entries: [],
        subfolderAddresses: [`http://test-server/bucket1/folder1/`, `http://test-server/bucket1/folder2/`],
        nextContinuationHandle: null,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    } else if (resourceAddress === `http://test-server/bucket2/`) {
      const body = {
        folderAddress: resourceAddress,
        isRoot: true,
        entries: [],
        subfolderAddresses: [`http://test-server/bucket2/folder3/`],
        nextContinuationHandle: null,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    } else if (resourceAddress === `http://test-server/bucket1/folder1/`) {
      const body = {
        folderAddress: resourceAddress,
        isRoot: false,
        entries: [],
        subfolderAddresses: [],
        nextContinuationHandle: null,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    } else if (resourceAddress === `http://test-server/bucket1/folder2/`) {
      const body = {
        folderAddress: resourceAddress,
        isTopLevelAddress: true,
        entries: [],
        subfolderAddresses: [`http://test-server/bucket1/folder2/subfolder1/`],
        nextContinuationHandle: null,
        isRoot: false,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    } else if (resourceAddress === `http://test-server/bucket2/folder3/`) {
      const body = {
        folderAddress: resourceAddress,
        isTopLevelAddress: true,
        entries: [],
        subfolderAddresses: [`http://test-server/bucket2/folder3/subfolder1/`],
        nextContinuationHandle: null,
        isRoot: false,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    } else if (resourceAddress === `omniverse://test/`) {
      const body = {
        folderAddress: resourceAddress,
        isTopLevelAddress: true,
        entries: [],
        subfolderAddresses: [`omniverse://test/folder/`],
        nextContinuationHandle: null,
        isRoot: false,
      } as ListStatResult;
      return new Response(JSON.stringify(body), { status: 200 });
    }
    return new Response(null, { status: 404 });
  }),

  http.get("http://*/v1alpha/capabilities/top-level-addresses", () => {
    const body = {
      items: [
        { topLevelAddress: "http://test-server/bucket1/" },
        { topLevelAddress: "http://test-server/bucket2/" },
        { topLevelAddress: "omniverse://test/" },
      ] as TopLevelAddressEntry[],
    };
    return new Response(JSON.stringify(body));
  }),

  http.get("http://*/v1alpha/versioning/*/versions", () => {
    const body: VersionPage = {
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
        {
          resourceInfo: {
            resourceIdentity: "3.dat",
            metadata: {
              dataObjectSize: 3,
              lastModifiedTimestamp: "2024-05-15T16:00:00Z",
            },
          },
        },
      ],
      nextContinuationHandle: null,
    };
    return new Response(JSON.stringify(body));
  }),

  http.post<{ resource_identity: string }, { destination_resource_address: string }>(
    "http://*/v1alpha/fileobject/by-identity/:resource_identity/copy",
    async ({ request }) => {
      const { destination_resource_address } = await request.json();
      return HttpResponse.json(
        {
          resourceIdentity: `${destination_resource_address}-identity`,
        } as CopyResponse,
        { status: 200 }
      );
    }
  ),

  // v2 discovery is unsupported by the test deployment, so callers fall back to v1.
  http.get("http://*/api/v2/services", () => {
    return new Response(null, { status: 404 });
  }),

  http.get("http://*/api/v1/services", () => {
    const body: DiscoveryStackInfo = {
      services: [
        {
          id: "storage",
          name: "Storage Service",
          type: "storage",
          rest: "http://test-endpoint",
        },
      ],
    };
    return new Response(JSON.stringify(body), { status: 200 });
  }),

  http.post("http://*/v1alpha/metadata/:resourceAddress", async ({ request }) => {
    const body = await request.text();
    const requestedFields = JSON.parse(body) as string[];

    if (requestedFields.includes("custom")) {
      return HttpResponse.json({
        custom: {
          "test-key": "test-value",
          "another-key": "another-value",
        },
      });
    }

    return HttpResponse.json({
      acl: {
        etag: "",
        value: ["read", "write"],
      },
    });
  }),

  http.put("http://*/v1alpha/metadata/:resourceAddress", () => {
    return HttpResponse.json({});
  }),

  // The v1beta metadata API is batch only: reading every key of a resource goes through
  // QueryMetadata and updating a key goes through the UpdateMetadataBatch route.
  http.post("http://*/v1beta/metadata/query", () => {
    return HttpResponse.json({
      results: [{ user_metadata_key: "acl", user_metadata: { etag: "", value: ["read", "write"] } }],
      next_continuation_handle: null,
    });
  }),

  http.put("http://*/v1beta/metadata", () => {
    return HttpResponse.json({ results: [{ request_index: 0, outcome: { Success: { etag: "updated-etag" } } }] });
  }),
];

export function createTestHttpError(init: ResponseInit, body?: BodyInit): HTTPError {
  const response = new Response(body, init);
  const request = new Request("http://localhost:3000/test");
  const options: NormalizedOptions = {
    body,
    method: "",
    prefixUrl: "",
    retry: {},
    onDownloadProgress: vi.fn(),
    onUploadProgress: vi.fn(),
  };
  return new HTTPError(response, request, options);
}
