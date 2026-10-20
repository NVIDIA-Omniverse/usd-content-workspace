// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

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
import { create } from "@/services/api";
import { runTags } from "./tags";

const address = "https://bucket.example.com/bucket/folder/";
const encoded = encodeURIComponent(address);
const api = create("https://storage.example.com");
let tags: Record<string, string> = { owner: "team" };
let currentEtag = "etag-1";
let receivedEtag: string | null = null;

const server = setupServer(
  http.post(`https://storage.example.com/v1alpha/metadata/${encoded}`, () =>
    HttpResponse.json({ tags: { etag: currentEtag, value: tags } }),
  ),
  http.put(
    `https://storage.example.com/v1alpha/metadata/${encoded}/tags`,
    async ({ request }) => {
      receivedEtag = request.headers.get("If-Match");
      if (receivedEtag !== currentEtag)
        return new HttpResponse(null, { status: 412 });
      tags = (await request.json()) as Record<string, string>;
      currentEtag = "etag-2";
      return new HttpResponse(null, { status: 204 });
    },
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
  tags = { owner: "team" };
  currentEtag = "etag-1";
  receivedEtag = null;
});

describe("runTags", () => {
  it("lists tags and their ETag as JSON", async () => {
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((value) => {
      output.push(String(value));
      return true;
    });
    await runTags(api, "list", address, { json: true });
    expect(JSON.parse(output.join(""))).toEqual({
      path: address,
      etag: "etag-1",
      tags: { owner: "team" },
    });
  });

  it("sets and removes tags using the current ETag", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await runTags(api, "set", address, {
      key: "project",
      value: "demo",
      json: false,
    });
    expect(receivedEtag).toBe("etag-1");
    expect(tags).toEqual({ owner: "team", project: "demo" });
    await runTags(api, "remove", address, { key: "owner", json: false });
    expect(receivedEtag).toBe("etag-2");
    expect(tags).toEqual({ project: "demo" });
  });

  it("surfaces a stale ETag conflict without reporting success", async () => {
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/metadata/${encoded}/tags`,
        () => new HttpResponse(null, { status: 412 }),
      ),
    );
    const output = vi
      .spyOn(process.stdout, "write")
      .mockImplementation(() => true);
    await expect(
      runTags(api, "set", address, {
        key: "owner",
        value: "other",
        json: false,
      }),
    ).rejects.toMatchObject({
      response: { status: 412 },
    });
    expect(output).not.toHaveBeenCalled();
  });
});
