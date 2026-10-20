// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moka::Expiry;

#[derive(Debug, Clone)]
pub(crate) struct CacheEntry<V> {
    pub value: V,
    pub ttl: Option<Duration>,
    /// Local invalidation generation that must still be current when moka
    /// publishes this entry. `None` is reserved for deliberately transient
    /// entries and values that are not racing an asynchronous publication.
    pub expected_local_generation: Option<u64>,
}

pub(crate) struct PerEntryExpiry {
    pub default_ttl: Duration,
    pub local_generation: Arc<AtomicU64>,
}

impl PerEntryExpiry {
    fn ttl_for<V>(&self, value: &CacheEntry<V>) -> Duration {
        if value
            .expected_local_generation
            .is_some_and(|expected| self.local_generation.load(Ordering::Acquire) != expected)
        {
            Duration::ZERO
        } else {
            value.ttl.unwrap_or(self.default_ttl)
        }
    }
}

impl<V> Expiry<String, CacheEntry<V>> for PerEntryExpiry {
    fn expire_after_create(&self, _key: &String, value: &CacheEntry<V>, _created_at: Instant) -> Option<Duration> {
        Some(self.ttl_for(value))
    }

    fn expire_after_update(
        &self,
        _key: &String,
        value: &CacheEntry<V>,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        Some(self.ttl_for(value))
    }

    fn expire_after_read(
        &self,
        _key: &String,
        _value: &CacheEntry<V>,
        _read_at: Instant,
        duration_until_expiry: Option<Duration>,
        _last_modified_at: Instant,
    ) -> Option<Duration> {
        duration_until_expiry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(expected_local_generation: Option<u64>) -> CacheEntry<()> {
        CacheEntry {
            value: (),
            ttl: Some(Duration::from_secs(30)),
            expected_local_generation,
        }
    }

    #[test]
    fn matching_generation_keeps_configured_ttl() {
        let local_generation = Arc::new(AtomicU64::new(7));
        let expiry = PerEntryExpiry {
            default_ttl: Duration::from_secs(60),
            local_generation,
        };

        assert_eq!(expiry.ttl_for(&entry(Some(7))), Duration::from_secs(30));
        assert_eq!(expiry.ttl_for(&entry(None)), Duration::from_secs(30));
    }

    #[test]
    fn stale_generation_forces_zero_ttl_on_create_and_update() {
        let local_generation = Arc::new(AtomicU64::new(8));
        let expiry = PerEntryExpiry {
            default_ttl: Duration::from_secs(60),
            local_generation,
        };
        let stale = entry(Some(7));
        let now = Instant::now();

        assert_eq!(
            expiry.expire_after_create(&String::new(), &stale, now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            expiry.expire_after_update(&String::new(), &stale, now, Some(Duration::from_secs(1))),
            Some(Duration::ZERO)
        );
    }
}
