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

// Use this for a plain, uncancellable pause (e.g. throttling between polls).
export async function sleep(time: number): Promise<void> {
  return new Promise<void>((resolve) => setTimeout(resolve, time));
}

// AbortSignal#reason is `unknown` and may not be an Error; normalize it so callers always reject
// with something Error-like.
export function abortError(signal: AbortSignal): Error {
  const reason: unknown = signal.reason;
  return reason instanceof Error || reason instanceof DOMException ? reason : new DOMException("Aborted", "AbortError");
}

// Mirrors ky's default backoff curve (300ms base, doubling each retry) unless overridden. Use
// alongside `delay` to space out retries, e.g. `delay(exponentialBackoff(attempt), signal)`.
export function exponentialBackoff(attempt: number, options?: { baseMs?: number; factor?: number }): number {
  const { baseMs = 300, factor = 2 } = options ?? {};
  return baseMs * factor ** (attempt - 1);
}

// Use this instead of `sleep` when the wait sits inside a larger operation that can be cancelled
// (e.g. backoff between retries) -- it rejects immediately once `signal` aborts rather than waiting
// out the full duration.
export function delay(time: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(abortError(signal));
      return;
    }
    const onAbort = () => {
      clearTimeout(timer);
      reject(abortError(signal));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, time);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}
