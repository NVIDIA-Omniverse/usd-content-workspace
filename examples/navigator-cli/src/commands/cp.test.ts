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
import { runCp } from "./cp";

const src = "omniverse://nucleus/Library/scene.usd";
const dst = "omniverse://nucleus/Library/copy.usd";
const srcEncoded = encodeURIComponent(src);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortSrc = `${topLevelAddress}Library/scene.usd`;
const shortSrcEncoded = encodeURIComponent(shortSrc);

const folderSrc = "omniverse://nucleus/Library/src-folder/";
const folderDst = "omniverse://nucleus/Library/dst-folder/";
const file1 = `${folderSrc}file1.usd`;
const file2 = `${folderSrc}file2.usd`;
const subFolder = `${folderSrc}sub/`;
const subFile = `${subFolder}sub-file.usd`;
const folderSrcEncoded = encodeURIComponent(folderSrc);
const subFolderEncoded = encodeURIComponent(subFolder);
const file1Encoded = encodeURIComponent(file1);
const file2Encoded = encodeURIComponent(file2);
const subFileEncoded = encodeURIComponent(subFile);

const statHeaders = {
  "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
    data_object_size: 1024,
  }),
  "X-Nvidia-Omniverse-Storage-Resource-Identity": "src-identity",
};

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
  http.head(
    `https://storage.example.com/v1alpha/fileobject/by-address/${srcEncoded}`,
    () => new HttpResponse(null, { status: 204, headers: statHeaders }),
  ),
  http.head(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortSrcEncoded}`,
    () => new HttpResponse(null, { status: 204, headers: statHeaders }),
  ),
  http.post(
    "https://storage.example.com/v1alpha/fileobject/by-identity/src-identity/copy",
    () => HttpResponse.json({ resource_identity: "copy-id" }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runCp", () => {
  it("reports successful earlier copies when a later child fails", async () => {
    const source = "omniverse://nucleus/Library/partial/";
    const destination = "omniverse://nucleus/Archive/partial/";
    const child = `${source}child/`;
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/filefolder/list/:address",
        ({ params }) =>
          HttpResponse.json({
            subfolder_addresses: params.address === source ? [child] : [],
            sub_resource_addresses:
              params.address === source
                ? [`${source}done.usd`]
                : [`${child}failed.usd`],
            next_continuation_handle: null,
          }),
      ),
      http.head(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        ({ params }) =>
          new HttpResponse(null, {
            status: 204,
            headers: {
              ...statHeaders,
              "X-Nvidia-Omniverse-Storage-Resource-Identity": String(
                params.address,
              ),
            },
          }),
      ),
      http.post(
        "https://storage.example.com/v1alpha/fileobject/by-identity/:identity/copy",
        ({ params }) =>
          String(params.identity).endsWith("failed.usd")
            ? new HttpResponse(null, { status: 500 })
            : HttpResponse.json({ resource_identity: "copied" }),
      ),
    );
    await expect(runCp(api, source, destination)).rejects.toMatchObject({
      code: "PARTIAL_FAILURE",
      details: {
        completedOperations: [
          { action: "create-folder", source: destination },
          {
            action: "copy",
            source: `${source}done.usd`,
            destination: `${destination}done.usd`,
          },
          { action: "create-folder", source: `${destination}child/` },
        ],
      },
    });
  });

  it("bounds file work across sibling folders instead of multiplying by depth", async () => {
    const source = "omniverse://nucleus/Library/bounded/";
    const previous = process.env.SN_CONCURRENCY;
    process.env.SN_CONCURRENCY = "2";
    let active = 0;
    let peak = 0;
    let copied = 0;
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/filefolder/list/:address",
        ({ params }) => {
          const folder = String(params.address);
          return HttpResponse.json({
            subfolder_addresses:
              folder === source
                ? [`${source}one/`, `${source}two/`, `${source}three/`]
                : [],
            sub_resource_addresses:
              folder === source
                ? []
                : Array.from({ length: 5 }, (_, i) => `${folder}${i}.usd`),
            next_continuation_handle: null,
          });
        },
      ),
      http.head(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        () => new HttpResponse(null, { status: 204, headers: statHeaders }),
      ),
      http.post(
        "https://storage.example.com/v1alpha/fileobject/by-identity/src-identity/copy",
        async () => {
          peak = Math.max(peak, ++active);
          await new Promise((resolve) => setTimeout(resolve, 10));
          active--;
          copied++;
          return HttpResponse.json({ resource_identity: "copy-id" });
        },
      ),
    );
    try {
      await runCp(api, source, "omniverse://nucleus/Archive/bounded/");
      expect(copied).toBe(15);
      expect(peak).toBe(2);
      expect(active).toBe(0);
    } finally {
      if (previous === undefined) delete process.env.SN_CONCURRENCY;
      else process.env.SN_CONCURRENCY = previous;
    }
  });

  const api = create("https://storage.example.com");

  it("preserves nested empty directories when copying", async () => {
    const source = "omniverse://nucleus/Library/empty/";
    const destination = "omniverse://nucleus/Archive/empty/";
    const created: string[] = [];
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/filefolder/list/:address",
        ({ params }) =>
          HttpResponse.json({
            subfolder_addresses:
              params.address === source ? [`${source}child/`] : [],
            sub_resource_addresses: [],
            next_continuation_handle: null,
          }),
      ),
      http.put(
        "https://storage.example.com/v1alpha/filefolder/:address",
        ({ params }) => {
          created.push(String(params.address));
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    await runCp(api, source, destination);
    expect(created).toEqual([destination, `${destination}child/`]);
  });

  it("copies file from src to dst", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await expect(runCp(api, src, dst)).resolves.toBeUndefined();
  });

  it("previews without copying and requests atomic conflict protection", async () => {
    let copies = 0;
    let body: unknown;
    server.use(
      http.post(
        "https://storage.example.com/v1alpha/fileobject/by-identity/src-identity/copy",
        async ({ request }) => {
          copies++;
          body = await request.json();
          return HttpResponse.json({ resource_identity: "copy-id" });
        },
      ),
    );
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((text) => {
      output.push(String(text));
      return true;
    });
    await runCp(api, src, dst, noopLogger(), undefined, { dryRun: true });
    expect(copies).toBe(0);
    expect(output.join("")).toContain(dst);
    await runCp(api, src, dst);
    expect(body).toEqual({
      destination_resource_address: dst,
      fail_if_exists: true,
    });
  });

  it("resolves short names to full addresses", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    await expect(
      runCp(api, "mybucket/Library/scene.usd", "mybucket/Library/copy.usd"),
    ).resolves.toBeUndefined();
  });

  it("resolves bare src and dst relative to cwd", async () => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    await expect(
      runCp(api, "scene.usd", "copy.usd", noopLogger(), cwd),
    ).resolves.toBeUndefined();
  });

  describe("folder copy", () => {
    afterEach(() => {
      delete process.env.SN_CONCURRENCY;
    });

    it("recursively copies folder contents without deleting source", async () => {
      const copiedFiles: string[] = [];

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
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file1Encoded}`,
          () =>
            new HttpResponse(null, {
              status: 204,
              headers: {
                "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
                  data_object_size: 100,
                }),
                "X-Nvidia-Omniverse-Storage-Resource-Identity": "id-file1",
              },
            }),
        ),
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file2Encoded}`,
          () =>
            new HttpResponse(null, {
              status: 204,
              headers: {
                "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
                  data_object_size: 200,
                }),
                "X-Nvidia-Omniverse-Storage-Resource-Identity": "id-file2",
              },
            }),
        ),
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${subFileEncoded}`,
          () =>
            new HttpResponse(null, {
              status: 204,
              headers: {
                "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
                  data_object_size: 300,
                }),
                "X-Nvidia-Omniverse-Storage-Resource-Identity": "id-subfile",
              },
            }),
        ),
        http.post(
          "https://storage.example.com/v1alpha/fileobject/by-identity/id-file1/copy",
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            copiedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "new-id-1" });
          },
        ),
        http.post(
          "https://storage.example.com/v1alpha/fileobject/by-identity/id-file2/copy",
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            copiedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "new-id-2" });
          },
        ),
        http.post(
          "https://storage.example.com/v1alpha/fileobject/by-identity/id-subfile/copy",
          async ({ request }) => {
            const body = (await request.json()) as {
              destination_resource_address: string;
            };
            copiedFiles.push(body.destination_resource_address);
            return HttpResponse.json({ resource_identity: "new-id-3" });
          },
        ),
      );

      const out = vi.spyOn(process.stdout, "write");
      await runCp(api, folderSrc, folderDst);

      expect(copiedFiles).toContain(`${folderDst}file1.usd`);
      expect(copiedFiles).toContain(`${folderDst}file2.usd`);
      expect(copiedFiles).toContain(`${folderDst}sub/sub-file.usd`);
      expect(out).toHaveBeenCalledWith(expect.stringContaining("Copied"));

      out.mockRestore();
    });

    it("respects SN_CONCURRENCY limit for file copies", async () => {
      process.env.SN_CONCURRENCY = "1";
      let activeCopies = 0;
      let maxActiveCopies = 0;

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
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file1Encoded}`,
          () =>
            new HttpResponse(null, {
              status: 204,
              headers: {
                "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
                  data_object_size: 100,
                }),
                "X-Nvidia-Omniverse-Storage-Resource-Identity": "id-file1",
              },
            }),
        ),
        http.head(
          `https://storage.example.com/v1alpha/fileobject/by-address/${file2Encoded}`,
          () =>
            new HttpResponse(null, {
              status: 204,
              headers: {
                "X-Nvidia-Omniverse-Storage-Metadata": JSON.stringify({
                  data_object_size: 200,
                }),
                "X-Nvidia-Omniverse-Storage-Resource-Identity": "id-file2",
              },
            }),
        ),
        http.post(
          "https://storage.example.com/v1alpha/fileobject/by-identity/id-file1/copy",
          async () => {
            activeCopies++;
            maxActiveCopies = Math.max(maxActiveCopies, activeCopies);
            await new Promise((resolve) => setTimeout(resolve, 20));
            activeCopies--;
            return HttpResponse.json({ resource_identity: "new-id-1" });
          },
        ),
        http.post(
          "https://storage.example.com/v1alpha/fileobject/by-identity/id-file2/copy",
          async () => {
            activeCopies++;
            maxActiveCopies = Math.max(maxActiveCopies, activeCopies);
            await new Promise((resolve) => setTimeout(resolve, 20));
            activeCopies--;
            return HttpResponse.json({ resource_identity: "new-id-2" });
          },
        ),
      );

      await runCp(api, folderSrc, folderDst);
      expect(maxActiveCopies).toBe(1);
    });
  });
});
