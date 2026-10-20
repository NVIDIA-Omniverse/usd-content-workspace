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
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, MockInstance, vi } from "vitest";
import { ApiInstance, create } from "./api";
import { DiscoveryStackInfo, fetchDiscoveryDocument, resolveDiscoveryUrls } from "./discoveryService";

describe("discoveryService", () => {
  const hostname = "discovery";

  describe("resolveDiscoveryUrls", () => {
    it("should create discovery URLs from the hostname", () => {
      expect(resolveDiscoveryUrls(hostname).v1Url).toBe(`https://${hostname}/api/v1/`);
      expect(resolveDiscoveryUrls(hostname).v2Url).toBe(`https://${hostname}/api/v2/`);
    });

    it("should create discovery URLs from the URL prefix", () => {
      expect(resolveDiscoveryUrls(`https://${hostname}/basePath/`).v1Url).toBe(`https://${hostname}/basePath/api/v1/`);
    });
  });

  describe("fetchDiscoveryDocument", () => {
    const { v1Url, v2Url } = resolveDiscoveryUrls(hostname);

    const stackInfo: DiscoveryStackInfo = {
      services: [{ id: "service1", name: "Storage Service", type: "storage", rest: "http://rest-endpoint" }],
    };

    const v2Info = {
      "schema-version": 2,
      services: [
        {
          name: "storage-service",
          rest: {
            url: "https://storage-v2",
            endpoints: [{ name: "filefolder", versions: { v1alpha: { path: "/v1alpha" } } }],
          },
        },
      ],
    };

    const server = setupServer(
      http.get(`${v1Url}services`, () => {
        return HttpResponse.json(stackInfo);
      })
    );

    const v2ProbeFailures: { reason: string; respond: () => Response }[] = [
      { reason: "the route is not found", respond: () => new HttpResponse(null, { status: 404 }) },
      { reason: "the request fails with a server error", respond: () => new HttpResponse(null, { status: 500 }) },
      { reason: "the request is rejected as unauthorized", respond: () => new HttpResponse(null, { status: 401 }) },
      { reason: "the body is not JSON", respond: () => new HttpResponse("<html lang='en'></html>") },
      { reason: "the document does not match the schema", respond: () => HttpResponse.json({ "schema-version": 3 }) },
      { reason: "the connection fails", respond: () => HttpResponse.error() },
    ];

    const respondWithV2 = (respond: () => Response) => {
      server.use(http.get(`${v2Url}services`, respond));
    };

    const fetchDocument = () => fetchDiscoveryDocument(create(v1Url), create(v2Url));

    let warn: MockInstance<typeof console.warn>;

    beforeAll(() => {
      server.listen();
    });

    afterAll(() => {
      server.close();
    });

    beforeEach(() => {
      warn = vi.spyOn(console, "warn").mockReturnValue(undefined);
    });

    afterEach(() => {
      server.resetHandlers();
      warn.mockRestore();
    });

    it("should return the v2 document when the deployment advertises it", async () => {
      respondWithV2(() => HttpResponse.json(v2Info));

      await expect(fetchDocument()).resolves.toEqual({ schemaVersion: 2, services: v2Info });
      expect(warn).not.toHaveBeenCalled();
    });

    it.each(v2ProbeFailures)("should fall back to v1 and log the failure when $reason", async ({ respond }) => {
      respondWithV2(respond);

      await expect(fetchDocument()).resolves.toEqual({ schemaVersion: 1, services: stackInfo });
      expect(warn).toHaveBeenCalledWith(expect.stringContaining(`${v2Url}services`));
    });

    it("should fall back to v1 when the v2 probe rejects without an error value", async () => {
      const missingError = undefined as unknown as Error;
      const v2Api = { serviceUrl: v2Url, get: () => Promise.reject(missingError) } as unknown as ApiInstance;

      await expect(fetchDiscoveryDocument(create(v1Url), v2Api)).resolves.toEqual({
        schemaVersion: 1,
        services: stackInfo,
      });
      expect(warn).toHaveBeenCalledWith(expect.stringContaining(`${v2Url}services`));
    });

    it("should log the response status of a failed v2 probe", async () => {
      respondWithV2(() => new HttpResponse(null, { status: 503, statusText: "Service Unavailable" }));

      await fetchDocument();
      expect(warn).toHaveBeenCalledWith(expect.stringContaining("HTTP 503 Service Unavailable"));
    });

    it("should log the schema violations of an invalid v2 document", async () => {
      respondWithV2(() => HttpResponse.json({ ...v2Info, "schema-version": 3 }));

      await fetchDocument();
      expect(warn).toHaveBeenCalledWith(expect.stringContaining("schema-version: Invalid literal value, expected 2"));
    });
  });
});
