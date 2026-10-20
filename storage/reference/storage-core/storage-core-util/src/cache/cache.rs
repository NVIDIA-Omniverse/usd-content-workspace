// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fred::clients::Pool;
use fred::interfaces::{ClientLike, KeysInterface, PubsubInterface};
use fred::types::scan::Scanner;
use fred::types::scripts::Script;
use fred::types::Value;
use futures::stream::TryStreamExt;

use super::envelope::Envelope;
use super::error::Error;
use super::expiry::{CacheEntry, PerEntryExpiry};
use super::lua::Scripts;
use super::pubsub::{self, InvalidationMessage, PubSubHandle};
use super::sized::{CachedSize, CachedValue};
use crate::observability::metrics::{BoundHistogram, BoundUIntCounter};
use crate::{trace_limited, warn_limited};

const PREFIX_INVALIDATE_SCAN_COUNT: u32 = 256;

/// Upper bound on the number of keys held in the local "distrust L2" set at once. Entries are only
/// added when an exact-key invalidation fails against Redis (rare), and auto-expire after the data
/// TTL, so this cap is a memory backstop for a prolonged outage, not an expected steady state.
const L2_DISTRUST_MAX_KEYS: u64 = 100_000;

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

/// L1-backed slot for the moka cache. Wraps the user value so the moka weigher
/// can read the key and value `cached_size` (proportional to retained bytes)
/// without forcing a clone on every weight check.
fn weight_of<V: CachedSize>(k: &String, v: &CacheEntry<V>) -> u32 {
    k.cached_size().saturating_add(v.value.cached_size())
}

fn redis_glob_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[derive(Clone, Copy)]
enum L2Operation {
    Read,
    Write,
    Invalidate,
}

#[derive(Clone, Copy)]
enum L2OperationResult {
    Hit,
    Miss,
    Error,
}

#[derive(Clone)]
pub(crate) struct L2OperationMetrics {
    hit: BoundUIntCounter,
    miss: BoundUIntCounter,
    error: BoundUIntCounter,
}

impl L2OperationMetrics {
    pub(crate) fn new(hit: BoundUIntCounter, miss: BoundUIntCounter, error: BoundUIntCounter) -> Self {
        Self { hit, miss, error }
    }

    fn record(&self, result: L2OperationResult) {
        match result {
            L2OperationResult::Hit => self.hit.add(1),
            L2OperationResult::Miss => self.miss.add(1),
            L2OperationResult::Error => self.error.add(1),
        }
    }
}

#[derive(Clone)]
pub(crate) struct L2Metrics {
    read: L2OperationMetrics,
    write: L2OperationMetrics,
    invalidate: L2OperationMetrics,
    read_latency: BoundHistogram,
    write_latency: BoundHistogram,
    invalidate_latency: BoundHistogram,
    lost_lease: BoundUIntCounter,
    tombstone_blocked: BoundUIntCounter,
    pubsub: pubsub::PubSubMetrics,
}

impl L2Metrics {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        read: L2OperationMetrics,
        write: L2OperationMetrics,
        invalidate: L2OperationMetrics,
        read_latency: BoundHistogram,
        write_latency: BoundHistogram,
        invalidate_latency: BoundHistogram,
        lost_lease: BoundUIntCounter,
        tombstone_blocked: BoundUIntCounter,
        pubsub: pubsub::PubSubMetrics,
    ) -> Self {
        Self {
            read,
            write,
            invalidate,
            read_latency,
            write_latency,
            invalidate_latency,
            lost_lease,
            tombstone_blocked,
            pubsub,
        }
    }

    fn record_operation(&self, op: L2Operation, result: L2OperationResult) {
        match op {
            L2Operation::Read => self.read.record(result),
            L2Operation::Write => self.write.record(result),
            L2Operation::Invalidate => self.invalidate.record(result),
        }
    }

    fn record_latency(&self, op: L2Operation, elapsed: Duration) {
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
        match op {
            L2Operation::Read => self.read_latency.record(elapsed_ms),
            L2Operation::Write => self.write_latency.record(elapsed_ms),
            L2Operation::Invalidate => self.invalidate_latency.record(elapsed_ms),
        }
    }

    fn record_lost_lease(&self) {
        self.lost_lease.add(1);
    }

    fn record_tombstone_blocked(&self) {
        self.tombstone_blocked.add(1);
    }

    fn pubsub_metrics(&self) -> pubsub::PubSubMetrics {
        self.pubsub.clone()
    }
}

struct Inner<V> {
    l1: moka::future::Cache<String, CacheEntry<V>>,
    redis: Option<Pool>,
    scripts: Scripts,
    namespace: String,
    default_ttl: Duration,
    tombstone_ttl: Duration,
    lease_ttl: Duration,
    pubsub: tokio::sync::Mutex<Option<PubSubHandle>>,
    /// Monotonic counter bumped before every local L1 invalidation (whether
    /// originated locally or driven by a pub/sub message). Read paths snapshot
    /// it before the L2 GET and validate it again when moka publishes the L1
    /// entry — if it advanced in the gap, the backfill expires immediately to
    /// avoid serving the pre-invalidate value past its tombstone.
    local_generation: Arc<AtomicU64>,
    /// Keys whose L2 invalidation could not be confirmed (the invalidate script or the broadcast
    /// failed), so Redis may still hold the pre-invalidate value with **no** tombstone. While a key
    /// is present here the L2 read paths treat it as a miss and refill L1 from the source of truth,
    /// so a stale L2 hit can't repopulate L1. Entries auto-expire after the data TTL — the maximum
    /// remaining life of the stale L2 value — and are cleared early by a later successful
    /// invalidation of the same key.
    local_l2_distrust: moka::future::Cache<String, ()>,
    l2_metrics: Option<L2Metrics>,
}

/// Cancellation guard for a Redis fill lease. A successful CAS write or an
/// observed lost lease disarms it. If the initializer future is dropped while
/// it still owns the lease, `Drop` schedules a best-effort token-checked
/// release so a waiter can take over without waiting for the lease TTL.
struct LeaseReleaseGuard {
    redis: Pool,
    script: Script,
    lease_key: String,
    token: u64,
    armed: bool,
}

