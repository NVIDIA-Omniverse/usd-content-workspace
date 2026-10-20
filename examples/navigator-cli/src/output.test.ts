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

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CliError,
  createFileProgressCallback,
  createFolderProgressCallback,
  formatSize,
  noopLogger,
  printError,
  printJson,
  printLs,
  printLsLong,
  printSuccess,
  printPreview,
  printServices,
  setJsonMode,
  setCommandName,
  verboseLogger,
  type LsLongEntry,
} from "./output";

afterEach(() => {
  setJsonMode(false);
  vi.restoreAllMocks();
});

describe("printLs", () => {
  it("prints each name on its own line", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    printLs(["assets/", "scene.usd"]);
    expect(lines.join("")).toContain("assets/");
    expect(lines.join("")).toContain("scene.usd");
  });
});

describe("printLsLong", () => {
  it("prints permissions, size, date, and name", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const entries: LsLongEntry[] = [
      {
        name: "scene.usd",
        isFolder: false,
        permissions: "-rwx",
        size: 1024 * 1024,
        createdAt: new Date("2026-04-10T09:15:00Z"),
      },
    ];
    printLsLong(entries);
    const out = lines.join("");
    expect(out).toContain("scene.usd");
    expect(out).toContain("1.0 MB");
    expect(out).toContain("-rwx");
  });
});

describe("formatSize", () => {
  it("formats bytes", () => expect(formatSize(500)).toBe("500 B"));
  it("formats KB", () => expect(formatSize(2048)).toBe("2.0 KB"));
  it("formats MB", () => expect(formatSize(4 * 1024 * 1024)).toBe("4.0 MB"));
  it("formats GB", () =>
    expect(formatSize(2 * 1024 * 1024 * 1024)).toBe("2.0 GB"));
});

describe("printJson", () => {
  it("outputs pretty-printed JSON to stdout", () => {
    const lines: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    printJson({ key: "value" });
    const parsed = JSON.parse(lines.join(""));
    expect(parsed).toEqual({ key: "value" });
  });
});

describe("printError", () => {
  it("writes to stderr with error: prefix", () => {
    const msgs: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      msgs.push(String(msg));
      return true;
    });
    printError("not found");
    expect(msgs.join("")).toContain("error: not found");
  });

  it("writes one parseable error in JSON mode", () => {
    const messages: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((message) => {
      messages.push(String(message));
      return true;
    });
    setJsonMode(true);
    printError(new Error("failed"));
    expect(messages).toHaveLength(1);
    expect(JSON.parse(messages[0])).toEqual({
      ok: false,
      error: { code: "COMMAND_FAILED", message: "failed" },
    });
  });

  it("preserves a stable CLI error code", () => {
    const messages: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((message) => {
      messages.push(String(message));
      return true;
    });
    setJsonMode(true);
    printError(new CliError("already exists", "CONFLICT", { path: "a" }));
    const result = JSON.parse(messages[0]) as { error: { code: string } };
    expect(result.error.code).toBe("CONFLICT");
    expect(result).toMatchObject({ error: { path: "a" } });
  });
});

describe("printSuccess", () => {
  it("writes success message when json mode is disabled", () => {
    const msgs: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((msg) => {
      msgs.push(String(msg));
      return true;
    });

    setJsonMode(false);
    printSuccess("done");
    expect(msgs.join("")).toContain("done");
  });

  it("writes one structured success result when json mode is enabled", () => {
    const stdoutSpy = vi
      .spyOn(process.stdout, "write")
      .mockImplementation(() => true);

    setJsonMode(true);
    printSuccess("done");
    expect(stdoutSpy).toHaveBeenCalledOnce();
    expect(JSON.parse(String(stdoutSpy.mock.calls[0][0]))).toEqual({
      ok: true,
      command: "unknown",
    });
  });
});

it("writes one preview document in JSON mode", () => {
  const messages: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation((message) => {
    messages.push(String(message));
    return true;
  });
  setJsonMode(true);
  setCommandName("rm");
  printPreview([{ action: "delete", source: "omniverse://bucket/file" }]);
  expect(messages).toHaveLength(1);
  expect(JSON.parse(messages[0])).toEqual({
    ok: true,
    command: "rm",
    dryRun: true,
    operations: [{ action: "delete", source: "omniverse://bucket/file" }],
  });
  setCommandName("unknown");
});

