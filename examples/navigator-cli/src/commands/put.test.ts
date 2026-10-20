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

import { mkdirSync, mkdtempSync, writeFileSync } from "fs";
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
import { noopLogger, type Logger } from "@/output";
import { runPut, runPutLines } from "./put";

const address = "omniverse://nucleus/Library/hello.txt";
const encoded = encodeURIComponent(address);

const topLevelAddress = "https://mybucket.blob.core.windows.net/mybucket/";
const shortAddress = `${topLevelAddress}Library/hello.txt`;
const shortAddressEncoded = encodeURIComponent(shortAddress);

let received = false;

const server = setupServer(
  http.put(
    "https://storage.example.com/v1alpha/filefolder/:address",
    () => new HttpResponse(null, { status: 204 }),
  ),
  http.head(
    "https://storage.example.com/v1alpha/fileobject/by-address/*",
    () => new HttpResponse(null, { status: 404 }),
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
  http.put(
    `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
    async ({ request }) => {
      await request.arrayBuffer();
      received = true;
      return new HttpResponse(null, { status: 201 });
    },
  ),
  http.put(
    `https://storage.example.com/v1alpha/fileobject/by-address/${shortAddressEncoded}`,
    async ({ request }) => {
      await request.arrayBuffer();
      received = true;
      return new HttpResponse(null, { status: 201 });
    },
  ),
);

beforeAll(() => server.listen());
afterAll(() => server.close());
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
  received = false;
});

