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

/**
 * Configuration options for batch processing
 * @template T The type of items to be processed
 * @template R The type of result returned by the handler
 * @template E The type of error that may be thrown
 */
export interface BatchOptions<T, R, E> {
  /** Maximum number of items to process in parallel */
  batchSize?: number;
  /** Signal to abort processing */
  signal?: AbortSignal;
  /** Callback when a single batch has completed */
  onBatchComplete?: () => void | Promise<void>;
  /** Callback for successful item processing */
  onItemSuccess?: (item: T, result: R) => void | Promise<void>;
  /** Callback for failed item processing */
  onItemError?: (item: T, error: E) => void | Promise<void>;
}

/**
 * A class that handles items asynchronously in configurable batch sizes.
 * This utility is useful for processing large sets of items while controlling memory usage
 * and providing progress feedback.
 *
 * @template T The type of items to be processed, must contain a property matching the key option
 * @template R The type of result returned by the handler
 * @template E The type of error that may be thrown
 */
export class Batch<T extends Record<string, any>, R = void, E = unknown> {
  private items: T[] = [];
  private processing: Set<T>;
  private flushPromise: Promise<void> | null = null;
  private readonly lockName = `batch-flush-${crypto.randomUUID()}`;
  private readonly handler: (item: T, signal?: AbortSignal) => Promise<R>;
  private readonly options: Required<BatchOptions<T, R, E>>;

  constructor(handler: (item: T, signal?: AbortSignal) => Promise<R>, options: BatchOptions<T, R, E> = {}) {
    this.handler = handler;
    this.processing = new Set<T>();
    this.options = {
      batchSize: DEFAULT_BATCH_SIZE,
      ...options,
    } as Required<BatchOptions<T, R, E>>;
  }

  private async onItemSuccess(item: T, result?: R): Promise<void> {
    try {
      await this.options.onItemSuccess?.(item, result as R);
    } catch (error) {
      // Intentionally suppress errors from success callback
    }
  }

  private async onItemError(item: T, error: E): Promise<void> {
    try {
      await this.options.onItemError?.(item, error);
    } catch (error) {
      // Intentionally suppress errors from error callback
    }
  }

  private async onBatchComplete(): Promise<void> {
    try {
      await this.options.onBatchComplete?.();
    } catch (error) {
      // Intentionally suppress errors from batch complete callback
    }
  }

  /**
   * Adds items to the processing queue and starts batch processing if not already in progress.
   */
  add(items: T[]): Promise<void> {
    this.items.push(...items);
    // start batch processing if not already started
    void this.flush();
    return Promise.resolve();
  }

  /**
   * Processes items in batches until the queue is empty.
   * `flushPromise` deduplicates concurrent callers so they share a single promise,
   * while the Web Lock in `flushInternal` serializes back-to-back flushes that overlap
   * (e.g. a new flush starts between `flushPromise = null` and the lock releasing).
   */
  async flush(): Promise<void> {
    if (this.flushPromise) {
      return this.flushPromise;
    }
    this.flushPromise = this.flushInternal();
    return this.flushPromise;
  }

  private async flushInternal(): Promise<void> {
    await navigator.locks.request(this.lockName, async () => {
      try {
        while (!this.options.signal?.aborted && this.items.length > 0) {
          const batchItems = this.items.splice(0, this.options.batchSize);
          batchItems.forEach((item) => this.processing.add(item));

          await Promise.all(
            batchItems.map(async (item) => {
              if (this.options.signal?.aborted) {
                return;
              }

              try {
                const result = await this.handler(item, this.options.signal);
                await this.onItemSuccess(item, result);
              } catch (error) {
                await this.onItemError(item, error as E);
              } finally {
                this.processing.delete(item);
              }
            })
          );
          await this.onBatchComplete();
        }
      } finally {
        this.flushPromise = null;
      }
    });
  }
}

const DEFAULT_BATCH_SIZE = 25;
