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

import { Readable } from "stream";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isStdinPiped, readStdinLines } from "./stdin";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("isStdinPiped", () => {
  it("returns false when stdin is a TTY", () => {
    Object.defineProperty(process.stdin, "isTTY", {
      value: true,
      configurable: true,
    });
    expect(isStdinPiped()).toBe(false);
  });

  it("returns true when stdin.isTTY is undefined (piped)", () => {
    Object.defineProperty(process.stdin, "isTTY", {
      value: undefined,
      configurable: true,
    });
    expect(isStdinPiped()).toBe(true);
  });

  it("returns true when stdin.isTTY is false", () => {
    Object.defineProperty(process.stdin, "isTTY", {
      value: false,
      configurable: true,
    });
    expect(isStdinPiped()).toBe(true);
  });
});

describe("readStdinLines", () => {
  function makeStdin(content: string): Readable {
    // eslint-disable-next-line @typescript-eslint/no-empty-function
    const r = new Readable({ read() {} });
    r.push(content);
    r.push(null);
    return r;
  }

  it("returns trimmed non-empty lines", async () => {
    const lines = await readStdinLines(
      makeStdin("  alpha  \nbeta\n  gamma  \n"),
    );
    expect(lines).toEqual(["alpha", "beta", "gamma"]);
  });

  it("filters blank lines", async () => {
    const lines = await readStdinLines(makeStdin("alpha\n\n\nbeta\n"));
    expect(lines).toEqual(["alpha", "beta"]);
  });

  it("strips ANSI escape codes", async () => {
    const lines = await readStdinLines(
      makeStdin("\x1b[34mblue/\x1b[0m\nplain.txt\n"),
    );
    expect(lines).toEqual(["blue/", "plain.txt"]);
  });

  it("strips OSC hyperlink sequences", async () => {
    const lines = await readStdinLines(
      makeStdin("\x1b]8;;https://example.com\x07linked.txt\x1b]8;;\x07\n"),
    );
    expect(lines).toEqual(["linked.txt"]);
  });
});
