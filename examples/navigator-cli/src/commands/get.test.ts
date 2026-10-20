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
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  symlinkSync,
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
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { create } from "@/services/api";
import { noopLogger, type Logger } from "@/output";
import { runGet, runGetLines } from "./get";

const address = "omniverse://nucleus/Library/hello.txt";
const encoded = encodeURIComponent(address);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortAddress = `${topLevelAddress}Library/hello.txt`;
const shortAddressEncoded = encodeURIComponent(shortAddress);

const folderAddress = `${topLevelAddress}Library/`;
const folderAddressEncoded = encodeURIComponent(folderAddress);
const fileA = `${topLevelAddress}Library/a.txt`;
const fileAEncoded = encodeURIComponent(fileA);
const subfolderAddress = `${topLevelAddress}Library/sub/`;
const subfolderAddressEncoded = encodeURIComponent(subfolderAddress);
const fileB = `${topLevelAddress}Library/sub/b.txt`;
const fileBEncoded = encodeURIComponent(fileB);

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
  http.get(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    () =>
      new HttpResponse("hello world", {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      }),
  ),
  http.get(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortAddressEncoded}`,
    () =>
      new HttpResponse("hello world", {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      }),
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => server.resetHandlers());

describe("runGet", () => {
  const api = create("https://storage.example.com");

  it("downloads file to specified local path", async () => {
    const dir = join(tmpdir(), `sn-get-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    await runGet(api, address, localPath);
    expect(existsSync(localPath)).toBe(true);
    expect(readFileSync(localPath, "utf-8")).toBe("hello world");
  });

  it("uses filename from address when no local path given", async () => {
    const destDir = join(tmpdir(), `sn-get-${Date.now()}`);
    mkdirSync(destDir, { recursive: true });
    vi.spyOn(process, "cwd").mockReturnValue(destDir);
    await runGet(api, address, undefined);
    expect(existsSync(join(destDir, "hello.txt"))).toBe(true);
    vi.restoreAllMocks();
  });

  it("resolves a short name to its full address", async () => {
    const dir = join(tmpdir(), `sn-get-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    await runGet(api, "mybucket/Library/hello.txt", localPath);
    expect(existsSync(localPath)).toBe(true);
    expect(readFileSync(localPath, "utf-8")).toBe("hello world");
  });

  it("resolves bare name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const dir = join(tmpdir(), `sn-get-cwd-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    await runGet(api, "hello.txt", localPath, noopLogger(), cwd);
    expect(existsSync(localPath)).toBe(true);
    expect(readFileSync(localPath, "utf-8")).toBe("hello world");
  });

  it("logs download start and complete when logger provided", async () => {
    const dir = join(tmpdir(), `sn-get-verbose-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "out.txt");
    const logs: string[] = [];
    const log: Logger = (label, msg) => logs.push(`[${label}] ${msg}`);
    await runGet(api, address, localPath, log);
    expect(
      logs.some((l) => l.startsWith("[download]") && l.includes("→")),
    ).toBe(true);
    expect(logs).toContain("[download] complete");
    const content = readFileSync(localPath, "utf-8");
    expect(content).toBe("hello world");
  });

  it("writes progress to stderr during download when progress=true", async () => {
    const dir = join(tmpdir(), `sn-get-progress-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");

    const stderrLines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      stderrLines.push(String(msg));
      return true;
    });

    await runGet(api, address, localPath, noopLogger(), undefined, true);
    vi.restoreAllMocks();

    const output = stderrLines.join("");
    expect(output).toContain("hello.txt");
  });

  it("does not write progress when progress=false", async () => {
    const dir = join(tmpdir(), `sn-get-noprogress-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");

    const stderrSpy = vi.spyOn(process.stderr, "write");
    await runGet(api, address, localPath, noopLogger(), undefined, false);
    vi.restoreAllMocks();

    const progressCalls = stderrSpy.mock.calls.filter((c) =>
      String(c[0]).includes("hello.txt"),
    );
    expect(progressCalls.length).toBe(0);
  });

  it("throws if local file already exists and overwrite is false", async () => {
    const dir = join(tmpdir(), `sn-get-nooverwrite-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    // pre-create the file
    writeFileSync(localPath, "existing");
    await expect(
      runGet(api, address, localPath, noopLogger(), undefined, false, false),
    ).rejects.toThrow("File already exists");
  });

  it("overwrites local file when overwrite is true", async () => {
    const dir = join(tmpdir(), `sn-get-overwrite-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "existing");
    await runGet(api, address, localPath, noopLogger(), undefined, false, true);
    expect(readFileSync(localPath, "utf-8")).toBe("hello world");
  });
});

describe("runGet — folder download", () => {
  const api = create("https://storage.example.com");

  beforeEach(() => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileA],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileAEncoded}`,
        () =>
          new HttpResponse("content-a", {
            status: 200,
            headers: { "Content-Type": "text/plain" },
          }),
      ),
    );
  });

  it("preserves nested empty directories", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-empty-"));
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/filefolder/list/:address",
        ({ params }) =>
          HttpResponse.json({
            subfolder_addresses:
              params.address === folderAddress ? [subfolderAddress] : [],
            sub_resource_addresses: [],
            next_continuation_handle: null,
          }),
      ),
    );
    await runGet(api, folderAddress, dest);
    expect(existsSync(join(dest, "Library", "sub"))).toBe(true);
  });

  it.each([
    `${folderAddress}%2e%2e%2fescaped.txt`,
    `${folderAddress}sub/%2e%2e/escaped.txt`,
    `${folderAddress}sub%5cescaped.txt`,
    `${folderAddress}C%3aescaped.txt`,
    `${folderAddress}bad%00name.txt`,
    `${folderAddress}bad%name.txt`,
    `${folderAddress}sub//escaped.txt`,
    `${topLevelAddress}Other/escaped.txt`,
  ])(
    "rejects unsafe listing address %s before writing",
    async (unsafeAddress) => {
      const dest = mkdtempSync(join(tmpdir(), "sn-get-unsafe-"));
      let downloads = 0;
      server.use(
        http.get(
          `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
          () =>
            HttpResponse.json({
              subfolder_addresses: [],
              sub_resource_addresses: [fileA, unsafeAddress],
              next_continuation_handle: null,
            }),
        ),
        http.get(
          "https://storage.example.com/v1alpha/fileobject/by-address/:address",
          () => {
            downloads++;
            return new HttpResponse("unsafe");
          },
        ),
      );
      await expect(runGet(api, folderAddress, dest)).rejects.toMatchObject({
        code: "INVALID_PATH",
      });
      expect(downloads).toBe(0);
      expect(existsSync(join(dest, "Library", "a.txt"))).toBe(false);
      expect(existsSync(join(dest, "escaped.txt"))).toBe(false);
    },
  );

  it("rejects unsafe subfolder addresses before following them", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-subfolder-"));
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [`${folderAddress}%2e%2e%2fOutside/`],
            sub_resource_addresses: [],
            next_continuation_handle: null,
          }),
      ),
    );
    await expect(runGet(api, folderAddress, dest)).rejects.toMatchObject({
      code: "INVALID_PATH",
    });
    expect(existsSync(join(dest, "Outside"))).toBe(false);
  });

  it("rejects existing symlink directories inside the download root", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-symlink-"));
    const outside = mkdtempSync(join(tmpdir(), "sn-get-outside-"));
    symlinkSync(outside, join(dest, "Library"), "dir");
    await expect(runGet(api, folderAddress, dest)).rejects.toMatchObject({
      code: "INVALID_PATH",
    });
    expect(existsSync(join(outside, "a.txt"))).toBe(false);
  });

  it("does not modify a symlink target when overwriting a downloaded file", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-leaf-link-"));
    const outside = mkdtempSync(join(tmpdir(), "sn-get-leaf-outside-"));
    const target = join(outside, "original.txt");
    writeFileSync(target, "original");
    mkdirSync(join(dest, "Library"));
    symlinkSync(target, join(dest, "Library", "a.txt"), "file");
    await runGet(
      api,
      folderAddress,
      dest,
      noopLogger(),
      undefined,
      false,
      true,
    );
    expect(readFileSync(target, "utf8")).toBe("original");
    expect(readFileSync(join(dest, "Library", "a.txt"), "utf8")).toBe(
      "content-a",
    );
  });

  it("decodes safe path segments exactly once", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-encoded-"));
    const encodedName = `${folderAddress}space%20and%20%252F.txt`;
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [encodedName],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(encodedName)}`,
        () => new HttpResponse("content"),
      ),
    );
    await runGet(api, folderAddress, dest);
    expect(
      readFileSync(join(dest, "Library", "space and %2F.txt"), "utf8"),
    ).toBe("content");
  });

  it("downloads all files in a flat folder to localDest/folderName/", async () => {
    const dest = join(tmpdir(), `sn-get-folder-flat-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    await runGet(api, folderAddress, dest);

    const localFile = join(dest, "Library", "a.txt");
    expect(existsSync(localFile)).toBe(true);
    expect(readFileSync(localFile, "utf-8")).toBe("content-a");
  });

  it("defaults to process.cwd()/folderName when no localDest given", async () => {
    const dest = join(tmpdir(), `sn-get-folder-cwd-${Date.now()}`);
    mkdirSync(dest, { recursive: true });
    vi.spyOn(process, "cwd").mockReturnValue(dest);

    await runGet(api, folderAddress, undefined);
    vi.restoreAllMocks();

    expect(existsSync(join(dest, "Library", "a.txt"))).toBe(true);
  });

  it("recursively downloads files in subfolders", async () => {
    const dest = join(tmpdir(), `sn-get-folder-recursive-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [subfolderAddress],
            sub_resource_addresses: [fileA],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${subfolderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileB],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileAEncoded}`,
        () =>
          new HttpResponse("content-a", {
            status: 200,
            headers: { "Content-Type": "text/plain" },
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileBEncoded}`,
        () =>
          new HttpResponse("content-b", {
            status: 200,
            headers: { "Content-Type": "text/plain" },
          }),
      ),
    );

    await runGet(api, folderAddress, dest);

    expect(existsSync(join(dest, "Library", "a.txt"))).toBe(true);
    expect(readFileSync(join(dest, "Library", "a.txt"), "utf-8")).toBe(
      "content-a",
    );
    expect(existsSync(join(dest, "Library", "sub", "b.txt"))).toBe(true);
    expect(readFileSync(join(dest, "Library", "sub", "b.txt"), "utf-8")).toBe(
      "content-b",
    );
  });

  it("throws if any local file exists and overwrite is false", async () => {
    const dest = join(tmpdir(), `sn-get-folder-nooverwrite-${Date.now()}`);
    mkdirSync(join(dest, "Library"), { recursive: true });
    writeFileSync(join(dest, "Library", "a.txt"), "existing");

    await expect(
      runGet(api, folderAddress, dest, noopLogger(), undefined, false, false),
    ).rejects.toThrow("File already exists");
  });

  it("overwrites existing files when overwrite is true", async () => {
    const dest = join(tmpdir(), `sn-get-folder-overwrite-${Date.now()}`);
    mkdirSync(join(dest, "Library"), { recursive: true });
    writeFileSync(join(dest, "Library", "a.txt"), "existing");

    await runGet(
      api,
      folderAddress,
      dest,
      noopLogger(),
      undefined,
      false,
      true,
    );

    expect(readFileSync(join(dest, "Library", "a.txt"), "utf-8")).toBe(
      "content-a",
    );
  });

  it("writes folder progress to stderr when progress=true", async () => {
    const dest = join(tmpdir(), `sn-get-folder-progress-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    const stderrLines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      stderrLines.push(String(msg));
      return true;
    });

    await runGet(api, folderAddress, dest, noopLogger(), undefined, true);
    vi.restoreAllMocks();

    const output = stderrLines.join("");
    expect(output).toContain("Downloading");
    expect(output).toContain("a.txt");
    expect(output).toContain("1/1 files");
  });

  it("does not write folder progress when progress=false", async () => {
    const dest = join(tmpdir(), `sn-get-folder-noprogress-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    const stderrSpy = vi.spyOn(process.stderr, "write");
    await runGet(api, folderAddress, dest, noopLogger(), undefined, false);
    vi.restoreAllMocks();

    const progressCalls = stderrSpy.mock.calls.filter((c) =>
      String(c[0]).includes("a.txt"),
    );
    expect(progressCalls.length).toBe(0);
  });
});

