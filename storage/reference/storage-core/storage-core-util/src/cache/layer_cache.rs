// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Layer-facing cache that combines the new two-tier [`super::cache::Cache`]
//! (L1 moka + optional L2 Redis) with the layer-level concerns the legacy
//! `Cache<K, V>` provided: `cache.*` metrics and an error allowlist that lets
//! selected response codes (e.g., `NotFound`, `AccessDenied`) be cached as
//! values rather than re-executing the inner service on every repeat call.

use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bytesize::ByteSize;
use fred::clients::Pool;
use storage_core_types::error::{StorageError, StorageErrorExt};

use super::cache::{Cache as LowLevelCache, CacheBuilder, L2Metrics, L2OperationMetrics};
use super::error::Error as LowLevelError;
use super::pubsub::PubSubMetrics;
use super::secrets_generation::SecretsGeneration;
use super::serializable_error::SerializableCachedError;
use super::sized::{CachedSize, CachedValue};
use crate::observability::metrics::{self, BoundUIntCounter, BoundUIntGauge, MetricBinder};
use crate::{trace_limited, warn_limited};

/// Layer-facing config args extracted from each layer's figment section.
#[derive(Debug, Clone)]
pub struct CacheArgs {
    /// Byte capacity for the L1 tier. Used directly as the moka byte budget
    /// (the string key size plus the cache value type's [`CachedSize`] impl
    /// supplies per-entry weight).
    pub capacity: ByteSize,

    /// Default TTL for entries.
    pub time_to_live: Duration,

    /// Allowlist of response error codes to cache (legacy: `errors`).
    pub errors: Vec<String>,

    /// Whether write/copy/delete operations should invalidate cached entries
    /// for the affected keys.
    pub invalidate_on_update: bool,

    /// Percentage of `capacity` at which a cache-pressure warning is logged
    /// on insert. `0` means warn on every insert, `>= 100` disables the warn.
    pub capacity_warn_pct: u8,

    /// Optional override for the L2 tombstone TTL. Defaults to `time_to_live`.
    pub tombstone_ttl: Option<Duration>,

    /// Optional override for the L2 lease TTL. Defaults to
    /// `max(time_to_live / 2, 30s)`.
    pub lease_ttl: Option<Duration>,

    /// Whether this cache instance may use the optional Redis L2 tier.
    pub l2_enabled: bool,
}

pub fn default_capacity_warn_pct() -> u8 {
    95
}

/// Per-fill override for retaining errors as cache values.
///
/// The configured allowlist remains the default for existing callers. Layers
/// that use an internal sentinel error, or otherwise must guarantee that no
/// backend error survives beyond the current single-flight cohort, can select
/// [`CacheErrorPolicy::Never`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheErrorPolicy {
    /// Retain errors whose tonic status code is present in [`CacheArgs::errors`].
    Configured,
    /// Never retain an error, regardless of the configured allowlist.
    Never,
}

/// `CachedSize` for the wrapper that moka actually stores. `Result<V, E>`
/// matches whatever the value path produces; the error path uses the
/// `SerializableCachedError` retained-bytes estimate.
impl<V: CachedSize> CachedSize for Result<V, SerializableCachedError> {
    fn cached_size(&self) -> u32 {
        match self {
            Ok(v) => v.cached_size(),
            Err(e) => {
                let metadata: usize = e.metadata.iter().map(|(k, v)| k.len() + v.len()).sum();
                let raw = std::mem::size_of::<SerializableCachedError>()
                    + e.message.len()
                    + e.reason.len()
                    + e.domain.len()
                    + metadata;
                u32::try_from(raw).unwrap_or(u32::MAX)
            },
        }
    }
}

impl<V: CachedValue> CachedValue for Result<V, SerializableCachedError> {
    // Schema is governed by the inner V; allowlist-cached error layout is
    // owned by `SerializableCachedError` and bumped there if it ever changes.
    const SCHEMA: u32 = V::SCHEMA;
}

/// Top-level layer cache: L1 moka + optional L2 Redis, with metrics and an
/// error allowlist on top.
pub struct LayerCache<V>
where
    V: Clone + Send + Sync + CachedValue + 'static,
{
    inner: LowLevelCache<Result<V, SerializableCachedError>>,
    cache_errors: Arc<[bool]>,
    invalidate_on_update: bool,
    /// When set, every cache key is prefixed with the current secrets-policy
    /// generation so a secrets hot-reload repartitions all tiers. `None`
    /// preserves the pre-feature behaviour (keys used verbatim).
    generation: Option<SecretsGeneration>,
    name: Arc<str>,
    cache_access_counter: BoundUIntCounter,
    cache_miss_counter: BoundUIntCounter,
    cache_entries_gauge: BoundUIntGauge,
    cache_size_gauge: BoundUIntGauge,
    capacity: u64,
    capacity_warn_threshold: u64,
}

