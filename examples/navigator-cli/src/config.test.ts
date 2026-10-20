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
  chmodSync,
  mkdtempSync,
  mkdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getActiveHost, readConfig, writeConfig, updateConfig } from "./config";

vi.mock("os", async (importOriginal) => {
  const orig = await importOriginal<typeof import("os")>();
  return { ...orig, homedir: () => testHome };
});

let testHome: string;

beforeEach(() => {
  testHome = mkdtempSync(join(tmpdir(), "sn-config-test-"));
});

afterEach(() => {
  vi.restoreAllMocks();
  rmSync(testHome, { recursive: true, force: true });
});

describe("readConfig", () => {
  it("returns empty config when file does not exist", () => {
    const config = readConfig();
    expect(config).toEqual({ hosts: {} });
  });

  it("reads existing config", () => {
    const configDir = join(testHome, ".config", "sn");
    mkdirSync(configDir, { recursive: true });
    writeFileSync(
      join(configDir, "config.json"),
      JSON.stringify({
        activeHost: "prod",
        hosts: {
          prod: {
            discoveryUrl: "https://d",
            storageUrl: "https://s",
            authUrl: "https://a",
          },
        },
      }),
    );
    const config = readConfig();
    expect(config.activeHost).toBe("prod");
  });
});

describe("writeConfig", () => {
  it("repairs existing file permissions on every write", () => {
    writeConfig({ hosts: {} });
    const file = join(testHome, ".config", "sn", "config.json");
    chmodSync(file, 0o644);
    writeConfig({ activeHost: "new", hosts: {} });
    expect(statSync(file).mode & 0o777).toBe(0o600);
    expect(readConfig().activeHost).toBe("new");
  });

  it("merges concurrent updates without losing host entries", async () => {
    await Promise.all(
      Array.from({ length: 10 }, (_, index) =>
        updateConfig((config) => {
          config.hosts[`host-${index}`] = {
            discoveryUrl: "https://d",
            storageUrl: "https://s",
            authUrl: "",
          };
        }),
      ),
    );
    expect(Object.keys(readConfig().hosts)).toHaveLength(10);
  });

  it("keeps serving queued updates after one update throws", async () => {
    const results = await Promise.allSettled(
      Array.from({ length: 3 }, (_, index) =>
        updateConfig((config) => {
          if (index === 0) throw new Error("boom");
          config.hosts[`host-${index}`] = {
            discoveryUrl: "https://d",
            storageUrl: "https://s",
            authUrl: "",
          };
        }),
      ),
    );
    expect(results.map((result) => result.status)).toEqual([
      "rejected",
      "fulfilled",
      "fulfilled",
    ]);
    expect(Object.keys(readConfig().hosts).sort()).toEqual([
      "host-1",
      "host-2",
    ]);
  });

  it("writes config and creates directory", () => {
    writeConfig({ activeHost: "dev", hosts: {} });
    const config = readConfig();
    expect(config.activeHost).toBe("dev");
  });

  it("sets file permissions to 0x600", () => {
    writeConfig({ hosts: {} });
    const configDir = join(testHome, ".config", "sn");
    const stat = statSync(join(configDir, "config.json"));
    expect(stat.mode & 0o777).toBe(0o600);
  });
});

describe("getActiveHost", () => {
  it("throws if no active host set", () => {
    expect(() => getActiveHost({ hosts: {} })).toThrow("no active host");
  });

  it("throws if host not found", () => {
    expect(() => getActiveHost({ activeHost: "missing", hosts: {} })).toThrow(
      "host 'missing' not found",
    );
  });

  it("returns host config", () => {
    const host = {
      discoveryUrl: "https://d",
      storageUrl: "https://s",
      authUrl: "https://a",
    };
    expect(
      getActiveHost({ activeHost: "prod", hosts: { prod: host } }),
    ).toEqual(host);
  });
});