describe("runGetLines", () => {
  const api = create("https://storage.example.com");

  it("reports a conflict for piped files with the same local filename", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-collision-"));
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        () => new HttpResponse("content"),
      ),
    );
    await expect(
      runGetLines(
        api,
        ["https://blob.example/a/same.txt", "https://blob.example/b/same.txt"],
        dest,
      ),
    ).rejects.toMatchObject({ code: "PARTIAL_FAILURE" });
  });

  it("reports duplicate input destinations even with overwrite enabled", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-overwrite-collision-"));
    writeFileSync(join(dest, "same.txt"), "original");
    server.use(
      http.get(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        ({ params }) => new HttpResponse(String(params.address)),
      ),
    );
    await expect(
      runGetLines(
        api,
        ["https://blob.example/a/same.txt", "https://blob.example/b/same.txt"],
        dest,
        noopLogger(),
        undefined,
        false,
        true,
      ),
    ).rejects.toMatchObject({ code: "PARTIAL_FAILURE" });
    expect(readFileSync(join(dest, "same.txt"), "utf8")).toBe(
      "https://blob.example/a/same.txt",
    );
  });

  it("rejects encoded separators in default file destinations", async () => {
    const dest = mkdtempSync(join(tmpdir(), "sn-get-name-"));
    await expect(
      runGetLines(api, ["https://blob.example/%2e%2e%2fescaped.txt"], dest),
    ).rejects.toMatchObject({ code: "PARTIAL_FAILURE" });
    expect(existsSync(join(dest, "escaped.txt"))).toBe(false);
  });

  it("downloads each piped file to localDest/<filename>", async () => {
    const dest = join(tmpdir(), `sn-getlines-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    await runGetLines(api, [address], dest, noopLogger());

    expect(existsSync(join(dest, "hello.txt"))).toBe(true);
    expect(readFileSync(join(dest, "hello.txt"), "utf-8")).toBe("hello world");
  });

  it("collects errors and rejects with failure count", async () => {
    server.use(
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
    );

    await expect(
      runGetLines(api, [address, address], undefined, noopLogger()),
    ).rejects.toThrow("2 error(s)");
  });

  it("succeeds when overwrite=true and file already exists", async () => {
    const dest = join(tmpdir(), `sn-getlines-overwrite-${Date.now()}`);
    mkdirSync(dest, { recursive: true });
    const { writeFileSync: wfs } = await import("fs");
    wfs(join(dest, "hello.txt"), "existing");

    await runGetLines(
      api,
      [address],
      dest,
      noopLogger(),
      undefined,
      false,
      true,
    );

    expect(readFileSync(join(dest, "hello.txt"), "utf-8")).toBe("hello world");
  });

  it("rejects when overwrite=false and file already exists", async () => {
    const dest = join(tmpdir(), `sn-getlines-nooverwrite-${Date.now()}`);
    mkdirSync(dest, { recursive: true });
    const { writeFileSync: wfs } = await import("fs");
    wfs(join(dest, "hello.txt"), "existing");

    await expect(
      runGetLines(api, [address], dest, noopLogger(), undefined, false, false),
    ).rejects.toThrow("1 error(s)");
  });

  it("passes localDest unchanged for folder lines (trailing /)", async () => {
    // folder lines end with "/" — dest should be localDest, not localDest/<filename>
    // runGet receives folderAddress and appends the folder name itself (Library/)
    // so the final file lands at dest/Library/a.txt, not dest/<something>/Library/a.txt
    const dest = join(tmpdir(), `sn-getlines-folder-${Date.now()}`);
    mkdirSync(dest, { recursive: true });

    server.use(
      http.get(
        `https://storage.example.com/v1alpha/filefolder/list/${folderAddressEncoded}`,
        () =>
          HttpResponse.json({
            subfolder_addresses: [],
            sub_resource_addresses: [fileA],
            next_continuation_handle: null,
          }),
      ),
      http.get(
        `https://storage.example.com/v1alpha/fileobject/by-address/${fileAEncoded}`,
        () =>
          new HttpResponse("content-a", {
            status: 200,
            headers: { "Content-Type": "text/plain" },
          }),
      ),
    );

    await runGetLines(api, [folderAddress], dest, noopLogger());

    // File should be at dest/Library/a.txt — not dest/<folderName>/Library/a.txt
    expect(existsSync(join(dest, "Library", "a.txt"))).toBe(true);
    expect(readFileSync(join(dest, "Library", "a.txt"), "utf-8")).toBe(
      "content-a",
    );
  });
});