impl LeaseReleaseGuard {
    fn new(redis: Pool, script: Script, lease_key: String, token: u64) -> Self {
        Self {
            redis,
            script,
            lease_key,
            token,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for LeaseReleaseGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let redis = self.redis.clone();
        let script = self.script.clone();
        let lease_key = self.lease_key.clone();
        let token = self.token;
        runtime.spawn(async move {
            let released: Result<i64, fred::error::Error> = script
                .evalsha_with_reload(
                    redis.next_connected(),
                    vec![lease_key.clone()],
                    vec![Value::Integer(token as i64)],
                )
                .await;
            match released {
                Ok(1) => trace_limited!(key = %lease_key, "released abandoned L2 fill lease"),
                Ok(_) => {},
                Err(e) => {
                    warn_limited!(error = %e, key = %lease_key, "failed to release abandoned L2 fill lease")
                },
            }
        });
    }
}

pub struct Cache<V> {
    inner: Arc<Inner<V>>,
}

impl<V> Clone for Cache<V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<V> Cache<V>
where
    V: Clone + Send + Sync + CachedSize + 'static,
{
    // --- key formatting ---

    fn data_key(&self, key: &str) -> String {
        format!("{}:d:{}", self.inner.namespace, key)
    }

    fn data_key_prefix_pattern(&self, prefix: &str) -> String {
        format!(
            "{}:d:{}*",
            redis_glob_escape(&self.inner.namespace),
            redis_glob_escape(prefix)
        )
    }

    fn lease_key(&self, key: &str) -> String {
        format!("{}:l:{}", self.inner.namespace, key)
    }

    fn tombstone_key(&self, key: &str) -> String {
        format!("{}:t:{}", self.inner.namespace, key)
    }

    fn invalidation_channel(&self) -> String {
        format!("{}:invalidate", self.inner.namespace)
    }

    /// Shut down the pub/sub subscriber background task.
    pub async fn shutdown(&self) {
        if let Some(handle) = self.inner.pubsub.lock().await.take() {
            handle.shutdown().await;
        }
    }

    /// Read a value from L1 only, without populating from L2 or running an init.
    pub async fn peek_l1(&self, key: &str) -> Option<V> {
        self.inner.l1.get(key).await.map(|entry| entry.value)
    }

    pub fn l1_entry_count(&self) -> u64 {
        self.inner.l1.entry_count()
    }

    pub fn l1_weighted_size(&self) -> u64 {
        self.inner.l1.weighted_size()
    }

    pub fn has_l2(&self) -> bool {
        self.inner.redis.is_some()
    }

    fn record_l2_operation(&self, op: L2Operation, result: L2OperationResult) {
        if let Some(metrics) = self.inner.l2_metrics.as_ref() {
            metrics.record_operation(op, result);
        }
    }

    fn record_l2_latency(&self, op: L2Operation, elapsed: Duration) {
        if let Some(metrics) = self.inner.l2_metrics.as_ref() {
            metrics.record_latency(op, elapsed);
        }
    }

    /// Insert into the local L1 tier only, using the cache-wide default TTL.
    pub async fn insert_l1(&self, key: String, value: V) {
        self.insert_l1_with_ttl(key, value, self.inner.default_ttl).await;
    }

    async fn insert_l1_with_ttl(&self, key: String, value: V, ttl: Duration) {
        // Bump generation before the L1 insert so any concurrent L2 reader
        // on this instance that snapshotted before us will skip its own
        // backfill. Mirrors the pubsub handler's ordering.
        let expected_local_generation = self
            .inner
            .local_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.inner
            .l1
            .insert(
                key,
                CacheEntry {
                    value,
                    ttl: Some(ttl),
                    expected_local_generation: Some(expected_local_generation),
                },
            )
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn run_pending_l1_tasks(&self) {
        self.inner.l1.run_pending_tasks().await;
    }

    #[cfg(test)]
    pub(crate) fn local_generation(&self) -> u64 {
        self.inner.local_generation.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) async fn pubsub_connection_ids(&self) -> Vec<i64> {
        self.inner
            .pubsub
            .lock()
            .await
            .as_ref()
            .map(PubSubHandle::connection_ids)
            .unwrap_or_default()
    }
}

impl<V> Cache<V>
where
    V: Clone + Send + Sync + CachedValue + 'static,
{
    /// Insert a value using the cache-wide default TTL.
    pub async fn insert(&self, key: String, value: V) -> Result<(), Error> {
        self.insert_with_ttl(key, value, None).await
    }

    /// Insert a value with an explicit TTL. `None` means use the default TTL.
    pub async fn insert_with_ttl(&self, key: String, value: V, ttl: Option<Duration>) -> Result<(), Error> {
        let effective_ttl = ttl.unwrap_or(self.inner.default_ttl);

        let allow_l1_insert = if let Some(redis) = self.inner.redis.as_ref() {
            self.tombstone_guarded_l2_insert(redis, &key, &value, effective_ttl)
                .await?
        } else {
            true
        };

        if allow_l1_insert {
            self.insert_l1_with_ttl(key, value, effective_ttl).await;
        }

        Ok(())
    }

    /// Insert into the L2 tier only, using the cache-wide default TTL. Returns
    /// `Ok(false)` when L2 is disabled or a tombstone blocks the write.
    pub async fn insert_l2(&self, key: String, value: V) -> Result<bool, Error> {
        let Some(redis) = self.inner.redis.as_ref() else {
            return Ok(false);
        };

        self.tombstone_guarded_l2_insert(redis, &key, &value, self.inner.default_ttl)
            .await
    }

    /// Invalidate a single key across both cache levels.
    ///
    /// L2 first (tombstone-plant → broadcast), then **always** bump the local
    /// generation and clear L1 — regardless of the L2 outcome — returning any L2
    /// error afterward. Local correctness must not depend on Redis: a caller that
    /// observed a successful mutation must never keep serving the old value from
    /// this process's L1 just because the L2 hop failed (callers like
    /// `LayerCache::invalidate` only log the error and never retry). This mirrors
    /// `invalidate_prefix`. The generation bump precedes the L1 op so a concurrent
    /// L2 backfill in this process re-checks and skips.
    pub async fn invalidate(&self, key: &str) -> Result<(), Error> {
        let mut l2_error = None;
        if let Some(redis) = self.inner.redis.as_ref() {
            let tombstone_ttl_ms: i64 = self.inner.tombstone_ttl.as_millis().try_into().unwrap_or(i64::MAX);
            let start = Instant::now();
            let invalidated: Result<i64, Error> = self
                .inner
                .scripts
                .invalidate_key
                .evalsha_with_reload(
                    redis.next_connected(),
                    vec![self.data_key(key), self.lease_key(key), self.tombstone_key(key)],
                    vec![Value::Integer(tombstone_ttl_ms)],
                )
                .await
                .map_err(Into::into);
            match invalidated {
                Err(e) => {
                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                    l2_error = Some(e);
                },
                Ok(_) => {
                    let msg = InvalidationMessage::Key { key: key.to_owned() };
                    match msg.encode() {
                        Err(e) => {
                            self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                            self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                            l2_error = Some(e.into());
                        },
                        Ok(payload) => {
                            let published: Result<Value, Error> = redis
                                .next_connected()
                                .publish(self.invalidation_channel(), Value::Bytes(payload.into()))
                                .await
                                .map_err(Into::into);
                            match published {
                                Err(e) => {
                                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                                    l2_error = Some(e);
                                },
                                Ok(_) => {
                                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Hit);
                                },
                            }
                        },
                    }
                },
            }
        }

        // Maintain the L2 distrust marker (only meaningful when an L2 tier exists). On failure,
        // Redis may still hold the old value with no tombstone, so distrust it; on success the data
        // was deleted and a tombstone planted, so trust can be restored immediately.
        if self.inner.redis.is_some() {
            if l2_error.is_some() {
                self.inner.local_l2_distrust.insert(key.to_owned(), ()).await;
            } else {
                self.inner.local_l2_distrust.invalidate(key).await;
            }
        }

        // Always clear local L1, independent of the L2 result above.
        self.inner.local_generation.fetch_add(1, Ordering::Release);
        self.inner.l1.invalidate(key).await;

        match l2_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Invalidate all L1 entries whose key starts with `prefix`. Local L1 is
    /// updated synchronously (via moka's `invalidate_entries_if`); peer
    /// instances receive the same starts-with sweep via the pub/sub channel.
    ///
    /// Matching L2 data entries are evicted before the pub/sub broadcast so
    /// peers cannot repopulate L1 from stale Redis values after observing the
    /// invalidation.
    pub async fn invalidate_prefix(&self, prefix: &str) -> Result<(), Error> {
        let mut l2_error = None;
        if let Some(redis) = self.inner.redis.as_ref() {
            let start = Instant::now();
            match self.delete_l2_prefix(redis, prefix).await {
                Ok(_) => {
                    let msg = InvalidationMessage::Prefix {
                        prefix: prefix.to_owned(),
                    };
                    match msg.encode() {
                        Ok(payload) => {
                            let published: Result<Value, Error> = redis
                                .next_connected()
                                .publish(self.invalidation_channel(), Value::Bytes(payload.into()))
                                .await
                                .map_err(Into::into);
                            match published {
                                Ok(_) => {
                                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Hit);
                                },
                                Err(e) => {
                                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                                    l2_error = Some(e);
                                },
                            }
                        },
                        Err(e) => {
                            self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                            self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                            l2_error = Some(e.into());
                        },
                    }
                },
                Err(e) => {
                    self.record_l2_latency(L2Operation::Invalidate, start.elapsed());
                    self.record_l2_operation(L2Operation::Invalidate, L2OperationResult::Error);
                    l2_error = Some(e);
                },
            }
        }

        self.inner.local_generation.fetch_add(1, Ordering::Release);
        let prefix_owned = prefix.to_owned();
        if let Err(e) = self
            .inner
            .l1
            .invalidate_entries_if(move |k, _| k.starts_with(&prefix_owned))
        {
            warn_limited!(error = %e, "moka invalidate_entries_if predicate cap exceeded");
        }

        match l2_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Look up a value across L1 and L2 without running an init. Returns
    /// `None` when neither tier has a fresh entry. Callers that want a
    /// populating fetch should use [`Cache::try_get_with_by_ref`] instead.
    pub async fn peek_l2_or_l1(&self, key: &str) -> Result<Option<V>, Error> {
        if let Some(entry) = self.inner.l1.get(key).await {
            return Ok(Some(entry.value));
        }
        let Some(redis) = self.inner.redis.as_ref() else {
            return Ok(None);
        };
        // Probe the pooled connection that will actually serve the read.
        // `Pool::is_connected()` is opaque about which connection it inspects;
        // `next_connected()` checks (and selects) a real pooled client so an
        // all-down pool fast-fails here instead of waiting out the command
        // timeout on a doomed request.
        if !redis.next_connected().is_connected() {
            self.record_l2_operation(L2Operation::Read, L2OperationResult::Error);
            warn_limited!(key = %key, "L2 client is disconnected; treating as miss");
            return Ok(None);
        }
        // Distrusted key (a prior invalidate's L2 hop failed): don't surface a possibly-stale L2
        // value. Report a miss so the caller refills from the source of truth.
        if self.inner.local_l2_distrust.get(key).await.is_some() {
            return Ok(None);
        }
        let gen_before = self.inner.local_generation.load(Ordering::Acquire);
        match self.l2_read(redis, key).await {
            Ok(Some(entry)) => {
                let gen_after = self.inner.local_generation.load(Ordering::Acquire);
                if gen_after == gen_before {
                    self.inner
                        .l1
                        .insert(
                            key.to_owned(),
                            CacheEntry {
                                value: entry.value.clone(),
                                ttl: entry.ttl,
                                expected_local_generation: Some(gen_after),
                            },
                        )
                        .await;
                }
                Ok(Some(entry.value))
            },
            Ok(None) => Ok(None),
            Err(e) => {
                warn_limited!(error = %e, key = %key, "L2 read failed; treating as miss");
                Ok(None)
            },
        }
    }

    /// Look up a value, populating it via `init` on a miss.
    ///
    /// Concurrent misses for the same key are coalesced across the complete
    /// L1/L2/fill path within this cache instance (and therefore this process),
    /// including while Redis is unavailable. The Redis lease coordinates L2
    /// write-back across processes; it does not make backend fills globally
    /// single-flight.
    pub async fn try_get_with_by_ref<F, E>(&self, key: &str, init: F) -> Result<V, Arc<E>>
    where
        F: Future<Output = Result<V, E>>,
        E: From<Error> + Send + Sync + 'static,
    {
        let default_ttl = self.inner.default_ttl;

        let Some(redis) = self.inner.redis.as_ref() else {
            // Snapshot the generation before the (single-flight) fill. Moka inserts the init
            // result unconditionally, so an invalidate() that lands while this fill is in
            // flight would otherwise leave a stale value pinned in L1. If the generation
            // advanced during the fill we evict the key so the next read refetches. This
            // mirrors the generation guard on the L2-backfill path below.
            let gen_before = self.inner.local_generation.load(Ordering::Acquire);
            let result = self
                .inner
                .l1
                .try_get_with_by_ref(key, async move {
                    let value = init.await?;
                    Ok::<_, E>(CacheEntry {
                        value,
                        ttl: Some(default_ttl),
                        expected_local_generation: Some(gen_before),
                    })
                })
                .await
                .map(|entry| entry.value);
            if self.inner.local_generation.load(Ordering::Acquire) != gen_before {
                self.inner.l1.invalidate(key).await;
            }
            return result;
        };

        // Keep the complete L2 path behind moka's per-key initializer. Besides
        // ordinary connected Redis misses, this coalesces local callers while
        // Redis is disconnected or a command fails. A zero-TTL CacheEntry is a
        // transient result: moka shares it with callers already enrolled in the
        // initializer's waiter set, but it is not retained for a later lookup.
        self.inner
            .l1
            .try_get_with_by_ref(
                key,
                Box::pin(async move { self.fill_from_l2_or_init(redis, key, init, default_ttl).await }),
            )
            .await
            .map(|entry| entry.value)
    }

    async fn fill_from_l2_or_init<F, E>(
        &self,
        redis: &Pool,
        key: &str,
        init: F,
        default_ttl: Duration,
    ) -> Result<CacheEntry<V>, E>
    where
        F: Future<Output = Result<V, E>>,
    {
        // Probe the pooled connection that will actually serve the read (see
        // `peek_l2_or_l1`): fast-fail to the backend when the whole pool is
        // down rather than waiting out the per-command timeout.
        if !redis.next_connected().is_connected() {
            self.record_l2_operation(L2Operation::Read, L2OperationResult::Error);
            warn_limited!(key = %key, "L2 client is disconnected; falling back to backend");
            let value = init.await?;
            return Ok(Self::transient_entry(value));
        }

        // A prior invalidate could not be confirmed against L2, so Redis may still hold a stale
        // value with no tombstone. Do not trust L2 for this key: refill from the source of truth
        // and overwrite L1, so a stale L2 hit can't repopulate it until the data TTL elapses.
        if self.inner.local_l2_distrust.get(key).await.is_some() {
            self.record_l2_operation(L2Operation::Read, L2OperationResult::Miss);
            // Snapshot the generation before the (possibly slow) source refill. If an invalidate
            // lands mid-flight, skip retaining the value so we don't pin one that raced an
            // invalidation — mirrors the L2-backfill guard below. Do NOT bump the generation here:
            // this is a read path, and an unrelated bump would spuriously force concurrent fills to
            // skip their own legitimate L1 inserts.
            let gen_before = self.inner.local_generation.load(Ordering::Acquire);
            let value = init.await?;
            return Ok(CacheEntry {
                value,
                ttl: Some(default_ttl),
                expected_local_generation: Some(gen_before),
            });
        }

        // Snapshot generation before the L2 read so a pubsub invalidation that
        // arrives mid-flight forces us to skip the L1 backfill.
        let gen_before = self.inner.local_generation.load(Ordering::Acquire);

        // L2 read. Errors fail open: a Redis outage must not turn the cache
        // into a hard dependency; treat as a miss + skip lease/write-back.
        let l2_healthy = match self.l2_read(redis, key).await {
            Ok(Some(entry)) => {
                let gen_after = self.inner.local_generation.load(Ordering::Acquire);
                return if gen_after == gen_before {
                    Ok(CacheEntry {
                        value: entry.value,
                        ttl: entry.ttl,
                        expected_local_generation: Some(gen_after),
                    })
                } else {
                    Ok(Self::transient_entry(entry.value))
                };
            },
            Ok(None) => true,
            Err(e) => {
                warn_limited!(error = %e, key = %key, "L2 read failed; falling back to backend");
                false
            },
        };

        let token: u64 = rand::random();
        let lease_acquired = if l2_healthy {
            match self.lease_acquire(redis, key, token).await {
                Ok(v) => v,
                Err(e) => {
                    warn_limited!(error = %e, key = %key, "lease acquire failed");
                    false
                },
            }
        } else {
            false
        };

        let mut lease_release_guard = lease_acquired.then(|| {
            LeaseReleaseGuard::new(
                redis.clone(),
                self.inner.scripts.lease_release.clone(),
                self.lease_key(key),
                token,
            )
        });

        let value = init.await?;

        // Re-check before caching: an invalidate may have landed *during* this (possibly slow)
        // init. If that invalidate's L2 hop failed, the key is now distrusted — Redis may still
        // hold the old value with no tombstone and no deleted lease, so this fill could otherwise
        // write its (now-stale) value straight back into L2/L1. Skip the write-back and L1 insert;
        // the callers in this cohort still get this fill's value, but the next read refills from
        // source. We key off the per-key distrust marker rather than the global generation because
        // init can be slow and the global generation is bumped by unrelated fills, which would
        // cause spurious cache skips under concurrency. A *successful* invalidate mid-fill is
        // instead caught by the tombstone guard inside `l2_write` (which returns `Ok(false)`).
        let distrusted_mid_fill = self.inner.local_l2_distrust.get(key).await.is_some();

        if lease_acquired && !distrusted_mid_fill {
            // Keep this snapshot immediately adjacent to the CAS write. If any
            // local invalidation lands during or after the Redis operation, the
            // compare-exchange fails or insertion-time expiry makes the entry
            // transient. Unrelated operations can only suppress caching during
            // this narrow Redis command window, not during the backend init.
            let generation_before_write = self.inner.local_generation.load(Ordering::Acquire);
            match self.l2_write(redis, key, &value, token, default_ttl).await {
                Ok(true) => {
                    lease_release_guard
                        .as_mut()
                        .expect("lease guard exists when lease was acquired")
                        .disarm();
                    let published_generation = generation_before_write.wrapping_add(1);
                    if self
                        .inner
                        .local_generation
                        .compare_exchange(
                            generation_before_write,
                            published_generation,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                        || self.inner.local_l2_distrust.get(key).await.is_some()
                    {
                        trace_limited!(key = %key, "local generation changed during L2 write; value not retained in L1");
                        return Ok(Self::transient_entry(value));
                    }
                    return Ok(CacheEntry {
                        value,
                        ttl: Some(default_ttl),
                        expected_local_generation: Some(published_generation),
                    });
                },
                Ok(false) => {
                    lease_release_guard
                        .as_mut()
                        .expect("lease guard exists when lease was acquired")
                        .disarm();
                    warn_limited!(key = %key, "lost lease during fill; value not cached");
                },
                Err(e) => {
                    warn_limited!(error = %e, key = %key, "L2 write-back failed; value not cached");
                },
            }
        } else if distrusted_mid_fill {
            trace_limited!(key = %key, "key was invalidated (distrusted) during fill; not caching this value");
        }

        Ok(Self::transient_entry(value))
    }

    fn transient_entry(value: V) -> CacheEntry<V> {
        CacheEntry {
            value,
            ttl: Some(Duration::ZERO),
            expected_local_generation: None,
        }
    }

    // --- internal L2 helpers ---

    async fn l2_read(&self, redis: &Pool, key: &str) -> Result<Option<CacheEntry<V>>, Error> {
        let start = Instant::now();
        let result: Result<Value, Error> = self
            .inner
            .scripts
            .l2_read
            .evalsha_with_reload(redis.next_connected(), vec![self.data_key(key)], Vec::<Value>::new())
            .await
            .map_err(Into::into);
        let result = match result {
            Ok(result) => {
                self.record_l2_latency(L2Operation::Read, start.elapsed());
                result
            },
            Err(e) => {
                self.record_l2_latency(L2Operation::Read, start.elapsed());
                self.record_l2_operation(L2Operation::Read, L2OperationResult::Error);
                return Err(e);
            },
        };

        let entry = match result {
            Value::Array(items) if items.len() == 2 => {
                let (data_v, pttl_v) = {
                    let mut iter = items.into_iter();
                    (iter.next().unwrap(), iter.next().unwrap())
                };
                let bytes: Vec<u8> = match data_v.convert::<Vec<u8>>() {
                    Ok(v) => v,
                    Err(_) => return self.finish_l2_read_miss(),
                };
                let pttl: i64 = match pttl_v.convert() {
                    Ok(v) => v,
                    Err(_) => return self.finish_l2_read_miss(),
                };
                let envelope: Envelope<V> = match postcard::from_bytes(&bytes) {
                    Ok(env) => env,
                    Err(e) => {
                        warn_limited!(error = %e, key = %key, "L2 envelope decode failed; treating as miss");
                        return self.finish_l2_read_miss();
                    },
                };
                if envelope.schema != V::SCHEMA {
                    warn_limited!(
                        got = envelope.schema,
                        expected = V::SCHEMA,
                        key = %key,
                        "L2 schema mismatch; treating as miss"
                    );
                    return self.finish_l2_read_miss();
                }
                let remaining = if pttl > 0 {
                    Some(Duration::from_millis(pttl as u64))
                } else {
                    None
                };
                Some(CacheEntry {
                    value: envelope.value,
                    ttl: remaining,
                    expected_local_generation: None,
                })
            },
            _ => None,
        };

        self.record_l2_operation(
            L2Operation::Read,
            if entry.is_some() {
                L2OperationResult::Hit
            } else {
                L2OperationResult::Miss
            },
        );

        Ok(entry)
    }

    async fn delete_l2_prefix(&self, redis: &Pool, prefix: &str) -> Result<u64, Error> {
        // SCAN is a cursor that must stay on one connection across pages, so
        // pin a single pooled client for the whole scan + the unlinks it drives,
        // and gate on that exact connection's health: an all-down pool hands
        // back a disconnected client here and fast-fails the invalidation
        // instead of waiting out the per-command timeout.
        let client = redis.next_connected();
        if !client.is_connected() {
            return Err(fred::error::Error::new(
                fred::error::ErrorKind::Canceled,
                "redis client is disconnected before prefix invalidation",
            )
            .into());
        }

        let pattern = self.data_key_prefix_pattern(prefix);
        let mut deleted = 0u64;
        let mut scan_stream = client.scan(pattern, Some(PREFIX_INVALIDATE_SCAN_COUNT), None);

        while let Some(mut page) = self.await_l2_command(redis, scan_stream.try_next()).await? {
            if let Some(keys) = page.take_results() {
                if !keys.is_empty() {
                    match self.await_l2_command(redis, client.unlink(keys)).await {
                        Ok(count) => {
                            deleted = deleted.saturating_add(count);
                        },
                        Err(e) => {
                            page.cancel();
                            return Err(e);
                        },
                    }
                }
            }
            page.next();
        }

        Ok(deleted)
    }

    async fn await_l2_command<T, F>(&self, redis: &Pool, command: F) -> Result<T, Error>
    where
        F: Future<Output = Result<T, fred::error::Error>>,
    {
        let timeout = redis.perf_config().default_command_timeout;
        if timeout.is_zero() {
            return command.await.map_err(Into::into);
        }

        tokio::time::timeout(timeout, command)
            .await
            .map_err(|_| Error::Timeout {
                timeout_ms: duration_millis_u64(timeout),
            })?
            .map_err(Into::into)
    }

    fn finish_l2_read_miss(&self) -> Result<Option<CacheEntry<V>>, Error> {
        self.record_l2_operation(L2Operation::Read, L2OperationResult::Miss);
        Ok(None)
    }

    async fn lease_acquire(&self, redis: &Pool, key: &str, token: u64) -> Result<bool, Error> {
        let lease_ttl_ms: i64 = self.inner.lease_ttl.as_millis().try_into().unwrap_or(i64::MAX);
        let start = Instant::now();
        let result: Result<i64, Error> = self
            .inner
            .scripts
            .lease_acquire
            .evalsha_with_reload(
                redis.next_connected(),
                vec![self.lease_key(key)],
                vec![Value::Integer(token as i64), Value::Integer(lease_ttl_ms)],
            )
            .await
            .map_err(Into::into);
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                self.record_l2_latency(L2Operation::Write, start.elapsed());
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e);
            },
        };
        Ok(result == 1)
    }