impl<V> fmt::Debug for LayerCache<V>
where
    V: Clone + Send + Sync + CachedValue + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayerCache")
            .field("name", &self.name)
            .field("cache_errors", &self.cache_errors)
            .field("invalidate_on_update", &self.invalidate_on_update)
            .field("capacity", &self.capacity)
            .field("capacity_warn_threshold", &self.capacity_warn_threshold)
            .field("l1_entry_count", &self.inner.l1_entry_count())
            .field("l1_weighted_size", &self.inner.l1_weighted_size())
            .finish()
    }
}

impl<V> LayerCache<V>
where
    V: Clone + Send + Sync + CachedValue + 'static,
{
    /// Build a new `LayerCache`. The L2 (Redis) tier is used only when
    /// `args.l2_enabled` is true and `redis` is `Some`; otherwise the cache is
    /// L1-only.
    pub async fn new(
        args: &CacheArgs,
        name: impl Into<String>,
        namespace: impl Into<String>,
        redis: Option<Pool>,
        generation: Option<SecretsGeneration>,
    ) -> Result<Self, LowLevelError> {
        let name = name.into();
        let namespace = namespace.into();

        metrics::metric_base!(
            static CACHE_ACCESS_COUNTER_BASE: UIntCounter,
            name = "cache.operations",
            description = "Number of cache operations (hits and misses)",
            { name, access_type, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_ENTRIES_GAUGE_BASE: UIntGauge,
            name = "cache.entries",
            description = "Number of entries in the cache",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_SIZE_GAUGE_BASE: UIntGauge,
            name = "cache.size",
            description = "Estimate of the size of the cache in bytes",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_OPERATIONS_BASE: UIntCounter,
            name = "cache.l2.operations",
            description = "Number of L2 cache operations by operation and result",
            { name, op, result, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_LATENCY_BASE: Histogram,
            name = "cache.l2.latency_ms",
            unit = "ms",
            description = "Latency of L2 cache operations in milliseconds",
            buckets = [0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 5000.0],
            { name, op, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_LOST_LEASE_COUNTER_BASE: UIntCounter,
            name = "cache.l2.lost_lease",
            description = "Number of L2 writes skipped because the fill lease was lost",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_TOMBSTONE_BLOCKED_COUNTER_BASE: UIntCounter,
            name = "cache.l2.tombstone_blocked",
            description = "Number of L2 writes blocked by an invalidation tombstone",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_PUBSUB_MESSAGE_DROPPED_COUNTER_BASE: UIntCounter,
            name = "cache.l2.pubsub_message_dropped",
            description = "Number of invalidation pubsub messages dropped before they could be applied",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static CACHE_L2_PUBSUB_RECONNECT_COUNTER_BASE: UIntCounter,
            name = "cache.l2.pubsub_reconnect",
            description = "Number of L2 invalidation pubsub reconnects",
            { name, pod_name, pkg_version }
        );

        let pod_name = std::env::var("STORAGE_POD_NAME").unwrap_or_else(|_| "pod".to_string());
        let pkg_version = env!("CARGO_PKG_VERSION").to_string();

        let capacity = args.capacity.as_u64();
        let capacity_warn_threshold = capacity.saturating_mul(u64::from(args.capacity_warn_pct)) / 100;

        let bind_l2_operation = |op: &str, result: &str| {
            CACHE_L2_OPERATIONS_BASE.bind([
                ("name", name.clone()),
                ("op", op.to_string()),
                ("result", result.to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])
        };
        let bind_l2_latency = |op: &str| {
            CACHE_L2_LATENCY_BASE.bind([
                ("name", name.clone()),
                ("op", op.to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])
        };
        let bind_l2_counter = |base: &metrics::UIntCounter| {
            base.bind([
                ("name", name.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])
        };
        let l2_metrics = L2Metrics::new(
            L2OperationMetrics::new(
                bind_l2_operation("read", "hit"),
                bind_l2_operation("read", "miss"),
                bind_l2_operation("read", "error"),
            ),
            L2OperationMetrics::new(
                bind_l2_operation("write", "hit"),
                bind_l2_operation("write", "miss"),
                bind_l2_operation("write", "error"),
            ),
            L2OperationMetrics::new(
                bind_l2_operation("invalidate", "hit"),
                bind_l2_operation("invalidate", "miss"),
                bind_l2_operation("invalidate", "error"),
            ),
            bind_l2_latency("read"),
            bind_l2_latency("write"),
            bind_l2_latency("invalidate"),
            bind_l2_counter(&CACHE_L2_LOST_LEASE_COUNTER_BASE),
            bind_l2_counter(&CACHE_L2_TOMBSTONE_BLOCKED_COUNTER_BASE),
            PubSubMetrics::new(
                bind_l2_counter(&CACHE_L2_PUBSUB_MESSAGE_DROPPED_COUNTER_BASE),
                bind_l2_counter(&CACHE_L2_PUBSUB_RECONNECT_COUNTER_BASE),
            ),
        );

        let mut builder = CacheBuilder::new(namespace)
            .default_ttl(args.time_to_live)
            .max_l1_capacity_bytes(capacity)
            .l2_metrics(l2_metrics);
        if let Some(t) = args.tombstone_ttl {
            builder = builder.tombstone_ttl(t);
        }
        if let Some(l) = args.lease_ttl {
            builder = builder.lease_ttl(l);
        }
        let redis = if args.l2_enabled { redis } else { None };
        let inner = builder.build(redis).await?;

        Ok(Self {
            inner,
            cache_errors: Arc::from(get_cache_errors(&args.errors)),
            invalidate_on_update: args.invalidate_on_update,
            generation,
            cache_access_counter: CACHE_ACCESS_COUNTER_BASE.bind([
                ("name", name.clone()),
                ("access_type", "access".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            cache_miss_counter: CACHE_ACCESS_COUNTER_BASE.bind([
                ("name", name.clone()),
                ("access_type", "miss".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            cache_entries_gauge: CACHE_ENTRIES_GAUGE_BASE.bind([
                ("name", name.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            cache_size_gauge: CACHE_SIZE_GAUGE_BASE.bind([
                ("name", name.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            capacity,
            capacity_warn_threshold,
            name: Arc::from(name.into_boxed_str()),
        })
    }

    /// Prefixes `key` with the current secrets-policy generation when
    /// partitioning is enabled, so entries written under one policy are
    /// unreachable after the policy changes. Returns the key untouched when no
    /// generation was configured, preserving the exact pre-feature key layout
    /// (and L2 Redis wire format) for callers that don't opt in.
    fn effective_key<'a>(&self, key: &'a str) -> Cow<'a, str> {
        self.effective_key_with(key, self.generation_snapshot())
    }

    /// Snapshots the current secrets-policy generation (`None` when partitioning
    /// is disabled). A caller that reads at request entry and writes back at
    /// response completion should snapshot once at entry and thread the value
    /// into the `*_with_generation` write methods, so the fill lands in the same
    /// partition the read observed even if a hot-reload bumps the generation in
    /// between (mirrors the single-key guarantee of `try_get_with_by_ref`).
    pub fn generation_snapshot(&self) -> Option<u64> {
        self.generation.as_ref().map(SecretsGeneration::current)
    }

    /// Like [`effective_key`](Self::effective_key), but uses a caller-supplied
    /// generation snapshot instead of reading the live value.
    fn effective_key_with<'a>(&self, key: &'a str, generation: Option<u64>) -> Cow<'a, str> {
        match generation {
            Some(generation) => Cow::Owned(format!("{generation:016x}:{key}")),
            None => Cow::Borrowed(key),
        }
    }

    /// Look up an entry from L1 only — returns `None` on L1 miss without
    /// consulting L2. Preserved for callers that explicitly want L1-only
    /// semantics.
    pub async fn get(&self, key: &str) -> Option<Result<V, Arc<StorageError>>> {
        self.cache_access_counter.add(1);
        let result = self.inner.peek_l1(self.effective_key(key).as_ref()).await;
        if result.is_none() {
            self.cache_miss_counter.add(1);
        }
        result.map(|stored| match stored {
            Ok(v) => Ok(v),
            Err(serr) => Err(Arc::new(Box::new(serr) as StorageError)),
        })
    }

    /// Look up an entry from L1; on miss, consult L2 before returning. Does
    /// not run an init or take a lease — this is a peek path that lets
    /// hot-call paths benefit from cross-pod L2 hits without paying the
    /// stampede coordination cost of `try_get_with_by_ref`.
    pub async fn get_two_tier(&self, key: &str) -> Option<Result<V, Arc<StorageError>>> {
        self.cache_access_counter.add(1);
        let stored = match self.inner.peek_l2_or_l1(self.effective_key(key).as_ref()).await {
            Ok(v) => v,
            Err(e) => {
                warn_limited!(name = %self.name, error = %e, "L2 peek failed");
                None
            },
        };
        if stored.is_none() {
            self.cache_miss_counter.add(1);
        }
        stored.map(|inner| match inner {
            Ok(v) => Ok(v),
            Err(serr) => Err(Arc::new(Box::new(serr) as StorageError)),
        })
    }

    /// Whether this cache has a configured L2 Redis tier.
    pub fn has_l2(&self) -> bool {
        self.inner.has_l2()
    }

    /// Look up an entry, populating it via `init` on miss.
    pub async fn try_get_with_by_ref<F>(&self, key: &str, init: F) -> Result<V, Arc<StorageError>>
    where
        F: Future<Output = Result<V, StorageError>>,
    {
        self.try_get_with_by_ref_with_options(key, self.generation_snapshot(), CacheErrorPolicy::Configured, init)
            .await
    }

    /// Look up an entry under an explicit secrets-policy generation,
    /// populating it via `init` on miss and applying a per-fill error policy.
    ///
    /// Callers handling a request across multiple cache operations should take
    /// one [`generation_snapshot`](Self::generation_snapshot) at request entry
    /// and pass it here and to the other `*_with_generation` methods. This
    /// prevents a hot reload from moving a mid-request fill into a newer policy
    /// partition. [`CacheErrorPolicy::Never`] still coalesces concurrent callers
    /// onto the same failing init, but the error is not retained afterward.
    pub async fn try_get_with_by_ref_with_options<F>(
        &self,
        key: &str,
        generation: Option<u64>,
        error_policy: CacheErrorPolicy,
        init: F,
    ) -> Result<V, Arc<StorageError>>
    where
        F: Future<Output = Result<V, StorageError>>,
    {
        self.cache_access_counter.add(1);

        let cache_errors = Arc::clone(&self.cache_errors);
        let name = Arc::clone(&self.name);
        let miss_counter = self.cache_miss_counter.clone();

        let init_wrapper = async move {
            trace_limited!(name = %name, "Cache executing init future");
            miss_counter.add(1);
            match init.await {
                Ok(v) => Ok::<Result<V, SerializableCachedError>, WrapperError>(Ok(v)),
                Err(e) => {
                    let status = e.tonic_status();
                    let code_idx = status.code() as usize;
                    if error_policy == CacheErrorPolicy::Configured
                        && cache_errors.get(code_idx).copied().unwrap_or(false)
                    {
                        Ok::<Result<V, SerializableCachedError>, WrapperError>(Err(
                            SerializableCachedError::from_storage_error(&e),
                        ))
                    } else {
                        Err(WrapperError::NonCacheable(Arc::new(e)))
                    }
                },
            }
        };

        let stored = self
            .inner
            .try_get_with_by_ref(self.effective_key_with(key, generation).as_ref(), init_wrapper)
            .await;
        let weighted_size = self.inner.l1_weighted_size();
        self.cache_entries_gauge.record(self.inner.l1_entry_count());
        self.cache_size_gauge.record(weighted_size);
        self.check_capacity_threshold(weighted_size);

        match stored {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(serr)) => Err(Arc::new(Box::new(serr) as StorageError)),
            Err(arc_wrapper) => match &*arc_wrapper {
                WrapperError::NonCacheable(arc_err) => Err(arc_err.clone()),
                WrapperError::CacheBackend(e) => {
                    warn_limited!(name = %self.name, error = %e, "Cache backend failure");
                    let msg = format!("cache backend error: {e}");
                    Err(Arc::new(Box::new(BackendError(msg)) as StorageError))
                },
            },
        }
    }

    /// Insert a value (or an allowlisted error) directly. No-op when the
    /// stored type is the wrong shape for caching.
    pub async fn insert(&self, key: String, value: Result<V, Arc<StorageError>>) {
        let Some(stored) = self.cacheable_stored_value(value) else {
            return;
        };
        let key = self.effective_key(&key).into_owned();
        if let Err(e) = self.inner.insert(key, stored).await {
            warn_limited!(name = %self.name, error = %e, "Cache insert failed");
        }
        self.record_l1_metrics();
    }

    /// Insert a value (or an allowlisted error) into local L1 only. No-op when
    /// the stored type is the wrong shape for caching.
    pub async fn insert_l1(&self, key: String, value: Result<V, Arc<StorageError>>) {
        let Some(stored) = self.cacheable_stored_value(value) else {
            return;
        };
        let key = self.effective_key(&key).into_owned();
        self.inner.insert_l1(key, stored).await;
        self.record_l1_metrics();
    }

    /// Insert a value (or an allowlisted error) into L2 only. No-op when the
    /// stored type is the wrong shape for caching or L2 is disabled.
    pub async fn insert_l2(&self, key: String, value: Result<V, Arc<StorageError>>) {
        let Some(stored) = self.cacheable_stored_value(value) else {
            return;
        };
        let key = self.effective_key(&key).into_owned();
        if let Err(e) = self.inner.insert_l2(key, stored).await {
            warn_limited!(name = %self.name, error = %e, "Cache L2 insert failed");
        }
    }

    /// Like [`get_two_tier`](Self::get_two_tier), but keys the lookup by a
    /// caller-supplied [`generation_snapshot`](Self::generation_snapshot).
    pub async fn get_two_tier_with_generation(
        &self,
        key: &str,
        generation: Option<u64>,
    ) -> Option<Result<V, Arc<StorageError>>> {
        self.cache_access_counter.add(1);
        let stored = match self
            .inner
            .peek_l2_or_l1(self.effective_key_with(key, generation).as_ref())
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn_limited!(name = %self.name, error = %e, "L2 peek failed");
                None
            },
        };
        if stored.is_none() {
            self.cache_miss_counter.add(1);
        }
        stored.map(|inner| match inner {
            Ok(v) => Ok(v),
            Err(serr) => Err(Arc::new(Box::new(serr) as StorageError)),
        })
    }

    /// Like [`insert_l1`](Self::insert_l1), but keys the entry by a
    /// caller-supplied [`generation_snapshot`](Self::generation_snapshot).
    pub async fn insert_l1_with_generation(
        &self,
        key: String,
        value: Result<V, Arc<StorageError>>,
        generation: Option<u64>,
    ) {
        let Some(stored) = self.cacheable_stored_value(value) else {
            return;
        };
        let key = self.effective_key_with(&key, generation).into_owned();
        self.inner.insert_l1(key, stored).await;
        self.record_l1_metrics();
    }

    /// Like [`insert_l2`](Self::insert_l2), but keys the entry by a
    /// caller-supplied [`generation_snapshot`](Self::generation_snapshot).
    pub async fn insert_l2_with_generation(
        &self,
        key: String,
        value: Result<V, Arc<StorageError>>,
        generation: Option<u64>,
    ) {
        let Some(stored) = self.cacheable_stored_value(value) else {
            return;
        };
        let key = self.effective_key_with(&key, generation).into_owned();
        if let Err(e) = self.inner.insert_l2(key, stored).await {
            warn_limited!(name = %self.name, error = %e, "Cache L2 insert failed");
        }
    }

    fn cacheable_stored_value(
        &self,
        value: Result<V, Arc<StorageError>>,
    ) -> Option<Result<V, SerializableCachedError>> {
        Some(match value {
            Ok(v) => Ok(v),
            Err(arc_err) => {
                let status = arc_err.tonic_status();
                let code_idx = status.code() as usize;
                if !self.cache_errors.get(code_idx).copied().unwrap_or(false) {
                    return None;
                }
                Err(SerializableCachedError::from_storage_error(&arc_err))
            },
        })
    }

    fn record_l1_metrics(&self) {
        let weighted_size = self.inner.l1_weighted_size();
        self.cache_entries_gauge.record(self.inner.l1_entry_count());
        self.cache_size_gauge.record(weighted_size);
        self.check_capacity_threshold(weighted_size);
    }

    /// Invalidate a single key across both cache levels. No-op when the
    /// cache was configured with `invalidate_on_update = false`. Errors are
    /// swallowed (logged) — invalidation is best-effort and must not fail
    /// user requests.
    pub async fn invalidate(&self, key: &str) {
        if !self.invalidate_on_update {
            return;
        }
        if let Err(e) = self.inner.invalidate(self.effective_key(key).as_ref()).await {
            warn_limited!(name = %self.name, error = %e, "Cache invalidate failed");
        }
        self.cache_entries_gauge.record(self.inner.l1_entry_count());
        self.cache_size_gauge.record(self.inner.l1_weighted_size());
    }

    /// Invalidate every key whose prefix matches. Errors are swallowed
    /// (logged) — symmetric with `invalidate`. Returns `()` on success or
    /// log-and-go on failure.
    pub async fn invalidate_prefix(&self, prefix: &str) {
        if !self.invalidate_on_update {
            return;
        }
        if let Err(e) = self.inner.invalidate_prefix(self.effective_key(prefix).as_ref()).await {
            warn_limited!(name = %self.name, error = %e, "Cache invalidate_prefix failed");
        }
        self.cache_entries_gauge.record(self.inner.l1_entry_count());
        self.cache_size_gauge.record(self.inner.l1_weighted_size());
    }

    /// Shut down any background tasks (pub/sub subscriber).
    pub async fn shutdown(&self) {
        self.inner.shutdown().await;
    }

    /// Warn when cache utilization crosses the configured warn threshold.
    #[inline]
    fn check_capacity_threshold(&self, weighted_size: u64) {
        if self.capacity == 0 || weighted_size < self.capacity_warn_threshold {
            return;
        }
        warn_limited!(
            cache_name = %self.name,
            size_bytes = weighted_size,
            capacity_bytes = self.capacity,
            utilization_pct = weighted_size * 100 / self.capacity,
            "Cache nearing capacity, eviction pressure is high"
        );
    }
}

/// Internal error type for the init future. `From<LowLevelError>` is required
/// by `LowLevelCache::try_get_with_by_ref`.
#[derive(Debug)]
enum WrapperError {
    NonCacheable(Arc<StorageError>),
    CacheBackend(LowLevelError),
}

impl std::fmt::Display for WrapperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WrapperError::NonCacheable(e) => write!(f, "non-cacheable: {e}"),
            WrapperError::CacheBackend(e) => write!(f, "cache backend: {e}"),
        }
    }
}

impl std::error::Error for WrapperError {}

impl From<LowLevelError> for WrapperError {
    fn from(err: LowLevelError) -> Self {
        WrapperError::CacheBackend(err)
    }
}

#[derive(Debug)]
struct BackendError(String);

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendError {}

impl StorageErrorExt for BackendError {
    fn tonic_status(&self) -> tonic::Status {
        storage_core_types::error::status_with_default_error_info(tonic::Code::Internal, self.0.clone(), "Internal")
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::INTERNAL_SERVER_ERROR
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.0.capacity()
    }
}

fn get_cache_errors(error_str: &[String]) -> Vec<bool> {
    let mut cache_errors = vec![false; tonic::Code::Unauthenticated as usize + 1];
    for arg in error_str.iter() {
        match arg.as_str() {
            "Cancelled" => cache_errors[tonic::Code::Cancelled as usize] = true,
            "Unknown" => cache_errors[tonic::Code::Unknown as usize] = true,
            "InvalidArgument" => cache_errors[tonic::Code::InvalidArgument as usize] = true,
            "DeadlineExceeded" => cache_errors[tonic::Code::DeadlineExceeded as usize] = true,
            "NotFound" => cache_errors[tonic::Code::NotFound as usize] = true,
            "AlreadyExists" => cache_errors[tonic::Code::AlreadyExists as usize] = true,
            "PermissionDenied" | "AccessDenied" => cache_errors[tonic::Code::PermissionDenied as usize] = true,
            "ResourceExhausted" => cache_errors[tonic::Code::ResourceExhausted as usize] = true,
            "FailedPrecondition" => cache_errors[tonic::Code::FailedPrecondition as usize] = true,
            "Aborted" => cache_errors[tonic::Code::Aborted as usize] = true,
            "OutOfRange" => cache_errors[tonic::Code::OutOfRange as usize] = true,
            "Unimplemented" => cache_errors[tonic::Code::Unimplemented as usize] = true,
            "Internal" => cache_errors[tonic::Code::Internal as usize] = true,
            "Unavailable" => cache_errors[tonic::Code::Unavailable as usize] = true,
            "DataLoss" => cache_errors[tonic::Code::DataLoss as usize] = true,
            "Unauthenticated" => cache_errors[tonic::Code::Unauthenticated as usize] = true,
            _ => warn_limited!(arg, "Invalid cache error type"),
        }
    }
    cache_errors
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn args(invalidate_on_update: bool, errors: Vec<String>) -> CacheArgs {
        CacheArgs {
            capacity: ByteSize::mib(1),
            time_to_live: Duration::from_secs(60),
            errors,
            invalidate_on_update,
            capacity_warn_pct: default_capacity_warn_pct(),
            tombstone_ttl: None,
            lease_ttl: None,
            l2_enabled: false,
        }
    }

    async fn build_cache(invalidate_on_update: bool) -> LayerCache<String> {
        LayerCache::new(&args(invalidate_on_update, vec![]), "test", "test", None, None)
            .await
            .expect("L1-only LayerCache build")
    }

    /// Minimal `StorageErrorExt` impl for tests.
    #[derive(Debug)]
    struct TestErr {
        code: tonic::Code,
        reason: &'static str,
    }

    impl std::fmt::Display for TestErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}: test", self.reason)
        }
    }

    impl std::error::Error for TestErr {}

    impl StorageErrorExt for TestErr {
        fn tonic_status(&self) -> tonic::Status {
            storage_core_types::error::status_with_default_error_info(self.code, self.to_string(), self.reason)
        }

        fn http_status(&self) -> http::StatusCode {
            http::StatusCode::INTERNAL_SERVER_ERROR
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    fn err(code: tonic::Code, reason: &'static str) -> StorageError {
        Box::new(TestErr { code, reason })
    }

    #[derive(Debug)]
    struct RichErr {
        code: tonic::Code,
        message: String,
        reason: String,
        domain: String,
        metadata: Vec<(String, String)>,
    }

    impl std::fmt::Display for RichErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.message)
        }
    }

    impl std::error::Error for RichErr {}

    impl StorageErrorExt for RichErr {
        fn tonic_status(&self) -> tonic::Status {
            storage_core_types::error::status_with_error_info(
                self.code,
                self.message.clone(),
                self.reason.clone(),
                self.domain.clone(),
                self.metadata.iter().map(|(k, v)| (k.clone(), v.clone())),
            )
        }

        fn http_status(&self) -> http::StatusCode {
            http::StatusCode::NOT_FOUND
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    fn assert_error_info(status: &tonic::Status, reason: &str, domain: &str, metadata: &[(&str, &str)]) {
        let (got_reason, got_domain, got_metadata) = storage_core_types::error::extract_error_info(status)
            .unwrap_or_else(|| panic!("status missing ErrorInfo: {status:?}"));
        assert_eq!(got_reason, reason, "ErrorInfo reason");
        assert_eq!(got_domain, domain, "ErrorInfo domain");
        for (k, v) in metadata {
            assert_eq!(
                got_metadata.get(*k).map(String::as_str),
                Some(*v),
                "ErrorInfo metadata key {k}"
            );
        }
    }

    #[tokio::test]
    async fn cached_error_preserves_error_info_details() {
        let cache: LayerCache<String> =
            LayerCache::new(&args(false, vec!["NotFound".to_string()]), "test", "test", None, None)
                .await
                .expect("LayerCache build");

        let make_err = || -> StorageError {
            Box::new(RichErr {
                code: tonic::Code::NotFound,
                message: "object not found".to_string(),
                reason: "NoSuchKey".to_string(),
                domain: "s3".to_string(),
                metadata: vec![("bucket".to_string(), "foo".to_string())],
            })
        };

        let first = cache
            .try_get_with_by_ref("k", async move { Err::<String, _>(make_err()) })
            .await
            .expect_err("first call should error");
        assert_eq!(first.tonic_status().code(), tonic::Code::NotFound);
        assert_error_info(&first.tonic_status(), "NoSuchKey", "s3", &[("bucket", "foo")]);

        let second = cache
            .try_get_with_by_ref("k", async {
                panic!("init must not run on a cached-error hit");
                #[allow(unreachable_code)]
                Ok::<String, StorageError>(String::new())
            })
            .await
            .expect_err("cached error path");
        assert_eq!(second.tonic_status().code(), tonic::Code::NotFound);
        assert_error_info(&second.tonic_status(), "NoSuchKey", "s3", &[("bucket", "foo")]);
    }

    #[tokio::test]
    async fn invalidate_clears_l1_when_enabled() {
        let cache = build_cache(true).await;
        cache.insert("k".to_string(), Ok("v".to_string())).await;
        assert!(cache.get("k").await.is_some(), "value should be present after insert");

        cache.invalidate("k").await;
        assert!(cache.get("k").await.is_none(), "value should be gone after invalidate");
    }

    #[tokio::test]
    async fn invalidate_is_noop_when_disabled() {
        let cache = build_cache(false).await;
        cache.insert("k".to_string(), Ok("v".to_string())).await;

        cache.invalidate("k").await;
        let after = cache.get("k").await;
        assert!(
            matches!(after, Some(Ok(ref s)) if s == "v"),
            "value should still be present when invalidate_on_update is false: {after:?}",
        );
    }

    #[tokio::test]
    async fn invalidate_prefix_clears_subtree_via_starts_with() {
        let cache = build_cache(true).await;

        cache.insert("a/b/c/".to_string(), Ok("v1".to_string())).await;
        cache.insert("a/b/c/d/".to_string(), Ok("v2".to_string())).await;
        cache.insert("a/b/c/d/e/".to_string(), Ok("v3".to_string())).await;
        cache.insert("a/sibling/".to_string(), Ok("v4".to_string())).await;

        cache.invalidate_prefix("a/b/").await;
        cache.inner.run_pending_l1_tasks().await;

        assert!(cache.get("a/b/c/").await.is_none(), "a/b/c/ should be invalidated");
        assert!(cache.get("a/b/c/d/").await.is_none(), "a/b/c/d/ should be invalidated");
        assert!(
            cache.get("a/b/c/d/e/").await.is_none(),
            "a/b/c/d/e/ should be invalidated"
        );
        assert!(cache.get("a/sibling/").await.is_some(), "a/sibling/ must remain");
    }

    #[tokio::test]
    async fn invalidate_prefix_is_noop_when_disabled() {
        let cache = build_cache(false).await;

        cache.insert("a/1".to_string(), Ok("v1".to_string())).await;
        cache.insert("a/2".to_string(), Ok("v2".to_string())).await;

        cache.invalidate_prefix("a/").await;
        cache.inner.run_pending_l1_tasks().await;

        assert!(
            cache.get("a/1").await.is_some(),
            "a/1 should remain when invalidate_on_update is false"
        );
        assert!(
            cache.get("a/2").await.is_some(),
            "a/2 should remain when invalidate_on_update is false"
        );
    }

    #[tokio::test]
    async fn cached_init_error_in_allowlist_is_returned_on_subsequent_get() {
        let cache: LayerCache<String> =
            LayerCache::new(&args(false, vec!["NotFound".to_string()]), "test", "test", None, None)
                .await
                .expect("LayerCache build");

        let calls = Arc::new(AtomicUsize::new(0));
        let init = || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(err(tonic::Code::NotFound, "NotFound"))
            }
        };

        let first = cache.try_get_with_by_ref("k", init()).await;
        assert!(matches!(&first, Err(e) if e.tonic_status().code() == tonic::Code::NotFound));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = cache.try_get_with_by_ref("k", init()).await;
        assert!(matches!(&second, Err(e) if e.tonic_status().code() == tonic::Code::NotFound));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "allowlisted error should be cached; init should not re-run"
        );
    }

    #[tokio::test]
    async fn init_error_not_in_allowlist_propagates_and_is_not_cached() {
        let cache: LayerCache<String> =
            LayerCache::new(&args(false, vec!["NotFound".to_string()]), "test", "test", None, None)
                .await
                .expect("LayerCache build");

        let calls = Arc::new(AtomicUsize::new(0));
        let init = || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(err(tonic::Code::Internal, "Internal"))
            }
        };

        let first = cache.try_get_with_by_ref("k", init()).await;
        assert!(matches!(&first, Err(e) if e.tonic_status().code() == tonic::Code::Internal));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = cache.try_get_with_by_ref("k", init()).await;
        assert!(matches!(&second, Err(e) if e.tonic_status().code() == tonic::Code::Internal));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "non-allowlisted error should not be cached; init should re-run"
        );
    }

    #[tokio::test]
    async fn never_error_policy_overrides_configured_allowlist() {
        let cache: LayerCache<String> =
            LayerCache::new(&args(false, vec!["Internal".to_string()]), "test", "test", None, None)
                .await
                .expect("LayerCache build");

        let calls = Arc::new(AtomicUsize::new(0));
        let init = || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(err(tonic::Code::Internal, "Internal"))
            }
        };

        for expected_calls in 1..=2 {
            let result = cache
                .try_get_with_by_ref_with_options("k", cache.generation_snapshot(), CacheErrorPolicy::Never, init())
                .await;
            assert!(matches!(&result, Err(e) if e.tonic_status().code() == tonic::Code::Internal));
            assert_eq!(
                calls.load(Ordering::SeqCst),
                expected_calls,
                "Never must not retain even an explicitly allowlisted error"
            );
        }
    }

    fn secrets_figment(key: &str) -> figment::Figment {
        figment::Figment::new().join((key.to_string(), "value".to_string()))
    }

    async fn build_cache_with_generation(generation: SecretsGeneration) -> LayerCache<String> {
        LayerCache::new(
            &args(true, vec!["NotFound".to_string()]),
            "test",
            "test",
            None,
            Some(generation),
        )
        .await
        .expect("L1-only LayerCache with generation")
    }

    /// Removal direction: a value cached while an endpoint was configured must
    /// become unreachable once the secrets policy changes (endpoint revoked).
    #[tokio::test]
    async fn generation_change_hides_previously_cached_value() {
        let generation = SecretsGeneration::new(Some(&secrets_figment("https://a.example")));
        let cache = build_cache_with_generation(generation.clone()).await;

        cache.insert("addr".to_string(), Ok("cached".to_string())).await;
        assert!(
            matches!(cache.get("addr").await, Some(Ok(ref v)) if v == "cached"),
            "value should be present under the original policy"
        );

        // Simulate a hot-reload that changes the endpoint allowlist.
        generation.update(Some(&secrets_figment("https://b.example")));

        assert!(
            cache.get("addr").await.is_none(),
            "value cached under the previous secrets generation must be unreachable"
        );
    }

    /// A write-back that snapshots the generation at request entry must land in
    /// the partition observed at entry, even if a hot-reload bumps the generation
    /// before the insert runs — otherwise a mid-read revocation could serve a
    /// revoked endpoint's body under the new policy.
    #[tokio::test]
    async fn insert_with_snapshotted_generation_pins_the_partition() {
        let generation = SecretsGeneration::new(Some(&secrets_figment("https://a.example")));
        let cache = build_cache_with_generation(generation.clone()).await;

        // Snapshot at "request entry".
        let snapshot = cache.generation_snapshot();

        // A hot-reload bumps the generation before the write-back runs.
        generation.update(Some(&secrets_figment("https://b.example")));

        // The write-back keys off the entry snapshot, so it lands in the old partition.
        cache
            .insert_l1_with_generation("addr".to_string(), Ok("cached".to_string()), snapshot)
            .await;

        // A reader on the current (new) policy can't see it...
        assert!(
            cache.get("addr").await.is_none(),
            "write-back under the entry snapshot must not be visible to the new policy"
        );
        // ...but it is reachable under the snapshot it was written with.
        assert!(
            matches!(cache.get_two_tier_with_generation("addr", snapshot).await, Some(Ok(ref v)) if v == "cached"),
            "entry must be reachable under the snapshot it was written with"
        );
    }

    #[tokio::test]
    async fn fill_with_snapshotted_generation_pins_the_partition() {
        let generation = SecretsGeneration::new(Some(&secrets_figment("https://a.example")));
        let cache = build_cache_with_generation(generation.clone()).await;
        let snapshot = cache.generation_snapshot();

        let generation_during_fill = generation.clone();
        let value = cache
            .try_get_with_by_ref_with_options("addr", snapshot, CacheErrorPolicy::Configured, async move {
                generation_during_fill.update(Some(&secrets_figment("https://b.example")));
                Ok::<String, StorageError>("cached".to_string())
            })
            .await
            .expect("fill under entry generation");
        assert_eq!(value, "cached");

        assert!(
            cache.get("addr").await.is_none(),
            "fill under the entry snapshot must not be visible to the new policy"
        );
        assert!(
            matches!(cache.get_two_tier_with_generation("addr", snapshot).await, Some(Ok(ref v)) if v == "cached"),
            "fill must be retained in the explicitly supplied partition"
        );
    }

    /// Addition direction: an allowlisted `NotFound` cached while an endpoint was
    /// unconfigured must not keep denying it after the policy adds the endpoint.
    #[tokio::test]
    async fn generation_change_clears_cached_notfound() {
        let generation = SecretsGeneration::new(None);
        let cache = build_cache_with_generation(generation.clone()).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let deny = || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(err(tonic::Code::NotFound, "NotFound"))
            }
        };

        assert!(cache.try_get_with_by_ref("addr", deny()).await.is_err());
        // Same policy: the cached NotFound is served without re-running init.
        assert!(cache.try_get_with_by_ref("addr", deny()).await.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "NotFound should be cached within a generation"
        );

        // The endpoint is added on hot-reload; the stale NotFound must not be served.
        generation.update(Some(&secrets_figment("https://a.example")));
        let result = cache
            .try_get_with_by_ref("addr", async {
                Ok::<String, StorageError>("now-configured".to_string())
            })
            .await;
        assert!(
            matches!(&result, Ok(v) if v == "now-configured"),
            "after the policy adds the endpoint it must resolve fresh, not from the cached NotFound: {result:?}"
        );
    }

    /// A cache built without a generation must use keys verbatim (no prefix), so
    /// existing consumers and the L2 wire format are unchanged.
    #[tokio::test]
    async fn no_generation_uses_key_verbatim() {
        let cache = build_cache(true).await;
        assert_eq!(cache.effective_key("addr"), std::borrow::Cow::Borrowed("addr"));
    }
}
