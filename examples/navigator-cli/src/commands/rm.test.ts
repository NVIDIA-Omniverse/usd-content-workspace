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
import { runRm, runRmLines } from "./rm";

const fileAddress = "omniverse://nucleus/Library/scene.usd";
const fileEncoded = encodeURIComponent(fileAddress);
const folderAddress = "omniverse://nucleus/Library/Sub/";
const folderEncoded = encodeURIComponent(folderAddress);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortFile = `${topLevelAddress}Library/scene.usd`;
const shortFileEncoded = encodeURIComponent(shortFile);
const shortFolderNoSlash = `${topLevelAddress}Library/Sub`;
const shortFolderWithSlash = `${topLevelAddress}Library/Sub/`;
const shortFolderEncoded = encodeURIComponent(shortFolderWithSlash);

const server = setupServer(
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
  http.delete(
    `https://storage.example.com/v1alpha/fileobject/by-address/${fileEncoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: [],
        sub_resource_addresses: [],
        next_continuation_handle: null,
      }),
  ),
  http.delete(
    `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.delete(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortFileEncoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/filefolder/list/${shortFolderEncoded}`,
    () =>
      HttpResponse.json({
        subfolder_addresses: [],
        sub_resource_addresses: [],
        next_continuation_handle: null,
      }),
  ),
  http.delete(
    `https://storage.example.com/v1alpha/filefolder/list/${shortFolderEncoded}`,
    () => new HttpResponse(null, { status: 204 }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runRm", () => {
  afterEach(() => {
    delete process.env.SN_CONCURRENCY;
  });

  const api = create("https://storage.example.com");

  it("deletes a file", async () => {
    await expect(
      runRm(api, fileAddress, { recursive: false }),
    ).resolves.toBeUndefined();
  });

  it("previews deletion without sending DELETE", async () => {
    let deletes = 0;
    server.use(
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileEncoded}`,
        () => {
          deletes++;
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((text) => {
      output.push(String(text));
      return true;
    });
    await runRm(api, fileAddress, { recursive: false, dryRun: true });
    expect(deletes).toBe(0);
    expect(output.join("")).toContain(fileAddress);
  });

  it("deletes an empty folder with -r flag", async () => {
    await expect(
      runRm(api, folderAddress, { recursive: true }),
    ).resolves.toBeUndefined();
  });

  it("throws if folder path used without -r", async () => {
    await expect(
      runRm(api, folderAddress, { recursive: false }),
    ).rejects.toThrow("use -r");
  });

  it("resolves a short name to its full address", async () => {
    await expect(
      runRm(api, "mybucket/Library/scene.usd", { recursive: false }),
    ).resolves.toBeUndefined();
  });

  it("resolves bare file name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    await expect(
      runRm(api, "scene.usd", { recursive: false }, noopLogger(), cwd),
    ).resolves.toBeUndefined();
  });

  it("deletes a folder by path without trailing slash when -r is set", async () => {
    await expect(
      runRm(api, shortFolderNoSlash, { recursive: true }),
    ).resolves.toBeUndefined();
  });

  it("deletes all files in a folder before deleting the folder", async () => {
    const fileInFolder = `${folderAddress}file.usd`;
    const fileInFolderEncoded = encodeURIComponent(fileInFolder);
    const deletedFiles: string[] = [];
    const deletedFolders: string[] = [];

    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileInFolder],
            next_continuation_handle: null,
          }),
      ),
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileInFolderEncoded}`,
        () => {
          deletedFiles.push(fileInFolder);
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.delete(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () => {
          deletedFolders.push(folderAddress);
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );

    await runRm(api, folderAddress, { recursive: true });

    expect(deletedFiles).toEqual([fileInFolder]);
    expect(deletedFolders).toEqual([folderAddress]);
  });

  it("recursively deletes files in subfolders, children before parents", async () => {
    const subfolderAddress = `${folderAddress}Sub/`;
    const subfolderEncoded = encodeURIComponent(subfolderAddress);
    const fileInSubfolder = `${subfolderAddress}file.usd`;
    const fileInSubfolderEncoded = encodeURIComponent(fileInSubfolder);
    const deletionOrder: string[] = [];

    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [subfolderAddress],
            sub_resource_addresses: [],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${subfolderEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileInSubfolder],
            next_continuation_handle: null,
          }),
      ),
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileInSubfolderEncoded}`,
        () => {
          deletionOrder.push(fileInSubfolder);
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.delete(
        `https://storage.example.com/v1alpha/filefolder/list/${subfolderEncoded}`,
        () => {
          deletionOrder.push(subfolderAddress);
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.delete(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () => {
          deletionOrder.push(folderAddress);
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );

    await runRm(api, folderAddress, { recursive: true });

    expect(deletionOrder).toContain(fileInSubfolder);
    expect(deletionOrder).toContain(subfolderAddress);
    expect(deletionOrder).toContain(folderAddress);
    expect(deletionOrder.indexOf(fileInSubfolder)).toBeLessThan(
      deletionOrder.indexOf(subfolderAddress),
    );
    expect(deletionOrder.indexOf(subfolderAddress)).toBeLessThan(
      deletionOrder.indexOf(folderAddress),
    );
  });

  it("respects SN_CONCURRENCY limit for file deletions", async () => {
    process.env.SN_CONCURRENCY = "1";

    const fileA = `${folderAddress}a.usd`;
    const fileB = `${folderAddress}b.usd`;
    const fileAEncoded = encodeURIComponent(fileA);
    const fileBEncoded = encodeURIComponent(fileB);
    let activeDeletes = 0;
    let maxActiveDeletes = 0;

    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileA, fileB],
            next_continuation_handle: null,
          }),
      ),
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileAEncoded}`,
        async () => {
          activeDeletes++;
          maxActiveDeletes = Math.max(maxActiveDeletes, activeDeletes);
          await new Promise((resolve) => setTimeout(resolve, 20));
          activeDeletes--;
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileBEncoded}`,
        async () => {
          activeDeletes++;
          maxActiveDeletes = Math.max(maxActiveDeletes, activeDeletes);
          await new Promise((resolve) => setTimeout(resolve, 20));
          activeDeletes--;
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.delete(
        `https://storage.example.com/v1alpha/filefolder/list/${folderEncoded}`,
        () => new HttpResponse(null, { status: 204 }),
      ),
    );

    await runRm(api, folderAddress, { recursive: true });
    expect(maxActiveDeletes).toBe(1);
  });
});

const mockApi = create("https://storage.example.com");

describe("runRmLines — stdin mode", () => {
  it("deletes each piped file path", async () => {
    let deleteCalled = 0;
    server.use(
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${shortFileEncoded}`,
        () => {
          deleteCalled++;
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    await runRmLines(mockApi, [shortFile], { recursive: false });
    expect(deleteCalled).toBe(1);
  });

  it("applies -r to each piped path (recursive delete)", async () => {
    let deleteCalled = 0;
    server.use(
      http.delete(
        `https://storage.example.com/v1alpha/filefolder/list/${shortFolderEncoded}`,
        () => {
          deleteCalled++;
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    await runRmLines(mockApi, [shortFolderWithSlash], { recursive: true });
    expect(deleteCalled).toBe(1);
  });

  it("collects errors and rejects with failure count", async () => {
    server.use(
      http.delete(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileEncoded}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    const stderrSpy = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    await expect(
      runRmLines(mockApi, [fileAddress, fileAddress], { recursive: false }),
    ).rejects.toThrow("2 error(s)");
    stderrSpy.mockRestore();
  });
});