    async fn tombstone_guarded_l2_insert(
        &self,
        redis: &Pool,
        key: &str,
        value: &V,
        ttl: Duration,
    ) -> Result<bool, Error> {
        let ttl_ms: i64 = ttl.as_millis().try_into().unwrap_or(i64::MAX);
        let envelope = Envelope {
            schema: V::SCHEMA,
            value: value.clone(),
        };
        let bytes = match postcard::to_allocvec(&envelope) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e.into());
            },
        };

        let start = Instant::now();
        let result: Result<i64, Error> = self
            .inner
            .scripts
            .tombstone_guarded_insert
            .evalsha_with_reload(
                redis.next_connected(),
                vec![self.data_key(key), self.tombstone_key(key)],
                vec![Value::Bytes(bytes.into()), Value::Integer(ttl_ms)],
            )
            .await
            .map_err(Into::into);
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                self.record_l2_latency(L2Operation::Write, start.elapsed());
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e);
            },
        };

        // Broadcast on every successful insert so peer L1s drop any stale
        // entry they may have for this key. Without this, a write that
        // refreshed the value would leave peers serving the previous L1
        // entry until its TTL expired.
        if result == 1 {
            let msg = InvalidationMessage::Key { key: key.to_owned() };
            let payload = match msg.encode() {
                Ok(payload) => payload,
                Err(e) => {
                    self.record_l2_latency(L2Operation::Write, start.elapsed());
                    self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                    return Err(e.into());
                },
            };
            let published: Result<Value, Error> = redis
                .next_connected()
                .publish(self.invalidation_channel(), Value::Bytes(payload.into()))
                .await
                .map_err(Into::into);
            if let Err(e) = published {
                self.record_l2_latency(L2Operation::Write, start.elapsed());
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e);
            }
        } else if let Some(metrics) = self.inner.l2_metrics.as_ref() {
            metrics.record_tombstone_blocked();
        }

        self.record_l2_latency(L2Operation::Write, start.elapsed());
        self.record_l2_operation(
            L2Operation::Write,
            if result == 1 {
                L2OperationResult::Hit
            } else {
                L2OperationResult::Miss
            },
        );

        Ok(result == 1)
    }

    async fn l2_write(&self, redis: &Pool, key: &str, value: &V, token: u64, ttl: Duration) -> Result<bool, Error> {
        let ttl_ms: i64 = ttl.as_millis().try_into().unwrap_or(i64::MAX);

        let envelope = Envelope {
            schema: V::SCHEMA,
            value: value.clone(),
        };
        let bytes = match postcard::to_allocvec(&envelope) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e.into());
            },
        };

        let start = Instant::now();
        let result: Result<i64, Error> = self
            .inner
            .scripts
            .lease_cas_write
            .evalsha_with_reload(
                redis.next_connected(),
                vec![self.data_key(key), self.lease_key(key)],
                vec![
                    Value::Integer(token as i64),
                    Value::Bytes(bytes.into()),
                    Value::Integer(ttl_ms),
                ],
            )
            .await
            .map_err(Into::into);
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                self.record_l2_latency(L2Operation::Write, start.elapsed());
                self.record_l2_operation(L2Operation::Write, L2OperationResult::Error);
                return Err(e);
            },
        };

        self.record_l2_latency(L2Operation::Write, start.elapsed());
        self.record_l2_operation(
            L2Operation::Write,
            if result == 1 {
                L2OperationResult::Hit
            } else {
                L2OperationResult::Miss
            },
        );
        if result != 1 {
            if let Some(metrics) = self.inner.l2_metrics.as_ref() {
                metrics.record_lost_lease();
            }
        }
        Ok(result == 1)
    }
}