describe("runPut", () => {
  it("encodes local file and directory names exactly once in storage addresses", async () => {
    const root = mkdtempSync(join(tmpdir(), "sn-put-names-"));
    const dirName = "folder #?% space";
    const fileName = "file #?%2F space.txt";
    const directory = join(root, dirName);
    mkdirSync(directory);
    writeFileSync(join(directory, fileName), "contents");
    const received: string[] = [];
    server.use(
      http.put(
        "https://storage.example.com/v1alpha/filefolder/:address",
        ({ params }) => {
          received.push(String(params.address));
          return new HttpResponse(null, { status: 204 });
        },
      ),
      http.head(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        () => new HttpResponse(null, { status: 404 }),
      ),
      http.put(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        ({ params }) => {
          received.push(String(params.address));
          return new HttpResponse(null, { status: 201 });
        },
      ),
    );
    const base = "omniverse://nucleus/Library/";
    await runPut(api, directory, base);
    await runPut(api, join(directory, fileName), undefined, noopLogger(), base);
    expect(received).toEqual([
      `${base}${encodeURIComponent(dirName)}/`,
      `${base}${encodeURIComponent(dirName)}/${encodeURIComponent(fileName)}`,
      `${base}${encodeURIComponent(fileName)}`,
    ]);
  });

  it("bounds destination checks across a large directory", async () => {
    const directory = mkdtempSync(join(tmpdir(), "sn-put-bounded-"));
    for (let index = 0; index < 20; index++)
      writeFileSync(join(directory, `${index}.txt`), "file");
    const previous = process.env.SN_CONCURRENCY;
    process.env.SN_CONCURRENCY = "2";
    let active = 0;
    let peak = 0;
    server.use(
      http.head(
        "https://storage.example.com/v1alpha/fileobject/by-address/:address",
        async () => {
          peak = Math.max(peak, ++active);
          await new Promise((resolve) => setTimeout(resolve, 10));
          active--;
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    try {
      await runPut(
        api,
        directory,
        "omniverse://nucleus/Library/",
        noopLogger(),
        undefined,
        false,
        { onConflict: "skip" },
      );
      expect(peak).toBe(2);
      expect(active).toBe(0);
    } finally {
      if (previous === undefined) delete process.env.SN_CONCURRENCY;
      else process.env.SN_CONCURRENCY = previous;
    }
  });

  const api = create("https://storage.example.com");

  it("preserves nested empty directories during upload and preview", async () => {
    const base = mkdtempSync(join(tmpdir(), "sn-put-empty-"));
    const source = join(base, "empty");
    mkdirSync(join(source, "child"), { recursive: true });
    const created: string[] = [];
    server.use(
      http.put(
        "https://storage.example.com/v1alpha/filefolder/:address",
        ({ params }) => {
          created.push(String(params.address));
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((text) => {
      output.push(String(text));
      return true;
    });
    await runPut(
      api,
      source,
      "omniverse://nucleus/Library/",
      noopLogger(),
      undefined,
      false,
      { dryRun: true },
    );
    expect(created).toEqual([]);
    expect(output.join("")).toContain(
      "create-folder omniverse://nucleus/Library/empty/child/",
    );
    await runPut(api, source, "omniverse://nucleus/Library/");
    expect(created).toEqual([
      "omniverse://nucleus/Library/empty/",
      "omniverse://nucleus/Library/empty/child/",
    ]);
    vi.restoreAllMocks();
  });

  it("uploads local file to remote path", async () => {
    const dir = join(tmpdir(), `sn-put-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await runPut(api, localPath, address);
    expect(received).toBe(true);
  });

  it("fails closed on an existing destination and previews without upload", async () => {
    const dir = join(tmpdir(), `sn-put-conflict-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    let checks = 0;
    server.use(
      http.head(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encoded}`,
        () => {
          checks++;
          return new HttpResponse(null, { status: 204 });
        },
      ),
    );
    const output: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((text) => {
      output.push(String(text));
      return true;
    });
    await runPut(api, localPath, address, noopLogger(), undefined, false, {
      dryRun: true,
    });
    expect(checks).toBe(0);
    expect(received).toBe(false);
    expect(output.join("")).toContain(address);
    await expect(runPut(api, localPath, address)).rejects.toThrow(
      "Destination already exists",
    );
    expect(received).toBe(false);
    await runPut(api, localPath, address, noopLogger(), undefined, false, {
      onConflict: "skip",
    });
    expect(received).toBe(false);
  });

  it("resolves a short name to its full address", async () => {
    const dir = join(tmpdir(), `sn-put-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await runPut(api, localPath, "mybucket/Library/hello.txt");
    expect(received).toBe(true);
  });

  it("resolves bare name relative to cwd", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const dir = join(tmpdir(), `sn-put-cwd-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await runPut(api, localPath, "hello.txt", noopLogger(), cwd);
    expect(received).toBe(true);
  });

  it("defaults remote path to local filename in cwd when no remote path given", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const dir = join(tmpdir(), `sn-put-default-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await runPut(api, localPath, undefined, noopLogger(), cwd);
    expect(received).toBe(true);
  });

  it("throws when no remote path and no cwd", async () => {
    const dir = join(tmpdir(), `sn-put-nocwd-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await expect(runPut(api, localPath, undefined)).rejects.toThrow(
      "no current folder set",
    );
  });

  it("logs upload start and complete when logger provided", async () => {
    const dir = join(tmpdir(), `sn-put-verbose-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    const logs: string[] = [];
    const log: Logger = (label, msg) => logs.push(`[${label}] ${msg}`);
    await runPut(api, localPath, address, log);
    expect(logs.some((l) => l.startsWith("[upload]") && l.includes("→"))).toBe(
      true,
    );
    expect(logs).toContain("[upload] using direct upload");
    expect(logs).toContain("[upload] complete");
  });

  it("uploads all files in a directory preserving structure", async () => {
    const base = join(tmpdir(), `sn-put-folder-${Date.now()}`);
    mkdirSync(join(base, "mydir", "sub"), { recursive: true });
    writeFileSync(join(base, "mydir", "a.txt"), "a");
    writeFileSync(join(base, "mydir", "sub", "b.txt"), "b");

    const received = new Set<string>();
    const addrA = "omniverse://nucleus/Library/mydir/a.txt";
    const addrB = "omniverse://nucleus/Library/mydir/sub/b.txt";
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(addrA)}`,
        () => {
          received.add("a.txt");
          return new HttpResponse(null, { status: 201 });
        },
      ),
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(addrB)}`,
        () => {
          received.add("sub/b.txt");
          return new HttpResponse(null, { status: 201 });
        },
      ),
    );

    await runPut(api, join(base, "mydir"), "omniverse://nucleus/Library/");
    expect(received.has("a.txt")).toBe(true);
    expect(received.has("sub/b.txt")).toBe(true);
    expect(received.size).toBe(2);
  });

  it("defaults folder upload to cwd/folderName when no remote path given", async () => {
    const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";
    const base = join(tmpdir(), `sn-put-folder-cwd-${Date.now()}`);
    mkdirSync(join(base, "mydir"), { recursive: true });
    writeFileSync(join(base, "mydir", "a.txt"), "a");

    const addrA =
      "https://mybucket.blob.core.windows.net/mybucket/Library/mydir/a.txt";
    const received = new Set<string>();
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(addrA)}`,
        () => {
          received.add("a.txt");
          return new HttpResponse(null, { status: 201 });
        },
      ),
    );

    await runPut(api, join(base, "mydir"), undefined, noopLogger(), cwd);
    expect(received.has("a.txt")).toBe(true);
  });

  it("throws when folder upload has no remote path and no cwd", async () => {
    const base = join(tmpdir(), `sn-put-folder-nocwd-${Date.now()}`);
    mkdirSync(join(base, "mydir"), { recursive: true });
    writeFileSync(join(base, "mydir", "a.txt"), "a");

    await expect(runPut(api, join(base, "mydir"), undefined)).rejects.toThrow(
      "no current folder set. Use 'sn cd",
    );
  });

  it("writes progress to stderr during single-file upload when progress=true", async () => {
    const dir = join(tmpdir(), `sn-put-progress-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");

    const stderrLines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      stderrLines.push(String(msg));
      return true;
    });

    await runPut(api, localPath, address, noopLogger(), undefined, true);
    vi.restoreAllMocks();

    const output = stderrLines.join("");
    expect(output).toContain("hello.txt");
  });

  it("does not write progress when progress=false", async () => {
    const dir = join(tmpdir(), `sn-put-noprogress-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");

    const stderrSpy = vi.spyOn(process.stderr, "write");
    await runPut(api, localPath, address, noopLogger(), undefined, false);
    vi.restoreAllMocks();

    const progressCalls = stderrSpy.mock.calls.filter((c) =>
      String(c[0]).includes("hello.txt"),
    );
    expect(progressCalls.length).toBe(0);
  });

  it("writes folder progress to stderr during folder upload when progress=true", async () => {
    const base = join(tmpdir(), `sn-put-folder-progress-${Date.now()}`);
    mkdirSync(join(base, "mydir"), { recursive: true });
    writeFileSync(join(base, "mydir", "a.txt"), "hello");

    const addrA = "omniverse://nucleus/Library/mydir/a.txt";
    server.use(
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(addrA)}`,
        async ({ request }) => {
          await request.arrayBuffer();
          return new HttpResponse(null, { status: 201 });
        },
      ),
    );

    const stderrLines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      stderrLines.push(String(msg));
      return true;
    });

    await runPut(
      api,
      join(base, "mydir"),
      "omniverse://nucleus/Library/",
      noopLogger(),
      undefined,
      true,
    );
    vi.restoreAllMocks();

    const output = stderrLines.join("");
    expect(output).toContain("a.txt");
    expect(output).toContain("1/1 files");
  });
});

