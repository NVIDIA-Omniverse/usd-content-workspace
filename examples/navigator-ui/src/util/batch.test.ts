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

import { beforeEach, describe, expect, it, vi } from "vitest";
import { Batch } from "./batch";

interface TestItem {
  key: string;
  value: string;
}

describe("Batch", () => {
  const mockHandler = vi.fn();
  const mockOnItemSuccess = vi.fn();
  const mockOnItemError = vi.fn();
  const mockOnBatchComplete = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should process items in batches", async () => {
    const items: TestItem[] = [
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
    ];

    mockHandler.mockImplementation((item: TestItem) => `processed-${item.value}`);

    const batchProcessor = new Batch(mockHandler, {
      batchSize: 2,
      onItemSuccess: mockOnItemSuccess,
    });

    await batchProcessor.add(items);
    await batchProcessor.flush();

    expect(mockHandler).toHaveBeenCalledTimes(3);
    expect(mockOnItemSuccess).toHaveBeenCalledTimes(3);
    expect(mockOnItemSuccess).toHaveBeenCalledWith({ key: "1", value: "a" }, "processed-a");
  });

  it("should handle errors gracefully", async () => {
    const items = [
      { key: "1", value: "a" },
      { key: "2", value: "b" },
    ];

    mockHandler
      .mockImplementationOnce(() => "success")
      .mockImplementationOnce(() => {
        throw new Error("test error");
      });

    const batchProcessor = new Batch(mockHandler, {
      onItemSuccess: mockOnItemSuccess,
      onItemError: mockOnItemError,
    });

    await batchProcessor.add(items);
    await batchProcessor.flush();

    expect(mockOnItemSuccess).toHaveBeenCalledTimes(1);
    expect(mockOnItemError).toHaveBeenCalledTimes(1);
    expect(mockOnItemError).toHaveBeenCalledWith({ key: "2", value: "b" }, expect.any(Error));
  });

  it("should respect batch size", async () => {
    const items = [
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
    ];

    mockHandler.mockImplementation((item: TestItem) => `processed-${item.value}`);

    const batchProcessor = new Batch(mockHandler, {
      batchSize: 2,
    });

    await batchProcessor.add(items);
    await batchProcessor.flush();

    expect(mockHandler).toHaveBeenCalledTimes(3);
  });

  it("should handle abort signal", async () => {
    const controller = new AbortController();
    const items = [
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
    ];

    mockHandler.mockImplementation((item: TestItem) => {
      if (item.key === "2") {
        controller.abort();
      }
      return `processed-${item.value}`;
    });

    const batchProcessor = new Batch(mockHandler, {
      signal: controller.signal,
    });

    await batchProcessor.add(items);
    await batchProcessor.flush();

    // Should only process items up to the abort
    expect(mockHandler).toHaveBeenCalledTimes(2);
  });

  it("should call onBatchComplete after each batch", async () => {
    const items = [
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
    ];

    mockHandler.mockImplementation((item: TestItem) => `processed-${item.value}`);

    const batchProcessor = new Batch(mockHandler, {
      batchSize: 2,
      onBatchComplete: mockOnBatchComplete,
    });

    await batchProcessor.add(items);
    await batchProcessor.flush();

    // Should be called twice - once for the first batch of 2 items and once for the remaining item
    expect(mockOnBatchComplete).toHaveBeenCalledTimes(2);
  });

  it("should run items within a single batch in parallel", async () => {
    const batchSize = 2;
    const order: string[] = [];
    let resolveFirst: () => void;
    const firstDone = new Promise<void>((r) => {
      resolveFirst = r;
    });
    let resolveSecond: () => void;
    const secondDone = new Promise<void>((r) => {
      resolveSecond = r;
    });

    mockHandler
      .mockImplementationOnce(async (item: TestItem) => {
        order.push(`start-${item.key}`);
        await firstDone;
        order.push(`end-${item.key}`);
        return "ok";
      })
      .mockImplementationOnce(async (item: TestItem) => {
        order.push(`start-${item.key}`);
        await secondDone;
        order.push(`end-${item.key}`);
        return "ok";
      });

    const batchProcessor = new Batch(mockHandler, { batchSize });
    await batchProcessor.add([
      { key: "1", value: "a" },
      { key: "2", value: "b" },
    ]);

    const flushPromise = batchProcessor.flush();

    // Allow both handlers to start (they run in parallel)
    await vi.waitFor(() => {
      expect(order).toContain("start-1");
      expect(order).toContain("start-2");
    });

    // Both started before either finished - so they ran in parallel
    expect(order).not.toContain("end-1");
    expect(order).not.toContain("end-2");

    resolveFirst!();
    resolveSecond!();
    await flushPromise;

    expect(order).toHaveLength(4);
    expect(order.slice(0, 2).sort()).toEqual(["start-1", "start-2"]);
    expect(order.slice(2, 4).sort()).toEqual(["end-1", "end-2"]);
  });

  it("should run next batch only after current batch has completed", async () => {
    const order: string[] = [];
    let resolveBatch1: () => void;
    const batch1Done = new Promise<void>((r) => {
      resolveBatch1 = r;
    });

    mockHandler
      .mockImplementationOnce(async (item: TestItem) => {
        order.push(`batch1-start-${item.key}`);
        await batch1Done;
        order.push(`batch1-end-${item.key}`);
        return "ok";
      })
      .mockImplementationOnce(async (item: TestItem) => {
        order.push(`batch1-start-${item.key}`);
        await batch1Done;
        order.push(`batch1-end-${item.key}`);
        return "ok";
      })
      .mockImplementation((item: TestItem) => {
        order.push(`batch2-${item.key}`);
        return Promise.resolve("ok");
      });

    const batchProcessor = new Batch(mockHandler, {
      batchSize: 2,
      onBatchComplete: () => {
        order.push("onBatchComplete");
      },
    });

    await batchProcessor.add([
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
      { key: "4", value: "d" },
    ]);

    const flushPromise = batchProcessor.flush();

    await vi.waitFor(() => {
      expect(order.filter((x) => x.startsWith("batch1-start"))).toHaveLength(2);
    });

    // Batch 2 must not have started yet
    expect(order.some((x) => x.startsWith("batch2-"))).toBe(false);
    expect(order).not.toContain("onBatchComplete");

    resolveBatch1!();
    await flushPromise;

    // Batch 1 ran (both in parallel), then onBatchComplete, then batch 2
    expect(order).toContain("onBatchComplete");
    const batch1EndIndex = Math.max(order.indexOf("batch1-end-1"), order.indexOf("batch1-end-2"));
    expect(order.indexOf("onBatchComplete")).toBeGreaterThan(batch1EndIndex);
    expect(order.some((x) => x === "batch2-3")).toBe(true);
    expect(order.some((x) => x === "batch2-4")).toBe(true);
  });

  it("flush() should resolve only after all batches have completed", async () => {
    const completed: string[] = [];
    let resolveBatch2: () => void;
    const batch2Start = new Promise<void>((r) => {
      resolveBatch2 = r;
    });

    mockHandler.mockImplementation((item: TestItem) => {
      if (item.key === "1" || item.key === "2") {
        completed.push(item.key);
        return Promise.resolve("ok");
      }
      return batch2Start.then(() => {
        completed.push(item.key);
        return "ok";
      });
    });

    const batchProcessor = new Batch(mockHandler, { batchSize: 2 });
    await batchProcessor.add([
      { key: "1", value: "a" },
      { key: "2", value: "b" },
      { key: "3", value: "c" },
      { key: "4", value: "d" },
    ]);

    const flushPromise = batchProcessor.flush();

    await vi.waitFor(() => {
      expect(completed).toContain("1");
      expect(completed).toContain("2");
    });

    // Batch 2 is still pending; flush should not have resolved until we release it
    expect(completed).not.toContain("3");

    resolveBatch2!();
    await flushPromise;

    expect(completed).toEqual(["1", "2", "3", "4"]);
  });

  it("should handle multiple concurrent add calls that trigger flush", async () => {
    const item1 = [{ key: "1", value: "a" }];
    const item2 = [{ key: "2", value: "b" }];

    let resolveHandler: (value: string) => void;
    const handlerPromise = new Promise<string>((resolve) => {
      resolveHandler = resolve;
    });

    mockHandler.mockImplementation(() => handlerPromise);

    const batchProcessor = new Batch(mockHandler, {
      batchSize: 2,
    });

    await batchProcessor.add(item1);
    const flush1Promise = batchProcessor.flush();

    // Add second item while flush is still running
    const add2Promise = batchProcessor.add(item2);

    // The add operation should complete (won't trigger flush due to isFlushing flag)
    await add2Promise;

    expect(mockHandler).toHaveBeenCalledTimes(1);

    resolveHandler!("processed");
    await flush1Promise;
    await batchProcessor.flush();

    expect(mockHandler).toHaveBeenCalledTimes(2);
  });
});
