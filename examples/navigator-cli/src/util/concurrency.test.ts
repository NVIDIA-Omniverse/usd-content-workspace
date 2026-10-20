// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, expect, it, vi } from "vitest";
import { forEachWithConcurrency, getConcurrencyLimit } from "./concurrency";

afterEach(() => vi.unstubAllEnvs());

it("stops queued work and waits for active workers after a failure", async () => {
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const started: string[] = [];
  let settled = false;
  const result = forEachWithConcurrency(
    ["failed", "active", "queued-1", "queued-2"],
    2,
    async (item) => {
      started.push(item);
      if (item === "failed") throw new Error("transfer failed");
      await gate;
    },
  ).catch((error: unknown) => {
    settled = true;
    return error;
  });
  await Promise.resolve();
  await Promise.resolve();
  expect(settled).toBe(false);
  expect(started).toEqual(["failed", "active"]);
  finish();
  expect(await result).toMatchObject({
    code: "PARTIAL_FAILURE",
    details: {
      completed: ["active"],
      failures: [{ item: "failed", message: "transfer failed" }],
      notStarted: ["queued-1", "queued-2"],
    },
  });
});

it("drains and reports every active failure", async () => {
  await expect(
    forEachWithConcurrency(["one", "two", "three"], 2, async (item) => {
      await Promise.resolve();
      throw new Error(item);
    }),
  ).rejects.toMatchObject({
    details: {
      completed: [],
      failures: [{ item: "one" }, { item: "two" }],
      notStarted: ["three"],
    },
  });
});

it.each(["0", "-1", "2junk", "Infinity", "1.5"])(
  "ignores invalid SN_CONCURRENCY value %s",
  (value) => {
    vi.stubEnv("SN_CONCURRENCY", value);
    expect(getConcurrencyLimit()).toBe(16);
  },
);
