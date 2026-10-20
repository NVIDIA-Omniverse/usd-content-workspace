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

import chalk from "chalk";
import { format } from "date-fns";
import { type DiscoveryStackEntry } from "@/services/discoveryService";

export type Logger = (label: string, message: string) => void;

export class CliError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly details: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

let jsonMode = false;
let commandName = "unknown";
let suppressSuccess = 0;

export async function withBatchOutput<T>(task: () => Promise<T>): Promise<T> {
  suppressSuccess++;
  try {
    return await task();
  } finally {
    suppressSuccess--;
  }
}

export function setJsonMode(enabled: boolean): void {
  jsonMode = enabled;
}

export function setCommandName(name: string): void {
  commandName = name;
}

export function verboseLogger(): Logger {
  return (label, message) =>
    process.stderr.write(
      `${new Date().toISOString()} ${chalk.cyan(`[${label}]`)} ${message}\n`,
    );
}

export function noopLogger(): Logger {
  return () => {
    /* noop */
  };
}

export type ProgressCallback = (
  transferred: number,
  total: number,
  speed: number,
) => void;

/** The `speed` arg on the returned callback is unused; speed is recalculated from the factory's own start time. */
export function createFileProgressCallback(
  verb: string,
  filename: string,
): ProgressCallback {
  const startMs = Date.now();
  let done = false;
  return (transferred, total) => {
    if (done) return;
    const elapsed = Math.max((Date.now() - startMs) / 1000, 0.001);
    const speed = transferred / elapsed;
    const speedStr = `${formatSize(Math.round(speed))}/s`;
    if (total > 0) {
      const pct = Math.min(100, Math.round((transferred / total) * 100));
      process.stderr.write(
        `\r${verb} ${filename}... ${pct}% (${speedStr})\x1b[K`,
      );
      if (transferred >= total) {
        process.stderr.write("\n");
        done = true;
      }
    } else {
      process.stderr.write(
        `\r${verb} ${filename}... ${formatSize(transferred)} (${speedStr})\x1b[K`,
      );
    }
  };
}

export function createFolderProgressCallback(
  verb: string,
  filename: string,
  fileIndex: number,
  totalFiles: number,
  getAggregateSpeed: () => number,
): ProgressCallback {
  return (transferred, total) => {
    if (transferred < total) return;
    const speedStr = `${formatSize(Math.round(getAggregateSpeed()))}/s`;
    process.stderr.write(
      `\r${verb} ${filename} (${fileIndex}/${totalFiles} files) — ${speedStr}\x1b[K`,
    );
  };
}

export interface LsLongEntry {
  name: string;
  resourceAddress?: string;
  isFolder: boolean;
  permissions?: string;
  size?: number;
  createdAt?: Date;
}

export function printLs(names: string[]): void {
  for (const name of names) {
    process.stdout.write(`${name.endsWith("/") ? chalk.blue(name) : name}\n`);
  }
}

export function printLsLong(entries: LsLongEntry[]): void {
  for (const entry of entries) {
    const perms = (entry.permissions ?? "----").padEnd(4);
    const size = chalk.dim(
      entry.isFolder ? "".padStart(9) : formatSize(entry.size ?? 0).padStart(9),
    );
    const date = chalk.dim(
      entry.createdAt
        ? format(entry.createdAt, "MMM dd HH:mm").padStart(12)
        : "".padStart(12),
    );
    const name = entry.isFolder ? chalk.blue(entry.name) : entry.name;
    process.stdout.write(`${perms}  ${size}  ${date}  ${name}\n`);
  }
}

export function printJson(data: unknown): void {
  process.stdout.write(`${JSON.stringify(data, null, 2)}\n`);
}

export interface PreviewOperation {
  action: string;
  source: string;
  destination?: string;
}

export function printPreview(operations: PreviewOperation[]): void {
  if (jsonMode) {
    printJson({ ok: true, command: commandName, dryRun: true, operations });
    return;
  }
  for (const operation of operations) {
    process.stdout.write(
      `${operation.action} ${operation.source}${operation.destination ? ` → ${operation.destination}` : ""}\n`,
    );
  }
}

export function printSuccess(
  message: string,
  details: Record<string, unknown> = {},
): void {
  if (suppressSuccess > 0) return;
  if (jsonMode) {
    printJson({ ok: true, command: commandName, ...details });
    return;
  }
  process.stdout.write(`${chalk.green(message)}\n`);
}

export function printError(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  if (jsonMode) {
    const typed = error as { code?: unknown; response?: { status?: number } };
    const status = typed?.response?.status;
    const code =
      typeof typed?.code === "string"
        ? typed.code
        : status === 401
          ? "AUTH_REQUIRED"
          : status === 403
            ? "FORBIDDEN"
            : status === 404
              ? "NOT_FOUND"
              : status === 409 || status === 412
                ? "CONFLICT"
                : "COMMAND_FAILED";
    process.stderr.write(
      `${JSON.stringify({ ok: false, error: { code, message, ...(status ? { status } : {}), ...(error instanceof CliError ? error.details : {}) } })}\n`,
    );
    return;
  }
  process.stderr.write(`${chalk.red(`error: ${message}`)}\n`);
}

export function printServices(services: DiscoveryStackEntry[]): void {
  if (services.length === 0) {
    process.stdout.write("No services found.\n");
    return;
  }
  for (const svc of services) {
    process.stdout.write(`[${svc.type}] ${svc.name}\n`);
    process.stdout.write(`  REST:  ${svc.rest ?? "(none)"}\n`);
    process.stdout.write(`  gRPC:  ${svc.grpc ?? "(none)"}\n`);
    process.stdout.write("\n");
  }
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
