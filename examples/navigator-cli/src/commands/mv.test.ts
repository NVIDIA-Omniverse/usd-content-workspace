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
import { noopLogger } from "@/output";
import { create } from "@/services/api";
import { runMv } from "./mv";

const src = "omniverse://nucleus/Library/scene.usd";
const dst = "omniverse://nucleus/Library/moved.usd";
const srcEncoded = encodeURIComponent(src);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortSrc = `${topLevelAddress}Library/scene.usd`;
const shortSrcEncoded = encodeURIComponent(shortSrc);

const folderSrc = "omniverse://nucleus/Library/old-folder/";
const folderDst = "omniverse://nucleus/Library/new-folder/";
const file1 = `${folderSrc}file1.usd`;
const file2 = `${folderSrc}file2.usd`;
const subFolder = `${folderSrc}sub/`;
const subFile = `${subFolder}sub-file.usd`;
const file1Encoded = encodeURIComponent(file1);
const file2Encoded = encodeURIComponent(file2);
const subFileEncoded = encodeURIComponent(subFile);
const folderSrcEncoded = encodeURIComponent(folderSrc);
const subFolderEncoded = encodeURIComponent(subFolder);

const server = setupServer(
  http.put(
    "https://storage.example.com/v1alpha/filefolder/:address",
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.get(
    "https://storage.example.com/v1alpha/capabilities/top-level-addresses",
    () =>
      HttpResponse.json({
        items: [
          {
            top_level_address:
              "https://mybucket.blob.core.windows.net/mybucket",
          },
        ],
      }),
  ),
  http.post(
    `https://storage.example.com/v1alpha/fileobject/by-address/${srcEncoded}/move`,
    () => HttpResponse.json({ resource_identity: "new-id" }),
  ),
  http.post(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortSrcEncoded}/move`,
    () => HttpResponse.json({ resource_identity: "new-id" }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
});

describe("runMv", () => {
  const api = create("https://storage.example.com");

  it("moves file from src to dst", async () => {
    await expect(runMv(api, src, dst)).resolves.toBeUndefined();
  });

  it("previews without moving and requests atomic conflict protection", async () => {
    let moves = 0;
    let body: unknown;
    server.use(
      http.post(
        `https://storage.example.com/v1alpha/fileobject/by-address/${srcEncoded}/move`,
        async ({ request }) => {
          moves++;
          body = await request.json();
          return HttpResponse.json({ resource_identity: "new-id" });
        },
      ),
    );
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((text) => {
      output.push(String(text));
      return true;
    });
    await runMv(api, src, dst, noopLogger(), undefined, { dryRun: true });
    expect(moves).toBe(0);
    expect(output.join("")).toContain(dst);
    await runMv(api, src, dst);
    expect(body).toEqual({
      destination_resource_address: dst,
      fail_if_exists: true,
    });
  });

  it("resolves short names to full addresses", async () => {
    await expect(
      runMv(api, "mybucket/Library/scene.usd", "mybucket/Library/moved.usd"),
    ).resolves.toBeUndefined();
  });

  it("resolves bare src and dst relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    await expect(
      runMv(api, "scene.usd", "moved.usd", noopLogger(), cwd),
    ).resolves.toBeUndefined();
  });

  describe("folder move", () => {
    afterEach(() => {
      delete process.env.SN_CONCURRENCY;
    });

    it("creates nested empty destinations before deleting source folders", async () => {
      const existing = new Set([folderSrc, subFolder]);
      server.use(
        http.get(
          "https://storage.example.com/v1alpha/filefolder/list/:address",
          ({ params }) =>
            HttpResponse.json({
              subfolder_addresses:
                params.address === folderSrc ? [subFolder] : [],
              sub_resource_addresses: [],
              next_continuation_handle: null,
            }),
        ),
        http.put(
          "https://storage.example.com/v1alpha/filefolder/:address",
          ({ params }) => {
            existing.add(String(params.address));
            return new HttpResponse(null, { status: 204 });
          },
        ),
        http.delete(
          "https://storage.example.com/v1alpha/filefolder/list/:address",
          ({ params }) => {
            const source = String(params.address);
            expect(existing.has(source.replace(folderSrc, folderDst))).toBe(
              true,
            );
            existing.delete(source);
            return new HttpResponse(null, { status: 204 });
          },
        ),
      );
      await runMv(api, folderSrc, folderDst);
      expect([...existing].sort()).toEqual(
        [folderDst, `${folderDst}sub/`].sort(),
      );
    });

    it("preserves the source when destination folder creation fails", async () => {
      let deleted = false;
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [],
              next_continuation_handle: null,
            }),
        ),
        http.put(
          "https://storage.example.com/v1alpha/filefolder/:address",
          () => new HttpResponse(null, { status: 500 }),
        ),
        http.delete(
          "https://storage.example.com/v1alpha/filefolder/list/:address",
          () => {
            deleted = true;
            return new HttpResponse(null, { status: 204 });
          },
        ),
      );
      await expect(runMv(api, folderSrc, folderDst)).rejects.toThrow();
      expect(deleted).toBe(false);
    });

    it("previews empty folder creation without changing resources", async () => {
      const output: string[] = [];
      vi.spyOn(process.stdout, "write").mockImplementation((text) => {
        output.push(String(text));
        return true;
      });
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [],
              next_continuation_handle: null,
            }),
        ),
      );
      await runMv(api, folderSrc, folderDst, noopLogger(), undefined, {
        dryRun: true,
      });
      expect(output.join("")).toContain(`create-folder ${folderDst}`);
      expect(output.join("")).toContain(`delete-folder ${folderSrc}`);
    });

    it("keeps a source folder when a child is skipped on conflict", async () => {
      let deleted = false;
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [file1],
              next_continuation_handle: null,
            }),
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file1Encoded}/move`,
          () => new HttpResponse(null, { status: 409 }),
        ),
        http.delete(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () => {
            deleted = true;
            return new HttpResponse(null, { status: 204 });
          },
        ),
      );
      await runMv(api, folderSrc, folderDst, noopLogger(), undefined, {
        onConflict: "skip",
      });
      expect(deleted).toBe(false);
    });

    it("recursively moves folder contents and deletes source", async () => {
      const movedFiles: string[] = [];
      const deletedFolders: string[] = [];

      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [subFolder],
              sub_resource_addresses: [file1, file2],
              next_continuation_handle: null,
            }),
        ),
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${subFolderEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [subFile],
              next_continuation_handle: null,
            }),
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file1Encoded}/move`,
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            movedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "id-1" });
          },
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file2Encoded}/move`,
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            movedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "id-2" });
          },
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${subFileEncoded}/move`,
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            movedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "id-3" });
          },
        ),
        http.delete(
          `https://storage.example.com/v1alpha/filefolder/list/${subFolderEncoded}`,
          () => {
            deletedFolders.push(subFolder);
            return new HttpResponse(null, { status: 204 });
          },
        ),
        http.delete(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () => {
            deletedFolders.push(folderSrc);
            return new HttpResponse(null, { status: 204 });
          },
        ),
      );

      const out = vi.spyOn(process.stdout, "write");
      await runMv(api, folderSrc, folderDst);

      expect(movedFiles).toContain(`${folderDst}file1.usd`);
      expect(movedFiles).toContain(`${folderDst}file2.usd`);
      expect(movedFiles).toContain(`${folderDst}sub/sub-file.usd`);
      expect(deletedFolders).toContain(subFolder);
      expect(deletedFolders).toContain(folderSrc);
      expect(out).toHaveBeenCalledWith(expect.stringContaining("Moved"));

      out.mockRestore();
    });

    it("respects SN_CONCURRENCY limit for file moves", async () => {
      process.env.SN_CONCURRENCY = "1";
      let activeMoves = 0;
      let maxActiveMoves = 0;

      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [file1, file2],
              next_continuation_handle: null,
            }),
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file1Encoded}/move`,
          async () => {
            activeMoves++;
            maxActiveMoves = Math.max(maxActiveMoves, activeMoves);
            await new Promise((resolve) => setTimeout(resolve, 20));
            activeMoves--;
            return HttpResponse.json({ resource_identity: "id-1" });
          },
        ),
        http.post(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file2Encoded}/move`,
          async () => {
            activeMoves++;
            maxActiveMoves = Math.max(maxActiveMoves, activeMoves);
            await new Promise((resolve) => setTimeout(resolve, 20));
            activeMoves--;
            return HttpResponse.json({ resource_identity: "id-2" });
          },
        ),
        http.delete(
          `https://storage.example.com/v1alpha/filefolder/list/${folderSrcEncoded}`,
          () => new HttpResponse(null, { status: 204 }),
        ),
      );

      await runMv(api, folderSrc, folderDst);
      expect(maxActiveMoves).toBe(1);
    });
  });
});