// --- Builder ---

pub struct CacheBuilder {
    namespace: String,
    default_ttl: Duration,
    max_l1_capacity_bytes: u64,
    tombstone_ttl: Option<Duration>,
    lease_ttl: Option<Duration>,
    l2_metrics: Option<L2Metrics>,
}

impl CacheBuilder {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            default_ttl: Duration::from_secs(300),
            max_l1_capacity_bytes: 32 * 1024 * 1024,
            tombstone_ttl: None,
            lease_ttl: None,
            l2_metrics: None,
        }
    }

    pub fn default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }

    /// Set the byte budget for the L1 cache. The string key size plus the
    /// configured `V::cached_size` determine per-entry weight; total weighted
    /// size is bounded by this.
    pub fn max_l1_capacity_bytes(mut self, bytes: u64) -> Self {
        self.max_l1_capacity_bytes = bytes;
        self
    }

    pub fn tombstone_ttl(mut self, ttl: Duration) -> Self {
        self.tombstone_ttl = Some(ttl);
        self
    }

    pub fn lease_ttl(mut self, ttl: Duration) -> Self {
        self.lease_ttl = Some(ttl);
        self
    }

    pub(crate) fn l2_metrics(mut self, metrics: L2Metrics) -> Self {
        self.l2_metrics = Some(metrics);
        self
    }

    /// Default tombstone TTL: matches `default_ttl` so a tombstone outlives
    /// the longest possible init. The 10s default in earlier revisions was
    /// shorter than typical fills and let stale data resurrect.
    fn effective_tombstone_ttl(&self) -> Duration {
        self.tombstone_ttl.unwrap_or(self.default_ttl)
    }

    /// Default lease TTL: half the data TTL, floored at 30s. A 5s default was
    /// shorter than common init durations, so the leaseholder consistently
    /// lost the CAS for slow operations.
    fn effective_lease_ttl(&self) -> Duration {
        self.lease_ttl
            .unwrap_or_else(|| std::cmp::max(self.default_ttl / 2, Duration::from_secs(30)))
    }

    /// Build the cache. Pass `Some(client)` to enable the L2 (Redis/Valkey)
    /// tier; pass `None` for an L1-only cache.
    pub async fn build<V>(self, redis: Option<Pool>) -> Result<Cache<V>, Error>
    where
        V: Clone + Send + Sync + CachedSize + 'static,
    {
        if redis.is_some() && self.namespace.is_empty() {
            return Err(Error::EmptyNamespace);
        }

        let tombstone_ttl = self.effective_tombstone_ttl();
        let lease_ttl = self.effective_lease_ttl();

        let local_generation = Arc::new(AtomicU64::new(0));

        let l1 = moka::future::Cache::builder()
            .max_capacity(self.max_l1_capacity_bytes)
            .weigher(weight_of::<V>)
            .eviction_policy(moka::policy::EvictionPolicy::tiny_lfu())
            .support_invalidation_closures()
            .expire_after(PerEntryExpiry {
                default_ttl: self.default_ttl,
                local_generation: Arc::clone(&local_generation),
            })
            .build();

        // Distrust markers live at most as long as the stale L2 value they guard against (the data
        // TTL), after which that value has expired from Redis anyway.
        let local_l2_distrust = moka::future::Cache::builder()
            .max_capacity(L2_DISTRUST_MAX_KEYS)
            .time_to_live(self.default_ttl)
            .build();

        let pubsub_handle = if let Some(client) = redis.as_ref() {
            let channel = format!("{}:invalidate", self.namespace);
            let pubsub_metrics = self.l2_metrics.as_ref().map(L2Metrics::pubsub_metrics);
            Some(
                pubsub::spawn_subscriber(
                    client.next(),
                    channel,
                    l1.clone(),
                    Arc::clone(&local_generation),
                    pubsub_metrics,
                )
                .await?,
            )
        } else {
            None
        };

        let inner = Inner {
            l1,
            redis,
            scripts: Scripts::new(),
            namespace: self.namespace,
            default_ttl: self.default_ttl,
            tombstone_ttl,
            lease_ttl,
            pubsub: tokio::sync::Mutex::new(pubsub_handle),
            local_generation,
            local_l2_distrust,
            l2_metrics: self.l2_metrics,
        };

        Ok(Cache { inner: Arc::new(inner) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::time::Duration;

    use fred::interfaces::{ClientInterface, ClientLike, KeysInterface};
    use fred::types::client::ClientKillFilter;
    use tokio::sync::Notify;

    use super::*;
    use crate::cache::redis_config::{build_client, build_client_with_config, RedisConfig};

    async fn build_l1_cache() -> Cache<String> {
        CacheBuilder::new("ns")
            .build::<String>(None)
            .await
            .expect("L1-only build")
    }

    #[test]
    fn redis_glob_escape_escapes_match_metacharacters() {
        assert_eq!(redis_glob_escape(r"a*?[b]\c"), r"a\*\?\[b\]\\c");
    }

    #[tokio::test]
    async fn l1_weight_includes_key_and_value_size() {
        let cache: Cache<String> = CacheBuilder::new("ns")
            .max_l1_capacity_bytes(1024)
            .build(None)
            .await
            .expect("L1-only build");

        let key = "entry-key".to_string();
        let value = "cached-value".to_string();
        let expected_weight = u64::try_from(key.len() + value.len()).expect("test strings fit in u64");

        cache.insert_l1(key, value).await;
        cache.run_pending_l1_tasks().await;

        assert_eq!(cache.l1_weighted_size(), expected_weight);
    }

    /// Regression: an `invalidate()` that races an in-flight L1-only fill must
    /// not leave a stale value pinned in L1. Moka inserts the init result unconditionally, so
    /// `try_get_with_by_ref` evicts the key when the generation advanced during the fill.
    #[tokio::test]
    async fn l1_only_fill_does_not_survive_concurrent_invalidate() {
        let cache = Arc::new(build_l1_cache().await);

        let in_init = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        let task = {
            let cache = cache.clone();
            let in_init = in_init.clone();
            let release = release.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref("k", async move {
                        // Signal that the (stale) fill has started, then block until released
                        // so the invalidate below is guaranteed to land mid-flight.
                        in_init.notify_one();
                        release.notified().await;
                        Ok::<String, Error>("stale".to_string())
                    })
                    .await
            })
        };

        in_init.notified().await;
        cache.invalidate("k").await.expect("invalidate");
        release.notify_one();
        let _ = task.await.expect("fill task joined");

        assert!(
            cache.peek_l1("k").await.is_none(),
            "a fill that raced an invalidate must not pin a stale value in L1"
        );
    }

    /// Regression: generation validation must happen when moka actually
    /// publishes an entry, not only before the asynchronous insert begins.
    #[tokio::test]
    async fn stale_generation_is_not_retained_at_moka_publication() {
        let cache = build_l1_cache().await;
        let expected_generation = cache.local_generation();

        cache.inner.local_generation.fetch_add(1, Ordering::AcqRel);
        cache
            .inner
            .l1
            .insert(
                "k".to_string(),
                CacheEntry {
                    value: "stale".to_string(),
                    ttl: Some(Duration::from_secs(60)),
                    expected_local_generation: Some(expected_generation),
                },
            )
            .await;
        cache.run_pending_l1_tasks().await;

        assert!(
            cache.peek_l1("k").await.is_none(),
            "an entry whose generation went stale before publication must expire immediately"
        );
    }

    /// User keys ending in `__lease`/`__tomb` previously collided with the
    /// internal lease/tombstone keys for the unsuffixed sibling. Side keys
    /// now live in a marker-prefixed sub-namespace so this can never happen.
    #[tokio::test]
    async fn side_keys_do_not_collide_with_user_keys() {
        let cache = build_l1_cache().await;

        assert_ne!(cache.data_key("foo__lease"), cache.lease_key("foo"));
        assert_ne!(cache.data_key("foo__tomb"), cache.tombstone_key("foo"));

        let k = "foo";
        let keys = [cache.data_key(k), cache.lease_key(k), cache.tombstone_key(k)];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "internal keys {i} and {j} collide: {}", keys[i]);
            }
        }
    }

    /// L2 errors must fail open.
    #[tokio::test]
    async fn redis_l2_read_failure_falls_through_to_init() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_l2_failopen_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let v = cache
            .try_get_with_by_ref::<_, Error>("k", async move {
                calls_c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("first".to_string())
            })
            .await
            .expect("first call succeeds");
        assert_eq!(v, "first");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let _ = client.quit().await;

        let calls_c = calls.clone();
        let v = cache
            .try_get_with_by_ref::<_, Error>("k2", async move {
                calls_c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("second".to_string())
            })
            .await
            .expect("second call falls through to init despite L2 failure");
        assert_eq!(v, "second");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "init must run when L2 read fails");

        cache.shutdown().await;
    }

    /// Regression: an exact-key `invalidate()` must clear local L1 even when the L2
    /// (Redis) hop fails. Otherwise a content-key write/delete that succeeds against the backend
    /// could keep serving the stale parsed manifest from this process's L1 until TTL, because
    /// `LayerCache::invalidate` only logs the error and never retries.
    #[tokio::test]
    async fn invalidate_clears_local_l1_even_when_l2_fails() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_inval_l1_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");

        // Populate L1 (and L2) with a value.
        let v = cache
            .try_get_with_by_ref::<_, Error>("k", async { Ok::<_, Error>("v1".to_string()) })
            .await
            .expect("initial fill");
        assert_eq!(v, "v1");
        assert_eq!(
            cache.peek_l1("k").await,
            Some("v1".to_string()),
            "value should be in L1"
        );

        // Take down L2 so the invalidate's Redis hop fails.
        let _ = client.quit().await;

        // The invalidate surfaces the L2 error...
        assert!(
            cache.invalidate("k").await.is_err(),
            "invalidate should report the L2 failure"
        );
        // ...but local L1 must still be cleared.
        assert!(
            cache.peek_l1("k").await.is_none(),
            "local L1 must be cleared even when the L2 invalidation failed"
        );

        cache.shutdown().await;
    }

    /// Regression: after an exact-key invalidation fails against L2, Redis may still
    /// hold the old value with no tombstone. The key is marked distrusted so neither the populating
    /// read nor the peek path repopulates L1 from that stale L2 value — they refill from the source
    /// of truth instead. Reproduces the post-failed-invalidate state directly (L2 still holds the
    /// old value, L1 cleared, distrust marker set, Redis connected).
    #[tokio::test]
    async fn distrusted_key_refills_from_source_not_stale_l2() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_distrust_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client))
            .await
            .expect("L1+L2 build");

        // Fill k=v1 into both tiers.
        cache
            .try_get_with_by_ref::<_, Error>("k", async { Ok::<_, Error>("v1".to_string()) })
            .await
            .expect("initial fill");

        // Reproduce the state a failed invalidate leaves behind: L2 still holds v1, L1 is cleared,
        // and the key is distrusted (Redis remains connected, so the disconnected fast-path is not
        // what we are exercising here).
        cache.inner.local_l2_distrust.insert("k".to_string(), ()).await;
        cache.inner.local_generation.fetch_add(1, Ordering::Release);
        cache.inner.l1.invalidate("k").await;
        cache.inner.l1.run_pending_tasks().await;

        // The peek path must not surface the stale L2 value.
        assert_eq!(
            cache.peek_l2_or_l1("k").await.expect("peek"),
            None,
            "peek must report a miss for a distrusted key, not the stale L2 value"
        );

        // A populating read must refill from the source (v2), not the stale L2 (v1)...
        let v = cache
            .try_get_with_by_ref::<_, Error>("k", async { Ok::<_, Error>("v2".to_string()) })
            .await
            .expect("refill");
        assert_eq!(v, "v2", "distrusted read must use the source of truth, not stale L2");
        // ...and L1 should now hold the fresh value.
        assert_eq!(
            cache.peek_l1("k").await,
            Some("v2".to_string()),
            "L1 should be refilled with the fresh value"
        );

        cache.shutdown().await;
    }

    /// Regression: a populating fill that started — acquired its lease and began
    /// init — BEFORE a failed invalidate must not write its now-possibly-stale value back into
    /// L2/L1. The failed invalidate marks the key distrusted while the fill is mid-init; the fill
    /// re-checks that marker after init and skips caching. (The round-5 test only covered a marker
    /// set *after* the value was already cached; this covers the in-flight race.)
    #[tokio::test]
    async fn fill_in_flight_before_failed_invalidate_does_not_repopulate() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_inflight_distrust_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client))
            .await
            .expect("L1+L2 build");

        let in_init = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        // Start a cold fill that blocks inside init (after it has read L2-miss + acquired a lease).
        let task = {
            let cache = cache.clone();
            let in_init = in_init.clone();
            let release = release.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref::<_, Error>("k", async move {
                        in_init.notify_one();
                        release.notified().await;
                        Ok::<_, Error>("stale".to_string())
                    })
                    .await
            })
        };

        in_init.notified().await;
        // A mutation completed and its exact-key invalidate then FAILED against L2: that plants the
        // distrust marker (and clears L1) while the fill above is still inside init.
        cache.inner.local_l2_distrust.insert("k".to_string(), ()).await;
        cache.inner.local_generation.fetch_add(1, Ordering::Release);
        cache.inner.l1.invalidate("k").await;
        release.notify_one();

        let filled = task.await.expect("fill task joined").expect("fill");
        assert_eq!(filled, "stale", "the racing read still returns its own init value");

        // ...but that fill must NOT have repopulated L1 with the stale value.
        cache.inner.l1.run_pending_tasks().await;
        assert!(
            cache.peek_l1("k").await.is_none(),
            "a fill racing a failed invalidate must not repopulate L1"
        );

        cache.shutdown().await;
    }

    #[tokio::test]
    async fn l1_only_coalesces_concurrent_fills() {
        let cache = build_l1_cache().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let release_init = Arc::new(Notify::new());

        let calls_a = calls.clone();
        let release_init_a = release_init.clone();
        let first = cache.try_get_with_by_ref::<_, Error>("k", async move {
            calls_a.fetch_add(1, Ordering::SeqCst);
            release_init_a.notified().await;
            Ok::<_, Error>("v".to_string())
        });
        tokio::pin!(first);

        assert!(
            matches!(futures::poll!(&mut first), std::task::Poll::Pending),
            "first fill should block until the test releases it"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "first poll must start exactly one fill"
        );

        let calls_b = calls.clone();
        let second = cache.try_get_with_by_ref::<_, Error>("k", async move {
            calls_b.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Error>("unexpected second fill".to_string())
        });
        tokio::pin!(second);

        assert!(
            matches!(futures::poll!(&mut second), std::task::Poll::Pending),
            "second get should wait for the in-flight fill"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second get must coalesce without running its init"
        );

        release_init.notify_one();
        let (ra, rb) = tokio::join!(first, second);
        assert_eq!(ra.expect("first result"), "v");
        assert_eq!(rb.expect("second result"), "v");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "L1-only path must coalesce concurrent fills onto a single init"
        );
    }

    /// The L2 fail-open path represents a successful backend result as a
    /// zero-TTL entry. Moka must still deliver that transient value to callers
    /// already waiting on the initializer, while a later cohort sees a miss.
    #[tokio::test]
    async fn zero_ttl_entry_is_shared_with_waiters_but_not_retained() {
        let cache = build_l1_cache().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let release_init = Arc::new(Notify::new());

        let calls_a = calls.clone();
        let release_init_a = release_init.clone();
        let first = cache.inner.l1.try_get_with_by_ref("k", async move {
            calls_a.fetch_add(1, Ordering::SeqCst);
            release_init_a.notified().await;
            Ok::<_, Error>(CacheEntry {
                value: "v".to_string(),
                ttl: Some(Duration::ZERO),
                expected_local_generation: None,
            })
        });
        tokio::pin!(first);
        assert!(matches!(futures::poll!(&mut first), std::task::Poll::Pending));

        let calls_b = calls.clone();
        let second = cache.inner.l1.try_get_with_by_ref("k", async move {
            calls_b.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Error>(CacheEntry {
                value: "unexpected".to_string(),
                ttl: Some(Duration::ZERO),
                expected_local_generation: None,
            })
        });
        tokio::pin!(second);
        assert!(matches!(futures::poll!(&mut second), std::task::Poll::Pending));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release_init.notify_one();
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.expect("first result").value, "v");
        assert_eq!(second.expect("waiter result").value, "v");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.run_pending_l1_tasks().await;
        assert!(cache.peek_l1("k").await.is_none(), "zero-TTL entry must not persist");
    }

    #[tokio::test]
    async fn redis_l2_coalesces_concurrent_local_fills() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_l2_single_flight_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client))
            .await
            .expect("L1+L2 build");

        let calls = Arc::new(AtomicUsize::new(0));
        let init_started = Arc::new(Notify::new());
        let release_init = Arc::new(Notify::new());

        let first = {
            let cache = cache.clone();
            let calls = calls.clone();
            let init_started = init_started.clone();
            let release_init = release_init.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref::<_, Error>("k", async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        init_started.notify_one();
                        release_init.notified().await;
                        Ok::<_, Error>("v".to_string())
                    })
                    .await
            })
        };

        init_started.notified().await;
        let second = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref::<_, Error>("k", async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, Error>("unexpected second fill".to_string())
                    })
                    .await
            })
        };

        // Give the second task a chance to enroll behind moka's active
        // initializer before releasing it.
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release_init.notify_one();

        assert_eq!(first.await.expect("first join").expect("first fill"), "v");
        assert_eq!(second.await.expect("second join").expect("second fill"), "v");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "L2 fills must be single-flight locally"
        );

        cache.shutdown().await;
    }

    #[tokio::test]
    async fn disconnected_redis_still_coalesces_but_does_not_retain_fill() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_l2_disconnected_single_flight_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");
        let _ = client.quit().await;

        let calls = Arc::new(AtomicUsize::new(0));
        let release_init = Arc::new(Notify::new());

        let calls_a = calls.clone();
        let release_init_a = release_init.clone();
        let first = cache.try_get_with_by_ref::<_, Error>("k", async move {
            calls_a.fetch_add(1, Ordering::SeqCst);
            release_init_a.notified().await;
            Ok::<_, Error>("v".to_string())
        });
        tokio::pin!(first);
        assert!(matches!(futures::poll!(&mut first), std::task::Poll::Pending));

        let calls_b = calls.clone();
        let second = cache.try_get_with_by_ref::<_, Error>("k", async move {
            calls_b.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Error>("unexpected second fill".to_string())
        });
        tokio::pin!(second);
        assert!(matches!(futures::poll!(&mut second), std::task::Poll::Pending));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release_init.notify_one();
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.expect("first fill"), "v");
        assert_eq!(second.expect("second fill"), "v");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.run_pending_l1_tasks().await;
        assert!(
            cache.peek_l1("k").await.is_none(),
            "a fail-open fill must be transient when it could not be written to L2"
        );

        let calls_c = calls.clone();
        let next = cache
            .try_get_with_by_ref::<_, Error>("k", async move {
                calls_c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("next".to_string())
            })
            .await
            .expect("next cohort refills");
        assert_eq!(next, "next");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "a later cohort must retry the backend");

        cache.shutdown().await;
    }

    #[tokio::test]
    async fn redis_l2_waiter_takes_over_after_initializer_cancellation() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_l2_cancel_takeover_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");

        let calls = Arc::new(AtomicUsize::new(0));
        let init_started = Arc::new(Notify::new());
        let never_release = Arc::new(Notify::new());

        let leader = {
            let cache = cache.clone();
            let calls = calls.clone();
            let init_started = init_started.clone();
            let never_release = never_release.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref::<_, Error>("k", async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        init_started.notify_one();
                        never_release.notified().await;
                        Ok::<_, Error>("cancelled".to_string())
                    })
                    .await
            })
        };

        // Notification happens only after the leader has completed the L2
        // probe and acquired its lease, so cancellation exercises takeover
        // from inside the backend init rather than before single-flight starts.
        init_started.notified().await;
        let waiter = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .try_get_with_by_ref::<_, Error>("k", async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, Error>("takeover".to_string())
                    })
                    .await
            })
        };

        tokio::task::yield_now().await;
        leader.abort();
        assert!(leader.await.expect_err("leader should be cancelled").is_cancelled());

        let value = waiter.await.expect("waiter join").expect("waiter takeover");
        assert_eq!(value, "takeover");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "waiter must run its own init after cancellation"
        );

        // The cancelled leader's lease is released asynchronously. Wait only
        // for that cleanup task, not the configured lease TTL, then prove a
        // later cohort can retain a value (whether the waiter won the released
        // lease or this call performs the refill).
        let lease_key = cache.lease_key("k");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let lease: Option<String> = client.get(&lease_key).await.expect("read lease key");
                if lease.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled initializer lease should be released promptly");

        cache.run_pending_l1_tasks().await;
        let calls_before_later = calls.load(Ordering::SeqCst);
        let calls_later = calls.clone();
        let retained = cache
            .try_get_with_by_ref::<_, Error>("k", async move {
                calls_later.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("refill".to_string())
            })
            .await
            .expect("later cohort");
        let calls_after_later = calls.load(Ordering::SeqCst);
        assert!(
            calls_after_later == calls_before_later || calls_after_later == calls_before_later + 1,
            "later cohort either hits the waiter's retained value or performs one refill"
        );

        cache.run_pending_l1_tasks().await;
        assert_eq!(cache.peek_l1("k").await.as_deref(), Some(retained.as_str()));

        let calls_final = calls.clone();
        let final_value = cache
            .try_get_with_by_ref::<_, Error>("k", async move {
                calls_final.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("unexpected final refill".to_string())
            })
            .await
            .expect("retained read");
        assert_eq!(final_value, retained);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            calls_after_later,
            "the post-cancellation value must be retained"
        );

        cache.shutdown().await;
    }

    /// A fill whose lease was deleted by a concurrent `invalidate` must not
    /// be cached in L1.
    #[tokio::test]
    async fn redis_l2_lost_lease_does_not_poison_l1() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_lost_lease_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client))
            .await
            .expect("L1+L2 build");

        let init_started = Arc::new(Notify::new());
        let release_init = Arc::new(Notify::new());

        let cache_a = cache.clone();
        let init_started_a = init_started.clone();
        let release_init_a = release_init.clone();
        let task_a = tokio::spawn(async move {
            cache_a
                .try_get_with_by_ref::<_, Error>("k", async move {
                    init_started_a.notify_one();
                    release_init_a.notified().await;
                    Ok::<_, Error>("stale".to_string())
                })
                .await
        });

        init_started.notified().await;
        cache.invalidate("k").await.expect("invalidate");
        release_init.notify_one();

        let result = task_a.await.expect("task join").expect("try_get_with_by_ref");
        assert_eq!(result, "stale", "init's value should still be returned to its caller");

        assert!(
            cache.peek_l1("k").await.is_none(),
            "L1 must not contain a value whose lease was lost during the fill"
        );

        cache.shutdown().await;
    }

    /// Building with an empty namespace and a Redis client must fail — a
    /// missing namespace would let two services share a keyspace.
    #[tokio::test]
    async fn empty_namespace_rejected_when_redis_present() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client = build_client(&address).await.expect("connect to redis");
        let result = CacheBuilder::new("").build::<String>(Some(client)).await;
        assert!(matches!(result, Err(Error::EmptyNamespace)));
    }

    #[tokio::test]
    async fn redis_invalidate_prefix_deletes_matching_l2_entries() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_prefix_delete_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace.clone())
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");

        cache
            .insert("a/b/k1".to_string(), "stale".to_string())
            .await
            .expect("insert matching");
        cache
            .insert("a/c/k2".to_string(), "keep".to_string())
            .await
            .expect("insert sibling");

        let matching_key = format!("{namespace}:d:a/b/k1");
        let sibling_key = format!("{namespace}:d:a/c/k2");
        let before: Option<Vec<u8>> = client.get(&matching_key).await.expect("redis GET before invalidate");
        assert!(
            before.is_some(),
            "matching L2 entry should be present before invalidation"
        );

        cache.invalidate_prefix("a/b/").await.expect("invalidate prefix");

        let after: Option<Vec<u8>> = client.get(&matching_key).await.expect("redis GET after invalidate");
        assert!(after.is_none(), "prefix invalidation must delete matching L2 data");
        let sibling: Option<Vec<u8>> = client.get(&sibling_key).await.expect("redis GET sibling");
        assert!(sibling.is_some(), "prefix invalidation must leave sibling L2 data");

        cache.shutdown().await;
    }

    #[tokio::test]
    async fn invalidate_prefix_clears_local_l1_even_when_l2_fails() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client = build_client(&address).await.expect("connect to redis");
        let namespace = format!("test_prefix_l1_failopen_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client.clone()))
            .await
            .expect("L1+L2 build");

        cache.insert_l1("a/b/k".to_string(), "stale".to_string()).await;
        assert_eq!(cache.peek_l1("a/b/k").await.as_deref(), Some("stale"));

        let _ = client.quit().await;
        let result = cache.invalidate_prefix("a/b/").await;
        assert!(result.is_err(), "Redis failure should still be reported");
        assert!(
            cache.peek_l1("a/b/k").await.is_none(),
            "local L1 must be cleared even when Redis invalidation fails"
        );

        cache.shutdown().await;
    }

    /// Pub/sub invalidation from one Cache instance evicts a peer instance's
    /// L1 entry for the same key.
    #[tokio::test]
    async fn redis_pubsub_invalidate_evicts_peer_l1() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client_a = build_client(&address).await.expect("connect a");
        let client_b = build_client(&address).await.expect("connect b");
        let namespace = format!("test_pubsub_{}", rand::random::<u64>());

        let cache_a: Cache<String> = CacheBuilder::new(namespace.clone())
            .build(Some(client_a))
            .await
            .expect("a build");
        let cache_b: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client_b))
            .await
            .expect("b build");

        cache_a
            .insert("k".to_string(), "v".to_string())
            .await
            .expect("a insert");

        // Wait for cache_b to observe the value via L2.
        let mut hits = 0;
        for _ in 0..50 {
            if cache_b.peek_l2_or_l1("k").await.expect("peek b").is_some() {
                hits += 1;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(hits > 0, "b should see the value through L2");

        cache_b.invalidate("k").await.expect("b invalidate");

        // a's L1 should drop within the pubsub round-trip.
        let mut empty = false;
        for _ in 0..50 {
            if cache_a.peek_l1("k").await.is_none() {
                empty = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(empty, "a's L1 must drop after b's invalidate broadcasts");

        cache_a.shutdown().await;
        cache_b.shutdown().await;
    }

    /// Regression: when the pub/sub connection drops and reconnects, the
    /// `on_reconnect` handler must clear local L1 because invalidation messages may have been
    /// missed while disconnected.
    #[tokio::test]
    async fn redis_pubsub_reconnect_clears_l1() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };

        let cfg = RedisConfig {
            address,
            reconnect_min_delay_ms: 10,
            reconnect_max_delay_ms: 50,
            reconnect_jitter_ms: 0,
            ..RedisConfig::default()
        };
        let client = build_client_with_config(&cfg).await.expect("connect cache client");
        let admin = build_client_with_config(&cfg).await.expect("connect admin client");
        let namespace = format!("test_pubsub_reconnect_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .build(Some(client))
            .await
            .expect("L1+L2 build");

        cache.insert_l1("k".to_string(), "stale".to_string()).await;
        assert_eq!(cache.peek_l1("k").await.as_deref(), Some("stale"));
        let generation_before = cache.local_generation();

        let mut subscriber_ids = Vec::new();
        for _ in 0..50 {
            subscriber_ids = cache.pubsub_connection_ids().await;
            if !subscriber_ids.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !subscriber_ids.is_empty(),
            "subscriber should have a Redis connection id"
        );

        // Kill every subscriber connection (centralized Redis has exactly one; looping is future-proof
        // for clustered topologies). The `ID` filter form returns a count and never errors on a
        // no-match, so a racing self-reconnect that already dropped the id just yields 0 — the
        // reconnect poll below is the real assertion, not the kill count.
        for id in &subscriber_ids {
            let _killed: i64 = admin
                .client_kill(vec![
                    ClientKillFilter::ID(id.to_string()),
                    ClientKillFilter::SkipMe(true),
                ])
                .await
                .expect("CLIENT KILL");
        }

        let mut reconnected = false;
        for _ in 0..100 {
            if cache.local_generation() > generation_before && cache.peek_l1("k").await.is_none() {
                reconnected = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            reconnected,
            "pub/sub reconnect should bump generation and clear local L1"
        );

        let value = cache
            .try_get_with_by_ref::<_, Error>("k", async { Ok::<_, Error>("fresh".to_string()) })
            .await
            .expect("read refills after reconnect flush");
        assert_eq!(value, "fresh");
        assert_eq!(cache.peek_l1("k").await.as_deref(), Some("fresh"));

        cache.shutdown().await;
        let _ = admin.quit().await;
    }

    /// A tombstone planted by `invalidate` must block a subsequent `insert`
    /// inside its TTL window, so a racing fill cannot resurrect stale data.
    #[tokio::test]
    async fn redis_tombstone_blocks_writes_during_window() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client = build_client(&address).await.expect("connect");
        let namespace = format!("test_tombstone_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace)
            .default_ttl(Duration::from_secs(60))
            .tombstone_ttl(Duration::from_secs(30))
            .build(Some(client))
            .await
            .expect("build");

        cache.insert("k".to_string(), "v1".to_string()).await.expect("first");
        cache.invalidate("k").await.expect("invalidate");

        // Insert immediately after invalidate must be blocked — peek_l2_or_l1
        // returns None because the tombstone deleted the data and the new
        // insert was dropped.
        cache.insert("k".to_string(), "v2".to_string()).await.expect("second");
        let after = cache.peek_l1("k").await;
        assert!(after.is_none(), "L1 must stay empty when tombstone blocks insert");

        cache.shutdown().await;
    }

    /// A junk byte string at the data key must be treated as a miss without
    /// poisoning subsequent reads.
    #[tokio::test]
    async fn redis_l2_malformed_envelope_falls_back_to_init() {
        let Ok(address) = std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS") else {
            eprintln!("skipping: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
            return;
        };
        let client = build_client(&address).await.expect("connect");
        let namespace = format!("test_malformed_{}", rand::random::<u64>());
        let cache: Cache<String> = CacheBuilder::new(namespace.clone())
            .build(Some(client.clone()))
            .await
            .expect("build");

        let data_key = format!("{namespace}:d:k");
        let _: Value = client
            .set(data_key, b"junk".as_slice(), None, None, false)
            .await
            .expect("plant junk");

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let v = cache
            .try_get_with_by_ref::<_, Error>("k", async move {
                calls_c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>("init".to_string())
            })
            .await
            .expect("call");
        assert_eq!(v, "init");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.shutdown().await;
    }
}
