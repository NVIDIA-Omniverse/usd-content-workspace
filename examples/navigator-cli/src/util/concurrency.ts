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

import { CliError } from "@/output";

const DEFAULT_CONCURRENCY = 16;
const CONCURRENCY_ENV = "SN_CONCURRENCY";

export function getConcurrencyLimit(): number {
  const raw = process.env[CONCURRENCY_ENV];
  if (!raw || !/^\d+$/.test(raw)) return DEFAULT_CONCURRENCY;
  const parsed = Number(raw);
  return Number.isSafeInteger(parsed) && parsed >= 1
    ? parsed
    : DEFAULT_CONCURRENCY;
}

function describeItem(item: unknown): string {
  if (typeof item === "string" || typeof item === "number") return String(item);
  return JSON.stringify(item);
}

/** Stop scheduling on failure and drain every active operation before rejecting. */
export async function forEachWithConcurrency<T>(
  items: readonly T[],
  limit: number,
  worker: (item: T, index: number) => Promise<void>,
): Promise<void> {
  if (!items.length) return;
  const effectiveLimit = Number.isFinite(limit)
    ? Math.max(1, Math.min(Math.floor(limit), items.length))
    : 1;
  let nextIndex = 0;
  const completed: string[] = [];
  const failures: { item: string; message: string; details?: unknown }[] = [];
  const runWorker = async () => {
    while (failures.length === 0 && nextIndex < items.length) {
      const index = nextIndex++;
      try {
        await worker(items[index], index);
        completed.push(describeItem(items[index]));
      } catch (error) {
        failures.push({
          item: describeItem(items[index]),
          message: error instanceof Error ? error.message : String(error),
          details: error instanceof CliError ? error.details : undefined,
        });
      }
    }
  };
  await Promise.all(Array.from({ length: effectiveLimit }, runWorker));
  if (failures.length) {
    throw new CliError(
      `${failures.length} operation(s) failed after ${completed.length} completed: ${failures.map(({ item, message }) => `${item}: ${message}`).join("; ")}`,
      "PARTIAL_FAILURE",
      {
        completed,
        failures,
        notStarted: items.slice(nextIndex).map(describeItem),
      },
    );
  }
}
