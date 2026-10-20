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
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { capabilitiesService } from "@/services/storage/capabilitiesService";
import { metadataService } from "@/services/storage/metadataService";
import { buildContext, getDiscoveredApi } from "./context";

vi.mock("@/config", () => ({
  readConfig: () => ({
    activeHost: "test",
    hosts: {
      test: {
        discoveryUrl: "https://discovery.example.com/api/v1/",
        storageUrl: "https://legacy-storage.example.com",
        authUrl: "",
      },
      other: {
        discoveryUrl: "https://other-discovery.example.com/api/v1/",
        storageUrl: "https://other-storage.example.com",
        authUrl: "",
        cwd: "omniverse://other/Library/",
      },
    },
  }),
  getActiveHost: (config: { hosts: { test: unknown } }) => config.hosts.test,
}));

let queryCalls = 0;
let updatedTags: unknown;
const server = setupServer(
  http.get("https://discovery.example.com/api/v2/services", () =>
    HttpResponse.json({
      "schema-version": 2,
      services: [
        {
          name: "storage-service",
          rest: {
            url: "https://new-storage.example.com",
            endpoints: [
              {
                name: "capabilities",
                versions: { v1alpha: { path: "/api/caps" } },
              },
              { name: "metadata", versions: { v1beta: { path: "/api/meta" } } },
            ],
          },
        },
      ],
    }),
  ),
  http.get("https://other-discovery.example.com/api/v2/services", () =>
    HttpResponse.json({
      "schema-version": 2,
      services: [
        {
          name: "other-storage",
          rest: {
            url: "https://other-new-storage.example.com",
            endpoints: [
              {
                name: "capabilities",
                versions: { v1alpha: { path: "/api/caps" } },
              },
            ],
          },
        },
      ],
    }),
  ),
  http.get(
    "https://other-new-storage.example.com/api/caps/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [{ top_level_address: "omniverse://other/" }],
      }),
  ),
  http.get(
    "https://new-storage.example.com/api/caps/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [{ top_level_address: "omniverse://nucleus/" }],
      }),
  ),
  http.post(
    "https://new-storage.example.com/api/meta/metadata/query",
    async ({ request }) => {
      const body = (await request.json()) as { continuation_handle?: string };
      queryCalls++;
      return HttpResponse.json(
        body.continuation_handle
          ? {
              results: [
                {
                  user_metadata_key: "tags",
                  user_metadata: { etag: "tag-1", value: { owner: "agent" } },
                },
              ],
              next_continuation_handle: null,
            }
          : {
              results: [
                {
                  user_metadata_key: "acl",
                  user_metadata: { etag: "acl-1", value: ["read"] },
                },
              ],
              next_continuation_handle: "next",
            },
      );
    },
  ),
  http.put(
    "https://new-storage.example.com/api/meta/metadata",
    async ({ request }) => {
      updatedTags = await request.json();
      return HttpResponse.json({ results: [{ outcome: { Success: {} } }] });
    },
  ),
);

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  queryCalls = 0;
  updatedTags = undefined;
});

describe("discovered CLI routes", () => {
  it("selects a host for one invocation without changing the active host", () => {
    const selected = buildContext({ host: "other" });
    expect(selected.hostName).toBe("other");
    expect(selected.cwd).toBe("omniverse://other/Library/");
    expect(buildContext({}).hostName).toBe("test");
    expect(() => buildContext({ host: "missing" })).toThrow(
      "host 'missing' not found",
    );
  });

  it("keeps concurrent host discovery and routes isolated", async () => {
    const first = buildContext({ host: "test" });
    const second = buildContext({ host: "other" });
    const [testBuckets, otherBuckets] = await Promise.all([
      capabilitiesService(first.api).listTopLevelAddresses(),
      capabilitiesService(second.api).listTopLevelAddresses(),
    ]);
    expect(testBuckets.subfolderAddresses).toEqual(["omniverse://nucleus/"]);
    expect(otherBuckets.subfolderAddresses).toEqual(["omniverse://other/"]);
    expect(buildContext({}).hostName).toBe("test");
  });

  it("routes legacy operations to advertised API URLs and uses metadata v1beta", async () => {
    const { api } = buildContext({});
    const buckets = await capabilitiesService(api).listTopLevelAddresses();
    expect(buckets.subfolderAddresses).toEqual(["omniverse://nucleus/"]);
    expect((await getDiscoveredApi(api, "metadata"))?.version).toBe("v1beta");

    const metadata = await metadataService(api).get(
      "omniverse://nucleus/file.usd",
    );
    expect(queryCalls).toBe(2);
    expect(metadata.acl.value).toEqual(["read"]);
    expect(metadata.tags.value).toEqual({ owner: "agent" });

    await metadataService(api).setTags(
      "omniverse://nucleus/file.usd",
      { owner: "other" },
      "tag-1",
    );
    expect(updatedTags).toEqual({
      requests: [
        {
          resource_location: {
            resource_address: "omniverse://nucleus/file.usd",
          },
          user_metadata_key: "tags",
          user_metadata: { owner: "other" },
          expected_etag: "tag-1",
        },
      ],
    });
  });

  it("fails clearly when a required API is not advertised", async () => {
    const { api } = buildContext({});
    await expect(
      api.head("v1alpha/fileobject/by-address/test"),
    ).rejects.toThrow("fileobject API is not available");
  });

  it("treats metadata batch errors as failures", async () => {
    server.use(
      http.put("https://new-storage.example.com/api/meta/metadata", () =>
        HttpResponse.json({
          results: [
            { outcome: { Success: {} } },
            {
              outcome: {
                Error: { code: "failed_precondition", message: "stale etag" },
              },
            },
          ],
        }),
      ),
    );
    const { api } = buildContext({});
    await expect(
      metadataService(api).setTags("omniverse://nucleus/file.usd", {}, "old"),
    ).rejects.toThrow("failed_precondition");
  });
});
