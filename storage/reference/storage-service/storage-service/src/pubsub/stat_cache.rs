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

use storage_core::stat_cache::StatCache;
use storage_core_util::{cache::ApiVersion, warn_limited};
use tonic::async_trait;
use url::Url;

use crate::pubsub::types::{ConsumeRequest, ConsumeResults, Consumer, StorageEvent};

/// Spell `address` the way the stat cache keys its entries, for every API version.
///
/// The stat cache stores under a canonically re-encoded path, so invalidating with the address as
/// it arrived on the event misses any entry whose spelling differs only by percent-encoding (a
/// blob named `a!b` is keyed `a%21b`), leaving stale metadata served until the TTL expires. The
/// key is also partitioned per API version, so one address occupies one entry per version and an
/// event must drop all of them. This mirrors storage-core's crate-private `canonical_cache_key`
/// and `all_version_keys`, so the two spellings can drift apart silently. It is meant to be
/// replaced by a `StatCache` invalidation handle owned by storage-core, mirroring
/// `ListCache::invalidate_cache_entry_by_object_url`; delete this helper once that lands.
///
/// An address that cannot be canonicalized is partitioned verbatim: a best-effort miss is better
/// than skipping the invalidation entirely.
fn cache_keys(address: &str) -> Vec<String> {
    let canonical = canonical_address(address).unwrap_or_else(|| address.to_owned());
    ApiVersion::ALL
        .iter()
        .map(|version| format!("{}\u{1f}{canonical}", version.prefix()))
        .collect()
}

fn canonical_address(address: &str) -> Option<String> {
    let mut url = match Url::parse(address) {
        Ok(url) => url,
        Err(e) => {
            warn_limited!(%e, "storage event carried an unparseable resource address");
            return None;
        },
    };
    match storage_core_util::url_path::canonicalize_url_path(&mut url) {
        Ok(()) => Some(url.into()),
        Err(e) => {
            warn_limited!(%e, "storage event carried an uncanonicalizable resource address path");
            None
        },
    }
}

#[async_trait]
impl<S: Consumer> Consumer for StatCache<S> {
    async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
        for event in &events {
            let resource_address = match event {
                StorageEvent::ObjectCreated(data) => Some(&data.resource_address),
                StorageEvent::ObjectDeleted(data) => Some(&data.resource_address),
                // Stat cache holds per-object stats; directory events have no per-object signal
                // to invalidate against. Forward without touching the cache.
                StorageEvent::DirCreated(_) | StorageEvent::DirDeleted(_) => None,
            };
            if let Some(address) = resource_address {
                for key in cache_keys(address) {
                    self.cache.invalidate(&key).await;
                }
            }
        }

        self.inner.consume(events).await
    }
}

#[cfg(test)]
mod tests {
    use super::cache_keys;

    /// A blob name spelled either way must resolve to the keys the stat cache stores under,
    /// otherwise a storage event invalidates nothing and stale metadata is served until the TTL.
    #[test]
    fn spellings_of_the_same_object_share_one_invalidation_key() {
        let raw = cache_keys("https://account.blob.core.windows.net/container/a!b");
        let encoded = cache_keys("https://account.blob.core.windows.net/container/a%21b");

        assert_eq!(raw, encoded);
        assert_eq!(
            raw,
            [
                "v1alpha\u{1f}https://account.blob.core.windows.net/container/a%21b",
                "v1beta\u{1f}https://account.blob.core.windows.net/container/a%21b",
            ]
        );
    }

    /// The cache keys one address per API version, so an event has to drop every partition or the
    /// versions that were not invalidated keep serving the stale stat.
    #[test]
    fn every_api_version_partition_is_invalidated() {
        let keys = cache_keys("https://account.blob.core.windows.net/container/blob");

        assert_eq!(keys.len(), storage_core_util::cache::ApiVersion::ALL.len());
    }

    #[test]
    fn uncanonicalizable_address_is_partitioned_verbatim() {
        // A dot segment is rejected rather than silently resolved, so the caller falls back to
        // invalidating the address as it arrived.
        let keys = cache_keys("https://account.blob.core.windows.net/container/..%2Fkey");

        assert_eq!(
            keys,
            [
                "v1alpha\u{1f}https://account.blob.core.windows.net/container/..%2Fkey",
                "v1beta\u{1f}https://account.blob.core.windows.net/container/..%2Fkey",
            ]
        );
    }
}