describe("printServices", () => {
  it("prints each service with REST and gRPC endpoints", () => {
    const out: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((s) => {
      out.push(s as string);
      return true;
    });
    printServices([
      {
        id: "s1",
        name: "Storage",
        type: "storage",
        rest: "https://storage.example.com",
        grpc: "storage.example.com:443",
      },
    ]);
    expect(out).toContain("[storage] Storage\n");
    expect(out).toContain("  REST:  https://storage.example.com\n");
    expect(out).toContain("  gRPC:  storage.example.com:443\n");
  });

  it("shows (none) for missing endpoints", () => {
    const out: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((s) => {
      out.push(s as string);
      return true;
    });
    printServices([{ id: "s2", name: "Perms", type: "permission" }]);
    expect(out).toContain("  REST:  (none)\n");
    expect(out).toContain("  gRPC:  (none)\n");
  });

  it("prints No services found when empty", () => {
    const out: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation((s) => {
      out.push(s as string);
      return true;
    });
    printServices([]);
    expect(out).toContain("No services found.\n");
  });
});

describe("verboseLogger", () => {
  it("writes ISO-timestamp [label] message to stderr", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const log = verboseLogger();
    log("upload", "complete");
    expect(lines[0]).toMatch(
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.* \[upload\] complete\n$/,
    );
  });
});

describe("noopLogger", () => {
  it("does nothing", () => {
    const stderrSpy = vi.spyOn(process.stderr, "write");
    const log = noopLogger();
    log("upload", "complete");
    expect(stderrSpy).not.toHaveBeenCalled();
  });
});

describe("createFileProgressCallback", () => {
  it("writes verb, filename, percentage and speed to stderr mid-transfer", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cb = createFileProgressCallback("Uploading", "hello.txt");
    cb(512 * 1024, 1024 * 1024, 0);
    const output = lines.join("");
    expect(output).toContain("\rUploading hello.txt...");
    expect(output).toContain("50%");
  });

  it("writes newline on completion when total is known", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cb = createFileProgressCallback("Uploading", "hello.txt");
    cb(1024 * 1024, 1024 * 1024, 0);
    expect(lines).toContain("\n");
  });

  it("shows bytes without percentage when total is 0", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cb = createFileProgressCallback("Downloading", "file.bin");
    cb(512 * 1024, 0, 0);
    const output = lines.join("");
    expect(output).toContain("Downloading file.bin...");
    expect(output).not.toContain("%");
    expect(output).toContain("512.0 KB");
  });

  it("writes newline on finalization call (total equals transferred)", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cb = createFileProgressCallback("Downloading", "file.bin");
    cb(1024, 0, 0); // mid — total unknown
    cb(1024, 1024, 0); // finalization — total now equals transferred
    expect(lines).toContain("\n");
  });
});

describe("createFolderProgressCallback", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("writes filename, file count and speed on completion", () => {
    const lines: string[] = [];
    vi.spyOn(process.stderr, "write").mockImplementation((msg) => {
      lines.push(String(msg));
      return true;
    });
    const cb = createFolderProgressCallback(
      "Uploading",
      "sub/file.txt",
      3,
      12,
      () => 2.3 * 1024 * 1024,
    );
    cb(1024, 1024, 0);
    const output = lines.join("");
    expect(output).toContain("sub/file.txt");
    expect(output).toContain("3/12 files");
    expect(output).toContain("/s");
    expect(output).toContain("Uploading");
    expect(output).toContain("\r");
    expect(output).toContain("\x1b[K");
  });

  it("does nothing mid-transfer", () => {
    const stderrSpy = vi.spyOn(process.stderr, "write");
    const cb = createFolderProgressCallback(
      "Uploading",
      "file.txt",
      1,
      5,
      () => 1024 * 1024,
    );
    cb(512, 1024, 0);
    expect(stderrSpy).not.toHaveBeenCalled();
  });
});
