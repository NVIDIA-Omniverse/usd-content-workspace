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
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  renameSync,
  unlinkSync,
} from "fs";
import { randomUUID, createHash } from "crypto";
import { lock } from "proper-lockfile";
import { homedir } from "os";
import { join } from "path";
import { z } from "zod";

const CONFIG_DIR = () => join(homedir(), ".config", "sn");
const CONFIG_FILE = () => join(CONFIG_DIR(), "config.json");

const HostConfigSchema = z.object({
  discoveryUrl: z.string(),
  storageUrl: z.string(),
  authUrl: z.string(),
  accessToken: z.string().optional(),
  refreshToken: z.string().optional(),
  tokenExpiry: z.string().optional(),
  cwd: z.string().optional(),
});

const ConfigSchema = z.object({
  activeHost: z.string().optional(),
  hosts: z.record(z.string(), HostConfigSchema).default({}),
});

export type HostConfig = z.infer<typeof HostConfigSchema>;
export type Config = z.infer<typeof ConfigSchema>;

export function readConfig(): Config {
  const file = CONFIG_FILE();
  if (!existsSync(file)) {
    return { hosts: {} };
  }
  return ConfigSchema.parse(JSON.parse(readFileSync(file, "utf-8")));
}

export function writeConfig(config: Config): void {
  const dir = CONFIG_DIR();
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const temporary = join(dir, `.config-${randomUUID()}.tmp`);
  try {
    // A fresh inode also repairs permissions on existing, overly readable files.
    writeFileSync(temporary, JSON.stringify(config, null, 2), {
      mode: 0o600,
      flag: "wx",
    });
    chmodSync(temporary, 0o600);
    renameSync(temporary, CONFIG_FILE());
  } finally {
    if (existsSync(temporary)) unlinkSync(temporary);
  }
}

/**
 * Retry policy for the cross-process file lock. Polls at a short, flat,
 * jittered interval (25-50 ms) rather than exponential backoff: with backoff
 * capped at 1 s and no jitter, waiters converged on the same 1 s polling phase
 * and only about one of them acquired the lock per second. The retry budget
 * (~75-150 s) still outlasts proper-lockfile's 10 s stale-lock reclamation.
 */
const LOCK_RETRIES = {
  retries: 3000,
  factor: 1,
  minTimeout: 25,
  maxTimeout: 50,
  randomize: true,
};

/** Tail of the in-process FIFO queue for each lock path. */
const inProcessQueues = new Map<string, Promise<void>>();

/**
 * Run `action` after every earlier in-process caller for `target` has
 * finished, so callers in the same process queue in order instead of
 * polling the file lock against each other.
 */
async function serializeInProcess<T>(
  target: string,
  action: () => Promise<T>,
): Promise<T> {
  const previous = inProcessQueues.get(target) ?? Promise.resolve();
  let done!: () => void;
  const current = new Promise<void>((resolve) => (done = resolve));
  const tail = previous.then(() => current);
  inProcessQueues.set(target, tail);
  try {
    await previous;
    return await action();
  } finally {
    done();
    if (inProcessQueues.get(target) === tail) inProcessQueues.delete(target);
  }
}

async function withConfigLock<T>(
  target: string,
  action: () => T | Promise<T>,
): Promise<T> {
  return serializeInProcess(target, async () => {
    mkdirSync(CONFIG_DIR(), { recursive: true, mode: 0o700 });
    const release = await lock(target, {
      realpath: false,
      retries: LOCK_RETRIES,
    });
    try {
      return await action();
    } finally {
      await release();
    }
  });
}

/** Read the latest state under a cross-process lock, then atomically persist it. */
export async function updateConfig(
  update: (config: Config) => void,
): Promise<void> {
  await withConfigLock(CONFIG_FILE(), () => {
    const config = readConfig();
    update(config);
    writeConfig(config);
  });
}

/** Serialize rotating refresh tokens independently for each configured host. */
export function withHostAuthLock<T>(
  host: string,
  action: () => T | Promise<T>,
): Promise<T> {
  const key = createHash("sha256").update(host).digest("hex");
  return withConfigLock(join(CONFIG_DIR(), `auth-${key}`), action);
}

export function getActiveHost(config: Config): HostConfig {
  if (!config.activeHost) {
    throw new Error("no active host. Run 'sn host add'");
  }
  const host = config.hosts[config.activeHost];
  if (!host) {
    throw new Error(`host '${config.activeHost}' not found. Run 'sn host add'`);
  }
  return host;
}
