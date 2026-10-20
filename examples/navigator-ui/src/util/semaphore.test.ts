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

import { Semaphore } from "./semaphore";

describe("Semaphore", () => {
  test("limits concurrency to the specified limit", async () => {
    const semaphore = new Semaphore(3);
    let active = 0;
    let maxActive = 0;

    const task = () =>
      semaphore.run(async () => {
        active++;
        maxActive = Math.max(maxActive, active);
        await new Promise((resolve) => setTimeout(resolve, 10));
        active--;
      });

    await Promise.all(Array.from({ length: 20 }, () => task()));

    expect(maxActive).toBe(3);
  });

  test("returns the result from the wrapped function", async () => {
    const semaphore = new Semaphore(2);
    const result = await semaphore.run(() => Promise.resolve(42));
    expect(result).toBe(42);
  });

  test("propagates errors from the wrapped function", async () => {
    const semaphore = new Semaphore(2);
    await expect(semaphore.run(() => Promise.reject(new Error("test error")))).rejects.toThrow("test error");
  });

  test("releases the slot when the wrapped function throws", async () => {
    const semaphore = new Semaphore(1);

    await semaphore.run(() => Promise.resolve("ok"));
    await expect(semaphore.run(() => Promise.reject(new Error("fail")))).rejects.toThrow("fail");

    const result = await semaphore.run(() => Promise.resolve("recovered"));
    expect(result).toBe("recovered");
  });

  test("throws immediately when signal is already aborted", async () => {
    const semaphore = new Semaphore(2);
    const controller = new AbortController();
    controller.abort();

    await expect(semaphore.run(() => Promise.resolve("ok"), controller.signal)).rejects.toMatchObject({
      name: "AbortError",
    });
  });

  test("removes queued entry and rejects when signal is aborted while waiting", async () => {
    const semaphore = new Semaphore(1);
    const controller = new AbortController();

    // Fill the single slot
    let releaseSlot!: () => void;
    const slotHolder = semaphore.run(() => new Promise<void>((resolve) => (releaseSlot = resolve)));

    // Queue a second task — it will wait
    const queued = semaphore.run(() => Promise.resolve("queued"), controller.signal);

    // Abort before the slot is released
    controller.abort();
    await expect(queued).rejects.toMatchObject({ name: "AbortError" });

    // Slot should still be usable after the aborted entry was removed
    releaseSlot();
    await slotHolder;
    await expect(semaphore.run(() => Promise.resolve("after"))).resolves.toBe("after");
  });

  test("rejects without running fn when signal aborts after dequeue but before execution", async () => {
    const semaphore = new Semaphore(1);
    const controller = new AbortController();

    // Fill the single slot with a task we control
    let releaseSlot!: () => void;
    const slotHolder = semaphore.run(() => new Promise<void>((resolve) => (releaseSlot = resolve)));

    let fnCalled = false;
    const queued = semaphore.run(() => {
      fnCalled = true;
      return Promise.resolve("queued");
    }, controller.signal);

    releaseSlot();
    await Promise.resolve();

    expect(fnCalled).toBe(false);
    controller.abort();

    await slotHolder;
    await expect(queued).rejects.toMatchObject({ name: "AbortError" });
    expect(fnCalled).toBe(false);

    // The slot should have been passed along — semaphore is usable immediately.
    await expect(semaphore.run(() => Promise.resolve("next"))).resolves.toBe("next");
  });
});
