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
import { AppConfigError, fetchAppConfig } from "./appConfigService";

describe("appConfigService", () => {
  const server = setupServer();

  beforeAll(() => {
    server.listen();
  });

  afterEach(() => {
    server.resetHandlers();
  });

  afterAll(() => {
    server.close();
  });

  it("parses a valid config.json", async () => {
    server.use(http.get("*/config.json", () => HttpResponse.json({ defaultDiscovery: "https://stack.example/" })));

    const config = await fetchAppConfig();
    expect(config.defaultDiscovery).toBe("https://stack.example/");
  });

  it("falls back to empty config when config.json is missing", async () => {
    server.use(http.get("*/config.json", () => new HttpResponse(null, { status: 404 })));

    const config = await fetchAppConfig();
    expect(config).toEqual({ defaultDiscovery: "", references: [], requestTimeout: 30000, batchSize: 25 });
  });

  it("falls back to empty config when fetch throws", async () => {
    server.use(http.get("*/config.json", () => HttpResponse.error()));

    const config = await fetchAppConfig();
    expect(config).toEqual({ defaultDiscovery: "", references: [], requestTimeout: 30000, batchSize: 25 });
  });

  it("throws with the status when config.json fails to load for a non-404 reason", async () => {
    server.use(http.get("*/config.json", () => new HttpResponse(null, { status: 500 })));

    await expect(fetchAppConfig()).rejects.toBeInstanceOf(AppConfigError);
    await expect(fetchAppConfig()).rejects.toMatchObject({ message: expect.stringContaining("500") });
  });

  it("applies the default for a missing field", async () => {
    server.use(http.get("*/config.json", () => HttpResponse.json({})));

    const config = await fetchAppConfig();
    expect(config.defaultDiscovery).toBe("");
  });

  it("parses configured reference links", async () => {
    server.use(
      http.get("*/config.json", () =>
        HttpResponse.json({
          references: [
            { label: "Docs", url: "https://docs.example/", icon: "book" },
            { label: "Support", url: "https://support.example/" },
          ],
        })
      )
    );

    const config = await fetchAppConfig();
    expect(config.references).toEqual([
      { label: "Docs", url: "https://docs.example/", icon: "book" },
      { label: "Support", url: "https://support.example/" },
    ]);
  });

  it("accepts http and root-relative reference urls", async () => {
    server.use(
      http.get("*/config.json", () =>
        HttpResponse.json({
          references: [
            { label: "Insecure", url: "http://docs.example/" },
            { label: "Local", url: "/help" },
          ],
        })
      )
    );

    const config = await fetchAppConfig();
    expect(config.references.map((reference) => reference.url)).toEqual(["http://docs.example/", "/help"]);
  });

  it.each([
    ["script scheme", "javascript" + ":alert(1)"],
    ["data scheme", "data:text/html,<script>alert(1)</script>"],
    ["protocol-relative path", "//evil.example/"],
    ["malformed url", "not a url"],
  ])("rejects an unsafe or malformed reference url (%s)", async (_label, url) => {
    server.use(http.get("*/config.json", () => HttpResponse.json({ references: [{ label: "Bad", url }] })));

    await expect(fetchAppConfig()).rejects.toBeInstanceOf(AppConfigError);
    await expect(fetchAppConfig()).rejects.toMatchObject({
      issues: expect.arrayContaining([expect.stringContaining("references.0.url")]),
    });
  });

  it("defaults references to an empty array when the field is missing", async () => {
    server.use(http.get("*/config.json", () => HttpResponse.json({ defaultDiscovery: "https://stack.example/" })));

    const config = await fetchAppConfig();
    expect(config.references).toEqual([]);
  });

  it("throws a validation error listing offending fields when a reference is malformed", async () => {
    server.use(
      http.get("*/config.json", () =>
        HttpResponse.json({
          defaultDiscovery: "https://stack.example/",
          references: [{ label: "Docs", link: "https://docs.example/" }],
        })
      )
    );

    await expect(fetchAppConfig()).rejects.toBeInstanceOf(AppConfigError);
    await expect(fetchAppConfig()).rejects.toMatchObject({
      issues: expect.arrayContaining([expect.stringContaining("references.0.url")]),
    });
  });

  it("throws a validation error when config.json is not valid JSON", async () => {
    server.use(http.get("*/config.json", () => new HttpResponse("not json", { status: 200 })));

    await expect(fetchAppConfig()).rejects.toBeInstanceOf(AppConfigError);
  });
});
