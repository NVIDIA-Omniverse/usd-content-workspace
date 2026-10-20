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

interface WaitingEntry {
  resolve: () => void;
  reject: (reason: unknown) => void;
}

export class Semaphore {
  private active = 0;
  private readonly waiting: WaitingEntry[] = [];

  constructor(private readonly limit: number) {}

  async run<T>(fn: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    if (signal?.aborted) {
      throw signal.reason ?? new DOMException("Aborted", "AbortError");
    }
    if (this.active >= this.limit) {
      await new Promise<void>((resolve, reject) => {
        const entry: WaitingEntry = { resolve, reject };
        this.waiting.push(entry);
        signal?.addEventListener(
          "abort",
          () => {
            const idx = this.waiting.indexOf(entry);
            if (idx >= 0) {
              this.waiting.splice(idx, 1);
              reject(signal.reason instanceof Error ? signal.reason : new DOMException("Aborted", "AbortError"));
            }
          },
          { once: true }
        );
      });
    }
    if (signal?.aborted) {
      this.waiting.shift()?.resolve();
      throw signal.reason ?? new DOMException("Aborted", "AbortError");
    }
    this.active++;
    try {
      return await fn();
    } finally {
      this.active--;
      this.waiting.shift()?.resolve();
    }
  }
}
