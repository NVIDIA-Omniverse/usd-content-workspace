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

import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "fs";
import { tmpdir } from "os";
import { join } from "path";
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
import { fileObjectService } from "./fileObjectService";

const address = "omniverse://nucleus/Library/scene.usd";
const encoded = encodeURIComponent(address);

const server = setupServer(
  http.head(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    () =>
      new HttpResponse(null, {
        status: 204,
        headers: {
          "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
            data_object_size: 1024,
            last_modified_timestamp: "2026-04-10T09:15:00Z",
          }),
          "X-Nvidia-Omniverse-Storage-Resource-Identity": "identity-123",
        },
      }),
  ),
  http.delete(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.post(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}/move`,
    () => HttpResponse.json({ resource_identity: "new-identity" }),
  ),
  http.post(
    `https://storage.example.com/v1alpha/fileobject/by-identity/identity-123/copy`,
    () => HttpResponse.json({ resource_identity: "copy-identity" }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    () => new HttpResponse("scene content", { status: 200 }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("fileObjectService", () => {
  const api = create("https://storage.example.com");
  const service = fileObjectService(api);

  describe("stat", () => {
    it("returns metadata and identity", async () => {
      const result = await service.stat(address);
      expect(result.metadata.dataObjectSize).toBe(1024);
      expect(result.resourceIdentity).toBe("identity-123");
    });

    it("throws when server returns non-204", async () => {
      server.use(
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () => new HttpResponse(null, { status: 404 }),
        ),
      );
      await expect(service.stat(address)).rejects.toThrow(`Failed to stat`);
    });
  });

  describe("delete", () => {
    it("sends DELETE request", async () => {
      await expect(service.delete(address)).resolves.toBeUndefined();
    });

    it("throws when server returns error status", async () => {
      server.use(
        http.delete(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () => new HttpResponse(null, { status: 500 }),
        ),
      );
      await expect(service.delete(address)).rejects.toThrow(`Failed to delete`);
    });
  });

  describe("move", () => {
    it("returns new resource identity", async () => {
      const result = await service.move(
        address,
        "omniverse://nucleus/Library/moved.usd",
      );
      expect(result.resourceIdentity).toBe("new-identity");
    });
  });

  describe("copy", () => {
    it("returns resource identity", async () => {
      const result = await service.copy("identity-123", address);
      expect(result.resourceIdentity).toBe("copy-identity");
    });
  });

  describe("download", () => {
    it("atomically refuses a concurrent download to the same destination", async () => {
      const dir = mkdtempSync(join(tmpdir(), "sn-download-race-"));
      const target = join(dir, "scene.usd");
      const outcomes = await Promise.allSettled([
        service.download(address, target),
        service.download(address, target),
      ]);
      expect(
        outcomes.filter((result) => result.status === "fulfilled"),
      ).toHaveLength(1);
      expect(
        outcomes.filter((result) => result.status === "rejected"),
      ).toHaveLength(1);
      expect(readFileSync(target, "utf8")).toBe("scene content");
      expect(readdirSync(dir)).toEqual(["scene.usd"]);
    });

    it("preserves the destination when an overwrite stream fails", async () => {
      const dir = mkdtempSync(join(tmpdir(), "sn-download-failure-"));
      const target = join(dir, "scene.usd");
      writeFileSync(target, "original");
      const failingApi = create("https://storage.example.com");
      const body = new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode("partial"));
        },
        pull(controller) {
          controller.error(new Error("connection lost"));
        },
      });
      vi.spyOn(failingApi, "get").mockReturnValue(
        Promise.resolve(new Response(body)) as ReturnType<
          typeof failingApi.get
        >,
      );
      await expect(
        fileObjectService(failingApi).download(address, target, undefined, {
          overwrite: true,
        }),
      ).rejects.toThrow("connection lost");
      expect(readFileSync(target, "utf8")).toBe("original");
      expect(readdirSync(dir)).toEqual(["scene.usd"]);
    });
    it("calls onProgress during download", async () => {
      const dir = join(tmpdir(), `sn-download-progress-${Date.now()}`);
      mkdirSync(dir, { recursive: true });
      const localPath = join(dir, "scene.usd");

      const calls: [number, number][] = [];
      await service.download(address, localPath, (transferred, total) => {
        calls.push([transferred, total]);
      });

      expect(calls.length).toBeGreaterThan(0);
      const [lastTransferred, lastTotal] = calls[calls.length - 1];
      expect(lastTransferred).toBe(lastTotal);
      expect(lastTotal).toBeGreaterThan(0);
    });

    it("calls onProgress with correct total when Content-Length is set", async () => {
      const content = "hello world"; // 11 bytes
      const knownAddress = "omniverse://nucleus/Library/known.txt";
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(knownAddress)}`,
          () =>
            new HttpResponse(content, {
              status: 200,
              headers: { "Content-Length": String(content.length) },
            }),
        ),
      );

      const dir = join(tmpdir(), `sn-download-progress-known-${Date.now()}`);
      mkdirSync(dir, { recursive: true });
      const localPath = join(dir, "known.txt");

      const calls: [number, number][] = [];
      await service.download(knownAddress, localPath, (transferred, total) => {
        calls.push([transferred, total]);
      });

      expect(calls.length).toBeGreaterThan(0);
      // every call should report total = content.length
      for (const [, total] of calls) {
        expect(total).toBe(content.length);
      }
      // final call: transferred === total
      const [lastTransferred, lastTotal] = calls[calls.length - 1];
      expect(lastTransferred).toBe(lastTotal);
      expect(lastTotal).toBe(content.length);
    });

    it("throws when server returns error status", async () => {
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () => new HttpResponse(null, { status: 500 }),
        ),
      );
      const dest = join(tmpdir(), "sn-test-download-error.usd");
      await expect(service.download(address, dest)).rejects.toThrow(
        `Failed to download`,
      );
    });

    it("streams file via 300 redirect", async () => {
      const redirectUrl = "https://cdn.example.com/scene.usd";
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () =>
            HttpResponse.json(
              { redirect_target_url: redirectUrl, additional_headers: {} },
              { status: 300 },
            ),
        ),
        http.get(
          redirectUrl,
          () => new HttpResponse("redirected content", { status: 200 }),
        ),
      );
      const dest = join(
        mkdtempSync(join(tmpdir(), "sn-download-redirect-")),
        "scene.usd",
      );
      await service.download(address, dest);
      expect(readFileSync(dest, "utf8")).toBe("redirected content");
    });
  });

  describe("upload", () => {
    it("allows direct uploads to exceed the API client's request timeout", async () => {
      const source = join(tmpdir(), `sn-upload-timeout-${Date.now()}.txt`);
      writeFileSync(source, "content");
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          async ({ request }) => {
            expect(await request.text()).toBe("content");
            await new Promise((resolve) => setTimeout(resolve, 50));
            return new HttpResponse(null, { status: 201 });
          },
        ),
      );
      const shortTimeoutService = fileObjectService(
        create("https://storage.example.com", { timeout: 10 }),
      );
      await shortTimeoutService.upload(source, address);
    });

    it("throws when server returns error status", async () => {
      const src = join(tmpdir(), "sn-test-upload-error.usd");
      writeFileSync(src, "content");
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () => new HttpResponse(null, { status: 500 }),
        ),
      );
      await expect(service.upload(src, address)).rejects.toThrow(
        `Failed to upload`,
      );
    });

    it("uploads via single redirect on 300 response", async () => {
      const src = join(tmpdir(), "sn-test-upload-redirect.usd");
      writeFileSync(src, "upload content");
      const redirectUrl = "https://blob.example.com/upload-target";
      let received = "";
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () =>
            HttpResponse.json(
              {
                redirect: {
                  redirect_target_url: redirectUrl,
                  method: "PUT",
                  additional_headers: [],
                  completion_header_names: [],
                },
              },
              { status: 300 },
            ),
        ),
        http.put(redirectUrl, async ({ request }) => {
          received = await request.text();
          return new HttpResponse(null, { status: 200 });
        }),
      );
      await service.upload(src, address);
      expect(received).toBe("upload content");
    });

    it("throws when single redirect upload fails", async () => {
      const src = join(tmpdir(), "sn-test-upload-redirect-fail.usd");
      writeFileSync(src, "content");
      const redirectUrl = "https://blob.example.com/upload-fail";
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () =>
            HttpResponse.json(
              {
                redirect: {
                  redirect_target_url: redirectUrl,
                  method: "PUT",
                  additional_headers: [],
                  completion_header_names: [],
                },
              },
              { status: 300 },
            ),
        ),
        http.put(redirectUrl, () => new HttpResponse(null, { status: 403 })),
      );
      await expect(service.upload(src, address)).rejects.toThrow(
        `Redirect upload failed`,
      );
    });

    it("throws when 300 response has unknown upload kind", async () => {
      const src = join(tmpdir(), "sn-test-upload-unknown.usd");
      writeFileSync(src, "content");
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () => HttpResponse.json({}, { status: 300 }),
        ),
      );
      await expect(service.upload(src, address)).rejects.toThrow(
        `Unknown upload redirect kind`,
      );
    });

    it("calls onProgress during upload", async () => {
      const dir = join(tmpdir(), `sn-upload-progress-${Date.now()}`);
      mkdirSync(dir, { recursive: true });
      const localPath = join(dir, "data.bin");
      writeFileSync(localPath, "hello world"); // 11 bytes

      const uploadAddress = "omniverse://nucleus/Library/data.bin";
      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(uploadAddress)}`,
          async ({ request }) => {
            await request.arrayBuffer(); // consume body so progressTransform fires
            return new HttpResponse(null, { status: 201 });
          },
        ),
      );

      const calls: [number, number][] = [];
      await service.upload(localPath, uploadAddress, (transferred, total) => {
        calls.push([transferred, total]);
      });

      expect(calls.length).toBeGreaterThan(0);
      const [lastTransferred, lastTotal] = calls[calls.length - 1];
      expect(lastTransferred).toBe(lastTotal);
      expect(lastTotal).toBe(11);
    });

    it("uploads via multipart on 300 response", async () => {
      // 10-byte file split into 2 parts of 5 bytes each
      const content = "0123456789";
      const src = join(tmpdir(), "sn-test-upload-multipart.usd");
      writeFileSync(src, content, "utf8");

      const uploadId = "upload-abc";
      const part1Url = "https://blob.example.com/part1";
      const part2Url = "https://blob.example.com/part2";
      const prepareUrl = `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}/multipart/prepare`;
      const completeUrl = `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}/multipart/complete`;

      const receivedParts: Record<number, string> = {};
      let completeBody: unknown;

      server.use(
        http.put(
          `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
          () =>
            HttpResponse.json(
              {
                multipart: {
                  upload_id: uploadId,
                  first_part_write_redirect: {
                    redirect_target_url: part1Url,
                    method: "PUT",
                    additional_headers: [],
                    completion_header_names: ["ETag"],
                  },
                  minimum_size_per_part: 1,
                  maximum_size_per_part: 5,
                  maximum_parts_number: 10,
                },
              },
              { status: 300 },
            ),
        ),
        http.post(prepareUrl, () =>
          HttpResponse.json({
            part_write_redirects: [
              {
                redirect_target_url: part2Url,
                method: "PUT",
                additional_headers: [],
                completion_header_names: ["ETag"],
              },
            ],
          }),
        ),
        http.put(part1Url, async ({ request }) => {
          receivedParts[1] = await request.text();
          return new HttpResponse(null, {
            status: 200,
            headers: { ETag: "etag-1" },
          });
        }),
        http.put(part2Url, async ({ request }) => {
          receivedParts[2] = await request.text();
          return new HttpResponse(null, {
            status: 200,
            headers: { ETag: "etag-2" },
          });
        }),
        http.post(completeUrl, async ({ request }) => {
          completeBody = await request.json();
          return HttpResponse.json({}, { status: 200 });
        }),
      );

      await service.upload(src, address);

      expect(receivedParts[1]).toBe("01234");
      expect(receivedParts[2]).toBe("56789");
      expect(completeBody).toMatchObject({
        upload_id: uploadId,
        parts: expect.any(Array),
      });
    });
  });
});
