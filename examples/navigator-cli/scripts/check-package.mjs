// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const temporary = mkdtempSync(join(tmpdir(), "sn-package-"));
const npm = process.platform === "win32" ? "npm.cmd" : "npm";
try {
  // Exercise prepack as well as the built artifact, without publishing anything.
  execFileSync(npm, ["pack", "--pack-destination", temporary], {
    stdio: "inherit",
  });
  const archive = readdirSync(temporary).find((name) => name.endsWith(".tgz"));
  assert(archive, "npm pack did not create an archive");
  const tarball = join(temporary, archive);
  const entries = execFileSync("tar", ["-tf", tarball], { encoding: "utf8" })
    .trim()
    .split(/\r?\n/);
  const allowed = new Set([
    "package/package.json",
    "package/README.md",
    "package/LICENSE.txt",
    "package/.agents/skills/storage-navigator-cli/SKILL.md",
  ]);
  for (const entry of entries) {
    assert(
      allowed.has(entry) || /^package\/dist\/index\.(?:js|d\.ts)$/.test(entry),
      `Unexpected package content: ${entry}`,
    );
  }
  assert(entries.includes("package/dist/index.js"), "Missing CLI executable");
  assert(entries.includes("package/LICENSE.txt"), "Missing license");
  const install = join(temporary, "installation");
  const testHome = join(temporary, "home");
  mkdirSync(testHome);
  execFileSync(
    npm,
    [
      "install",
      "--prefix",
      install,
      "--omit=dev",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      tarball,
    ],
    { stdio: "inherit" },
  );
  const binary = join(
    install,
    "node_modules",
    "@storage-navigator",
    "cli",
    "dist",
    "index.js",
  );
  assert(
    existsSync(
      join(
        install,
        "node_modules",
        ".bin",
        process.platform === "win32" ? "sn.cmd" : "sn",
      ),
    ),
    "Missing installed sn command",
  );
  const options = {
    encoding: "utf8",
    env: { ...process.env, HOME: testHome, USERPROFILE: testHome },
  };
  assert.match(
    execFileSync(process.execPath, [binary, "--help"], options),
    /Storage Navigator CLI/,
  );
  assert.deepEqual(
    JSON.parse(
      execFileSync(
        process.execPath,
        [binary, "--json", "host", "list"],
        options,
      ),
    ),
    [],
  );
  console.log("Package contents and clean installation passed.");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