describe("runPutLines", () => {
  const api = create("https://storage.example.com");
  const cwd = "https://mybucket.blob.core.windows.net/mybucket/Library/";

  it("uploads each piped local path to cwd/<basename>", async () => {
    const dir = join(tmpdir(), `sn-putlines-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await runPutLines(api, [localPath], noopLogger(), cwd);
    expect(received).toBe(true);
  });

  it("collects errors and rejects with failure count", async () => {
    const dir = join(tmpdir(), `sn-putlines-err-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const pathA = join(dir, "a.txt");
    const pathB = join(dir, "b.txt");
    writeFileSync(pathA, "a");
    writeFileSync(pathB, "b");

    server.use(
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(`${cwd}a.txt`)}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
      http.put(
        `https://storage.example.com/v1alpha/fileobject/by-address/${encodeURIComponent(`${cwd}b.txt`)}`,
        () => new HttpResponse(null, { status: 500 }),
      ),
    );

    await expect(
      runPutLines(api, [pathA, pathB], noopLogger(), cwd),
    ).rejects.toThrow("2 error(s)");
  });

  it("throws if no cwd is set", async () => {
    const dir = join(tmpdir(), `sn-putlines-nocwd-${Date.now()}`);
    mkdirSync(dir, { recursive: true });
    const localPath = join(dir, "hello.txt");
    writeFileSync(localPath, "hello world");
    await expect(
      runPutLines(api, [localPath], noopLogger(), undefined),
    ).rejects.toThrow("no current folder set");
  });
});
