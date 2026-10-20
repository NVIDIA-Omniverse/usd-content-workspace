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

import type { DiscoveryDocument } from "@/services/discoveryService";
import { Slice } from "@/state/slices/types";

/**
 * Per-store cache of resolved discovery documents keyed by discovery URL. Excluded from
 * persistence, so it lives for the lifetime of a running session and is dropped on reload.
 *
 * `resolveDiscoveryDocument` owns the read-modify-write of the cache: it returns the cached
 * promise when present, otherwise runs `fetch`, stores the pending promise so concurrent
 * lookups for the same deployment share a single request, and drops a rejected lookup so a
 * retry after authentication can succeed.
 */
export interface DiscoverySlice {
  discoveryCache: Map<string, Promise<DiscoveryDocument>>;
  resolveDiscoveryDocument: (key: string, fetch: () => Promise<DiscoveryDocument>) => Promise<DiscoveryDocument>;
  clearDiscoveryCache: () => void;
}

export const createDiscoverySlice: Slice<DiscoverySlice> = (set, get) => ({
  discoveryCache: new Map(),
  resolveDiscoveryDocument: (key, fetch) => {
    const cached = get().discoveryCache.get(key);
    if (cached) {
      return cached;
    }

    const pending = fetch().catch((error) => {
      set((state) => {
        state.discoveryCache.delete(key);
      });
      throw error;
    });
    set((state) => {
      state.discoveryCache.set(key, pending);
    });
    return pending;
  },
  clearDiscoveryCache: () =>
    set((state) => {
      state.discoveryCache.clear();
    }),
});
