// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest";
import { getDiscoveredApi, type CommandContext } from "@/context";
import { create } from "@/services/api";
import { runArchive } from "./archive";

vi.mock("@/context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/context")>()),
  getDiscoveredApi: vi.fn(),
}));

const file = "https://bucket.example.com/bucket/scene.usd";
const folder = "https://bucket.example.com/bucket/Textures/";
const context: CommandContext = {
  api: create("https://storage.example.com"),
  storageUrl: "https://storage.example.com",
  hostName: "test",
  discoveryUrl: "https://discovery.example.com",
  cwd: undefined,
  verbose: false,
  json: false,
};
let submitted: URLSearchParams | undefined;

const server = setupServer(
  http.post(
    "https://archive.example.com/v1alpha/archive/prepare",
    async ({ request }) => {
      expect(await request.json()).toEqual({
        items: [
          { type: "file", address: file },
          { type: "folder", address: folder },
        ],
        archive_name: "bundle",
      });
      return HttpResponse.json(
        {
          download_action: "/v1alpha/archive/download",
          expires_at: "2026-10-01T00:00:00Z",
        },
        {
          headers: {
            "Set-Cookie": "archive_token=abc; Path=/; SameSite=Strict",
          },
        },
      );
    },
  ),
  http.post(
    "https://archive.example.com/v1alpha/archive/download",
    async ({ request }) => {
      expect(request.headers.get("cookie")).toContain("archive_token=abc");
      submitted = new URLSearchParams(await request.text());
      return new HttpResponse("zip bytes", {
        headers: { "Content-Type": "application/zip" },
      });
    },
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
  submitted = undefined;
});

it("downloads an advertised archive using the prepare cookie and form fields", async () => {
  vi.mocked(getDiscoveredApi).mockResolvedValue({
    baseUrl: "https://archive.example.com",
    versionPath: "/v1alpha",
    version: "v1alpha",
    url: "https://archive.example.com/v1alpha",
  });
  const directory = await mkdtemp(join(tmpdir(), "sn-archive-"));
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  try {
    const output = join(directory, "bundle.zip");
    await runArchive(context, [file, folder], output, false);
    expect(await readFile(output, "utf8")).toBe("zip bytes");
    expect(submitted?.get("items[0][type]")).toBe("file");
    expect(submitted?.get("items[1][address]")).toBe(folder);
    expect(submitted?.get("archive_name")).toBe("bundle");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

it("reports an unavailable optional archive service", async () => {
  vi.mocked(getDiscoveredApi).mockResolvedValue(null);
  await expect(
    runArchive(context, [file], "bundle.zip", false),
  ).rejects.toMatchObject({
    code: "SERVICE_UNAVAILABLE",
  });
});

it("preserves an existing destination before contacting the archive service", async () => {
  vi.mocked(getDiscoveredApi).mockResolvedValue({
    baseUrl: "https://archive.example.com",
    versionPath: "/v1alpha",
    version: "v1alpha",
    url: "https://archive.example.com/v1alpha",
  });
  const directory = await mkdtemp(join(tmpdir(), "sn-archive-"));
  try {
    const output = join(directory, "bundle.zip");
    await writeFile(output, "original");
    await expect(
      runArchive(context, [file], output, false),
    ).rejects.toMatchObject({ code: "CONFLICT" });
    expect(await readFile(output, "utf8")).toBe("original");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

it("rejects a cross-origin download action before sending the cookie", async () => {
  vi.mocked(getDiscoveredApi).mockResolvedValue({
    baseUrl: "https://archive.example.com",
    versionPath: "/v1alpha",
    version: "v1alpha",
    url: "https://archive.example.com/v1alpha",
  });
  server.use(
    http.post("https://archive.example.com/v1alpha/archive/prepare", () =>
      HttpResponse.json(
        {
          download_action: "https://other.example.com/download",
          expires_at: "2026-10-01T00:00:00Z",
        },
        {
          headers: { "Set-Cookie": "archive_token=abc; Path=/" },
        },
      ),
    ),
  );
  await expect(
    runArchive(context, [file], join(tmpdir(), "sn-cross-origin.zip"), false),
  ).rejects.toThrow("different origin");
});
