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
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { create as createApi } from "@/services/api";
import { createMockFile, createMockFolder } from "@/test-utils/mockPath";
import { archiveService, isArchiveServiceUsable, toArchiveItems, triggerFormDownload } from "./archiveService";

const server = setupServer();

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
});
afterAll(() => server.close());

describe("archiveService.prepare", () => {
  it("sends the selection and returns the parsed prepare response", async () => {
    const requestSpy = vi.fn();
    server.use(
      http.post("http://archive-test/v1alpha/archive/prepare", async ({ request }) => {
        requestSpy({ body: await request.json(), credentials: request.credentials });
        return HttpResponse.json({
          download_action: "/v1alpha/archive/download",
          expires_at: "2026-01-01T00:00:00Z",
        });
      })
    );

    const service = archiveService(createApi("http://archive-test/v1alpha"));
    const items = toArchiveItems([createMockFile("a/file.txt"), createMockFolder("a/folder")]);

    const result = await service.prepare(items, "my-archive");

    expect(result).toEqual({ downloadAction: "/v1alpha/archive/download", expiresAt: "2026-01-01T00:00:00Z" });
    expect(requestSpy).toHaveBeenCalledWith({
      body: { items, archive_name: "my-archive" },
      credentials: "include",
    });
  });

  it("surfaces the problem+json detail on error", async () => {
    server.use(
      http.post("http://archive-test/v1alpha/archive/prepare", () =>
        HttpResponse.json(
          { title: "Bad Request", status: 400, detail: "too many items (max 10000)" },
          { status: 400, headers: { "Content-Type": "application/problem+json" } }
        )
      )
    );

    const service = archiveService(createApi("http://archive-test/v1alpha"));

    await expect(service.prepare([{ type: "file", address: "a" }], "x")).rejects.toThrow("too many items (max 10000)");
  });
});

describe("toArchiveItems", () => {
  it("maps files and folders to the archive item shape", () => {
    const items = toArchiveItems([createMockFile("a/file.txt"), createMockFolder("a/folder")]);
    expect(items).toEqual([
      { type: "file", address: "a/file.txt" },
      { type: "folder", address: "a/folder" },
    ]);
  });
});

describe("isArchiveServiceUsable", () => {
  const app = { protocol: "https:", host: "app.example.com", hostname: "app.example.com" } as Location;

  it("returns true for the same host", () => {
    expect(isArchiveServiceUsable("https://app.example.com", app)).toBe(true);
  });

  it("returns true for a subdomain of the same registrable domain", () => {
    expect(isArchiveServiceUsable("https://archive.example.com", app)).toBe(true);
  });

  it("returns true when both the app and service run on localhost", () => {
    const localApp = { protocol: "http:", host: "localhost:8080", hostname: "localhost" } as Location;
    expect(isArchiveServiceUsable("http://localhost:9090", localApp)).toBe(true);
  });

  it("returns false and warns when only the app runs on localhost", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const localApp = { protocol: "http:", host: "localhost:8080", hostname: "localhost" } as Location;
    expect(isArchiveServiceUsable("http://archive-service:8080", localApp)).toBe(false);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("returns false and warns for a different registrable domain", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    expect(isArchiveServiceUsable("https://archive.other.com", app)).toBe(false);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("returns false and warns for a different scheme", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    expect(isArchiveServiceUsable("http://app.example.com", app)).toBe(false);
    expect(warn).toHaveBeenCalledTimes(1);
  });
});

describe("triggerFormDownload", () => {
  it("submits a hidden form with bracket-notation fields into a hidden iframe", () => {
    const submitSpy = vi.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => undefined);

    triggerFormDownload({
      action: "https://archive.example.com/v1alpha/archive/download",
      items: [
        { type: "folder", address: "a/folder" },
        { type: "file", address: "a/file.txt" },
      ],
      archiveName: "bundle",
    });

    const form = document.querySelector("form");
    const iframe = document.querySelector("iframe");
    expect(form).not.toBeNull();
    expect(iframe).not.toBeNull();
    expect(form!.method).toBe("post");
    expect(form!.action).toBe("https://archive.example.com/v1alpha/archive/download");
    expect(form!.target).toBe(iframe!.name);

    const fields = Object.fromEntries(new FormData(form!).entries());
    expect(fields).toEqual({
      "items[0][type]": "folder",
      "items[0][address]": "a/folder",
      "items[1][type]": "file",
      "items[1][address]": "a/file.txt",
      archive_name: "bundle",
    });
    expect(submitSpy).toHaveBeenCalledTimes(1);
  });
});
