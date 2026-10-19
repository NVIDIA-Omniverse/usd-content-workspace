// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Small-object caching and same-key read coalescing.
//!
//! Complete bodies and metadata-only hints occupy separate cache keyspaces.
//! `b:{resource_identity}` stores a complete [`SmallObjectCacheEntry`] (metadata
//! plus bytes), while `m:{resource_identity}` stores metadata written by
//! `stat`, `list_stat`, or an uncacheable read. A metadata fill can therefore
//! hint a backend read but cannot satisfy or coalesce a body read.
//! Metadata-key writes are plain last-writer-wins upserts; they deliberately do
//! not merge with earlier entries or emit the former version/size consistency
//! warnings.
//!
//! Non-redirect body reads use [`LayerCache::try_get_with_by_ref_with_options`], backed by
//! Moka's same-key initialization, as their single-flight boundary. On a miss,
//! the selected initializer drains and validates a cacheable small body before
//! returning one complete entry; every coalesced caller receives that entry
//! directly. There is no separate leader election, replay stream, or
//! cross-task signalling.
//!
//! Redirects, large or unknown-size bodies, incomplete responses, and bodies
//! whose received size differs from their declared size are not cached. The
//! initializer reconstructs its already-read prefix plus the untouched stream
//! in a per-call stash and returns an [`Error::Uncacheable`] sentinel. The
//! sentinel is not retained by the cache: the caller whose initializer ran
//! takes its stashed response, while each coalesced caller still owns an
//! untouched backend closure and performs its own read. Explicit redirect
//! requests bypass this path entirely, so redirect URLs remain caller-specific.
//!
//! This changes when results become observable. A cacheable miss does not open
//! its returned stream until the candidate body reaches EOF, and a mid-stream
//! error encountered while draining it is returned by the read operation
//! instead of by the returned stream. Responses identified as uncacheable
//! early retain streaming passthrough behavior. Backend errors are shared only
//! with callers of the in-progress fill and are never retained. [`Config::errors`]
//! remains accepted as legacy configuration but has no effect in this layer.

use std::{
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use storage_core_util::cache::ApiVersion;

use bytes::{Bytes, BytesMut};
use bytesize::ByteSize;
use figment::Figment;
use fred::clients::Pool;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use storage_core_util::warn_limited;
use tokio::sync::Semaphore;
use tracing::instrument;
use url::Url;

use storage_core_builder::{
    layer::Layer,
    traits::{
        error::{Result, StorageError, StorageErrorExt},
        BoxStream,
    },
};
use storage_core_util::api_types::{ReadPart, ResourceInfo};
use storage_core_util::cache::{
    CacheErrorPolicy, CachedSize, CachedValue, LayerBuildError, LayerCache, LayerCacheArgs, RedisConfig,
    SecretsGeneration,
};

use crate::small_object_cache::error::{is_uncacheable, Error};

/// Derive the key for a complete body cache entry.
///
/// Body and metadata entries must live in disjoint keyspaces so a metadata-only
/// `stat`/`list_stat` fill can never short-circuit a body read or its coalescing key.
fn body_key(resource_identity: &str, version: ApiVersion) -> String {
    // Partition by API version: a v1beta fill (whose proto cannot carry native_metadata) must never
    // share a key with a v1alpha entry, or it would return an empty map on a v1alpha hit.
    format!("{}\u{1f}b:{resource_identity}", version.prefix())
}

/// Derive the key for a metadata-only cache entry. See [`body_key`] for why this is
/// separate from the body keyspace.
pub(super) fn meta_key(resource_identity: &str, version: ApiVersion) -> String {
    format!("{}\u{1f}m:{resource_identity}", version.prefix())
}

pub(super) fn canonical_identity(resource_identity: &str) -> Result<String> {
    let mut url = Url::parse(resource_identity).map_err(|e| Error::invalid_argument(e.to_string()))?;
    storage_core_util::url_path::canonicalize_url_path(&mut url).map_err(|e| Error::invalid_argument(e.to_string()))?;
    Ok(url.into())
}

pub(super) fn canonical_backend_identity(resource_identity: &str) -> Result<String> {
    let mut url = Url::parse(resource_identity)
        .map_err(|e| Error::internal(format!("backend returned an invalid resource identity: {e}")))?;
    storage_core_util::url_path::canonicalize_url_path(&mut url)
        .map_err(|e| Error::internal(format!("backend returned an invalid resource identity path: {e}")))?;
    Ok(url.into())
}

/// Download preference values that this layer reasons about. Mirrors the proto enum but
/// kept local so we don't depend on either v1alpha or v1beta here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedDownloadPreference {
    Unspecified,
    Body,
    Redirect,
}

impl From<i32> for CachedDownloadPreference {
    fn from(value: i32) -> Self {
        // Discriminants match the v1alpha/v1beta `DownloadPreference` proto enums.
        match value {
            1 => Self::Body,
            2 => Self::Redirect,
            _ => Self::Unspecified,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Enable the optional Redis L2 tier when a shared `[redis]` section and client are present.
    #[serde(default)]
    pub l2_enabled: bool,
    #[serde(default = "default_capacity")]
    pub capacity: ByteSize,
    #[serde(default = "default_time_to_live", with = "humantime_serde")]
    pub time_to_live: Duration,
    /// Legacy configuration retained for deserialization compatibility. The
    /// small-object cache never retains backend errors because its body key is
    /// shared across callers and the layer does not own mutation invalidation.
    #[serde(default = "default_errors")]
    pub errors: Vec<String>,
    #[serde(default = "default_small_object_size")]
    pub small_object_size: ByteSize,
    #[serde(default = "default_capacity_warn_pct")]
    pub capacity_warn_pct: u8,
    /// Maximum concurrent L2 cache writes. Bounds the spawned write fan-out so
    /// a burst of fills can't unbound-queue Redis writes on the runtime.
    #[serde(default = "default_l2_write_concurrency")]
    pub l2_write_concurrency: usize,
}

fn default_enabled() -> bool {
    true
}

fn default_capacity() -> ByteSize {
    ByteSize::gib(1)
}

fn default_time_to_live() -> Duration {
    Duration::from_secs(60 * 15)
}

fn default_errors() -> Vec<String> {
    vec!["AccessDenied".to_string(), "NotFound".to_string()]
}

fn default_small_object_size() -> ByteSize {
    ByteSize::kib(256)
}

fn default_capacity_warn_pct() -> u8 {
    storage_core_util::cache::default_capacity_warn_pct()
}

fn default_l2_write_concurrency() -> usize {
    64
}

impl Config {
    pub const CONFIG_KEY: &str = "small_object_cache";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            capacity: default_capacity(),
            time_to_live: default_time_to_live(),
            errors: default_errors(),
            small_object_size: default_small_object_size(),
            capacity_warn_pct: default_capacity_warn_pct(),
            l2_write_concurrency: default_l2_write_concurrency(),
            l2_enabled: false,
        }
    }
}

impl From<Config> for LayerCacheArgs {
    fn from(val: Config) -> Self {
        LayerCacheArgs {
            capacity: val.capacity,
            time_to_live: val.time_to_live,
            // This field used to be inert in the small-object cache. Keep that
            // safe behavior: retaining NotFound breaks read-after-create and
            // retaining AccessDenied can poison a shared key across callers.
            errors: Vec::new(),
            invalidate_on_update: false,
            capacity_warn_pct: val.capacity_warn_pct,
            tombstone_ttl: None,
            lease_ttl: None,
            l2_enabled: val.l2_enabled,
        }
    }
}

/// Layer for the small object cache.
pub struct SmallObjectCacheLayer {
    pub cache: Arc<LayerCache<SmallObjectCacheEntry>>,
    pub config: Config,
    pub l2_write_semaphore: Arc<Semaphore>,
}

/// Errors returned during construction of a `SmallObjectCacheLayer`.
pub type BuildError = LayerBuildError;

impl SmallObjectCacheLayer {
    pub const LAYER_NAME: &'static str = "small_object_cache";

    pub async fn new(
        config: Config,
        redis: Option<Pool>,
        namespace_prefix: Option<&str>,
    ) -> std::result::Result<Option<Self>, BuildError> {
        Self::new_with_generation(config, redis, namespace_prefix, None).await
    }

    /// Like [`new`](Self::new), but partitions every cache key by the given
    /// secrets-policy generation so a secrets hot-reload repartitions this tier.
    /// Pass `None` for the legacy (unpartitioned) behaviour.
    pub async fn new_with_generation(
        config: Config,
        redis: Option<Pool>,
        namespace_prefix: Option<&str>,
        generation: Option<SecretsGeneration>,
    ) -> std::result::Result<Option<Self>, BuildError> {
        if !config.enabled {
            return Ok(None);
        }
        let namespace = match namespace_prefix {
            Some(prefix) => format!("{prefix}:{}", Self::LAYER_NAME),
            None => Self::LAYER_NAME.to_owned(),
        };
        let l2_write_semaphore = Arc::new(Semaphore::new(config.l2_write_concurrency.max(1)));
        let args: LayerCacheArgs = config.clone().into();
        let cache = LayerCache::new(&args, Self::LAYER_NAME, namespace, redis, generation).await?;
        Ok(Some(Self {
            cache: Arc::new(cache),
            config,
            l2_write_semaphore,
        }))
    }

    pub async fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, BuildError> {
        Self::from_figment_with_redis(figment, None).await
    }

    pub async fn from_figment_with_redis(
        figment: &Figment,
        redis: Option<Pool>,
    ) -> std::result::Result<Option<Self>, BuildError> {
        Self::from_figment_with_redis_and_generation(figment, redis, None).await
    }

    /// Like [`from_figment_with_redis`](Self::from_figment_with_redis), but
    /// partitions every cache key by the given secrets-policy generation. Share
    /// one [`SecretsGeneration`] across all cache tiers and register it as a
    /// secrets subscriber so a hot-reload repartitions them together.
    pub async fn from_figment_with_redis_and_generation(
        figment: &Figment,
        redis: Option<Pool>,
        generation: Option<SecretsGeneration>,
    ) -> std::result::Result<Option<Self>, BuildError> {
        let config = Config::try_from(figment)?;
        let redis_cfg: Option<RedisConfig> = figment.focus(RedisConfig::CONFIG_KEY).extract().ok();
        let prefix = if config.l2_enabled && redis.is_some() {
            redis_cfg.as_ref().map(RedisConfig::required_namespace).transpose()?
        } else {
            redis_cfg.as_ref().and_then(RedisConfig::namespace_prefix)
        };
        let redis = redis.filter(|_| redis_cfg.is_some());
        Self::new_with_generation(config, redis, prefix, generation).await
    }
}

impl<S> Layer<S> for SmallObjectCacheLayer {
    type Service = SmallObjectCache<S>;
    fn layer(&self, inner: S) -> Self::Service {
        SmallObjectCache::new(
            inner,
            self.cache.clone(),
            self.config.clone(),
            self.l2_write_semaphore.clone(),
        )
    }
}

pub struct SmallObjectCache<S> {
    pub inner: Arc<S>,
    pub cache: Arc<LayerCache<SmallObjectCacheEntry>>,
    pub config: Config,
    pub l2_write_semaphore: Arc<Semaphore>,
}

impl<S> SmallObjectCache<S> {
    pub fn new(
        inner: S,
        cache: Arc<LayerCache<SmallObjectCacheEntry>>,
        config: Config,
        l2_write_semaphore: Arc<Semaphore>,
    ) -> Self {
        Self {
            inner: Arc::new(inner),
            cache,
            config,
            l2_write_semaphore,
        }
    }

    /// Read from the inner file object, consulting both L1 and L2 cache tiers.
    ///
    /// The body key is `b:{resource_identity}` and the metadata-hint key is
    /// `m:{resource_identity}`. An explicit `DownloadPreference::Redirect`
    /// bypasses both keys and calls `perform_read(None)` so a presigned URL is
    /// never shared. Otherwise, a body-key hit returns its complete metadata
    /// and bytes without consulting the metadata key. On a body miss, a
    /// metadata-key hit is passed to `perform_read` as a [`ResourceInfo`] hint;
    /// without one, the closure receives `None`.
    ///
    /// [`LayerCache::try_get_with_by_ref_with_options`] provides the body-key single flight.
    /// Its Moka initializer drains a candidate small response through EOF,
    /// verifies that it contains complete metadata and exactly the declared
    /// number of bytes, and returns a complete [`SmallObjectCacheEntry`]. Every
    /// coalesced caller returns that same entry directly. Consequently, opening
    /// a cacheable miss waits for the complete body, and an error encountered
    /// while draining the backend stream is returned from this method rather
    /// than later through the returned stream. Backend errors are shared with
    /// current callers but are never retained.
    ///
    /// An implicit redirect, a large or unknown-size body, incomplete metadata,
    /// or a declared/received size mismatch instead returns the private
    /// [`Error::Uncacheable`] sentinel from the initializer. Before doing so,
    /// the initializer puts every part it already consumed in this invocation's
    /// local stash, chained to the unconsumed remainder. The cache does not
    /// retain the sentinel. The caller whose initializer ran recovers that
    /// original response from its stash; coalesced callers have no stash but
    /// still own their untouched `perform_read` closure, so each performs a
    /// backend read of its own (using any metadata hint published by the first
    /// response). Thus uncacheable responses preserve per-caller content and
    /// stream after the point at which they were classified as uncacheable.
    /// `perform_read` remains one-shot, and its returned future must be `Send +
    /// 'static` because the boxed cache initializer owns it across await points.
    #[instrument(name = "small_object_cache::read_internal", skip_all)]
    pub(crate) async fn read_internal<F, Fut>(
        &self,
        resource_identity: String,
        version: ApiVersion,
        download_preference: i32,
        perform_read: F,
    ) -> Result<BoxStream<ReadPart>>
    where
        F: FnOnce(Option<ResourceInfo>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<BoxStream<ReadPart>>> + Send + 'static,
    {
        let preference = CachedDownloadPreference::from(download_preference);

        // Explicit Redirect: don't cache, don't coalesce. Presigned URLs are per-request.
        if preference == CachedDownloadPreference::Redirect {
            return perform_read(None).await;
        }

        // Pin every cache lookup and write performed by this request to one
        // secrets-policy generation. A hot reload during the backend read must
        // not publish old-policy data into the new partition.
        let generation = self.cache.generation_snapshot();

        // Each caller owns its closure and passthrough stash. Moka polls only one init
        // future for a body key, so only that caller consumes its closure. If the result
        // is uncacheable, the init runner finds its original stream in `stash`; every
        // waiter still has its own untouched closure and can fetch independently.
        let perform_read = Arc::new(Mutex::new(Some(perform_read)));
        let stash: Arc<Mutex<Option<BoxStream<ReadPart>>>> = Arc::new(Mutex::new(None));

        let init_perform_read = perform_read.clone();
        let init_stash = stash.clone();
        let cache = self.cache.clone();
        let l2_write_semaphore = self.l2_write_semaphore.clone();
        let small_object_size = self.config.small_object_size;
        let key = body_key(&resource_identity, version);
        let metadata_key = meta_key(&resource_identity, version);
        let init_metadata_key = metadata_key.clone();
        let init_resource_identity = resource_identity.clone();
        let init_generation = generation;

        // Erase the initializer future's deeply nested type before handing it to the
        // cache. Both file-object protocol adapters await this generic method from an
        // async-trait future; keeping the full initializer layout there can exceed
        // rustc's query-depth limit when every backend feature is enabled.
        let init: Pin<Box<dyn Future<Output = Result<SmallObjectCacheEntry>> + Send>> = Box::pin(async move {
            // Metadata-only entries can hint a backend fill, but can never satisfy a
            // body-key read. Keep this lookup inside the init so body hits do not
            // consult the metadata key at all.
            let metadata_hint = get_metadata_hint(&cache, &init_metadata_key, init_generation).await?;
            let perform_read =
                take_mutex_option(&init_perform_read, "small_object_cache: perform_read mutex poisoned")?.ok_or_else(
                    || Box::new(Error::internal("small_object_cache: perform_read already consumed")) as StorageError,
                )?;
            let mut stream = perform_read(metadata_hint).await?;
            let mut consumed = Vec::new();

            let Some(first) = stream.next().await else {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    empty_read_stream(),
                    &cache,
                    &l2_write_semaphore,
                    None,
                    init_generation,
                    version,
                )
                .await;
            };
            let first = first?;
            consumed.push(Ok(first.clone()));

            let ReadPart::ResourceInfo(resource_info) = first else {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    stream,
                    &cache,
                    &l2_write_semaphore,
                    None,
                    init_generation,
                    version,
                )
                .await;
            };

            let Some(declared_size) = resource_info.data_object_size else {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    stream,
                    &cache,
                    &l2_write_semaphore,
                    Some(&resource_info),
                    init_generation,
                    version,
                )
                .await;
            };
            if declared_size > small_object_size.as_u64() || !resource_info_is_complete(&resource_info) {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    stream,
                    &cache,
                    &l2_write_semaphore,
                    Some(&resource_info),
                    init_generation,
                    version,
                )
                .await;
            }

            // The cache fill is keyed by the caller's identity. Do not silently
            // alias a backend-rewritten identity onto that key: preserve the
            // response as passthrough and publish only its backend-keyed hint.
            if resource_info.encoded_identity != init_resource_identity {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    stream,
                    &cache,
                    &l2_write_semaphore,
                    Some(&resource_info),
                    init_generation,
                    version,
                )
                .await;
            }

            let mut chunks = Vec::new();
            let mut received_size = 0_u64;
            while let Some(part) = stream.next().await {
                let part = part?;
                consumed.push(Ok(part.clone()));
                match part {
                    ReadPart::Chunk(chunk) => {
                        received_size = received_size.saturating_add(chunk.len() as u64);
                        chunks.push(chunk);
                        if received_size > declared_size {
                            return stash_uncacheable_response(
                                &init_stash,
                                consumed,
                                stream,
                                &cache,
                                &l2_write_semaphore,
                                Some(&resource_info),
                                init_generation,
                                version,
                            )
                            .await;
                        }
                    },
                    ReadPart::ResourceInfo(_) | ReadPart::Redirect(_) => {
                        return stash_uncacheable_response(
                            &init_stash,
                            consumed,
                            stream,
                            &cache,
                            &l2_write_semaphore,
                            Some(&resource_info),
                            init_generation,
                            version,
                        )
                        .await;
                    },
                }
            }

            if received_size != declared_size {
                return stash_uncacheable_response(
                    &init_stash,
                    consumed,
                    empty_read_stream(),
                    &cache,
                    &l2_write_semaphore,
                    Some(&resource_info),
                    init_generation,
                    version,
                )
                .await;
            }

            Ok(SmallObjectCacheEntry {
                bytes: Some(concat_chunks(chunks)),
                resource_info,
            })
        });
        // Erase the cache lookup future as well as the initializer. The L2
        // single-flight path contains several generic async branches; embedding
        // their full layout in both protocol adapters can overflow rustc's
        // query-depth limit in all-feature service stacks.
        let fill: Pin<
            Box<dyn Future<Output = std::result::Result<SmallObjectCacheEntry, Arc<StorageError>>> + Send + '_>,
        > = Box::pin(
            self.cache
                .try_get_with_by_ref_with_options(&key, generation, CacheErrorPolicy::Never, init),
        );
        let result = fill.await;

        match result {
            Ok(entry) if entry.bytes.is_some() => entry.into(),
            Ok(entry) => {
                // Body keys should never contain metadata-only entries. If a legacy or
                // malformed entry does, use its metadata as a backend hint rather than
                // returning a bodyless cached response.
                let perform_read = take_mutex_option(&perform_read, "small_object_cache: perform_read mutex poisoned")?;
                match perform_read {
                    Some(perform_read) => perform_read(Some(entry.resource_info)).await,
                    None => Err(Box::new(Error::internal(
                        "small_object_cache: body cache entry had no bytes and perform_read was unavailable",
                    )) as StorageError),
                }
            },
            Err(e) => {
                let stashed = take_mutex_option(&stash, "small_object_cache: stash mutex poisoned")?;
                if let Some(stream) = stashed {
                    return Ok(stream);
                }

                if is_uncacheable(e.as_ref()) {
                    let perform_read =
                        take_mutex_option(&perform_read, "small_object_cache: perform_read mutex poisoned")?;
                    if let Some(perform_read) = perform_read {
                        // The init runner may just have published ResourceInfo before
                        // returning the sentinel, so read the hint after being woken.
                        let metadata_hint = get_metadata_hint(&self.cache, &metadata_key, generation).await?;
                        return perform_read(metadata_hint).await;
                    }
                }

                Err(Box::new(Error::cached_error(e)) as StorageError)
            },
        }
    }
}

fn take_mutex_option<T>(slot: &Mutex<Option<T>>, poisoned_message: &'static str) -> Result<Option<T>> {
    let mut slot = slot
        .lock()
        .map_err(|_| Box::new(Error::internal(poisoned_message)) as StorageError)?;
    Ok(slot.take())
}

async fn get_metadata_hint(
    cache: &Arc<LayerCache<SmallObjectCacheEntry>>,
    metadata_key: &str,
    generation: Option<u64>,
) -> Result<Option<ResourceInfo>> {
    match cache.get_two_tier_with_generation(metadata_key, generation).await {
        Some(Ok(entry)) => Ok(Some(entry.resource_info)),
        Some(Err(e)) => Err(Box::new(Error::cached_error(e)) as Box<dyn StorageErrorExt>),
        None => Ok(None),
    }
}

fn empty_read_stream() -> BoxStream<ReadPart> {
    Box::pin(futures::stream::empty())
}

/// Preserve an uncacheable response for the init runner and publish any complete
/// metadata as a future backend hint. Returning the sentinel keeps the response out of
/// both cache tiers while allowing Moka to wake coalesced waiters.
#[allow(clippy::too_many_arguments)]
async fn stash_uncacheable_response(
    stash: &Arc<Mutex<Option<BoxStream<ReadPart>>>>,
    consumed: Vec<Result<ReadPart>>,
    rest: BoxStream<ReadPart>,
    cache: &Arc<LayerCache<SmallObjectCacheEntry>>,
    l2_write_semaphore: &Arc<Semaphore>,
    resource_info: Option<&ResourceInfo>,
    generation: Option<u64>,
    version: ApiVersion,
) -> Result<SmallObjectCacheEntry> {
    let passthrough = futures::stream::iter(consumed).chain(rest);
    {
        let mut stash = stash
            .lock()
            .map_err(|_| Box::new(Error::internal("small_object_cache: stash mutex poisoned")) as StorageError)?;
        *stash = Some(Box::pin(passthrough) as BoxStream<ReadPart>);
    }

    if let Some(resource_info) = resource_info.filter(|info| resource_info_is_complete(info)) {
        let entry = SmallObjectCacheEntry {
            bytes: None,
            resource_info: resource_info.clone(),
        };
        // Preserve the backend-reported identity keying. If the backend rewrites the
        // request identity, this caller may miss the hint, but its independent
        // self-fetch remains correct.
        insert_cache_entry(
            cache,
            l2_write_semaphore,
            meta_key(&resource_info.encoded_identity, version),
            entry,
            generation,
        )
        .await;
    }

    Err(Box::new(Error::Uncacheable) as StorageError)
}

fn resource_info_is_complete(resource_info: &ResourceInfo) -> bool {
    !resource_info.encoded_identity.is_empty()
        && resource_info.data_object_size.is_some()
        && resource_info.last_modified_timestamp.is_some()
}

pub(crate) async fn insert_cache_entry(
    cache: &Arc<LayerCache<SmallObjectCacheEntry>>,
    l2_write_semaphore: &Arc<Semaphore>,
    key: String,
    entry: SmallObjectCacheEntry,
    generation: Option<u64>,
) {
    cache
        .insert_l1_with_generation(key.clone(), Ok(entry.clone()), generation)
        .await;

    if cache.has_l2() {
        let semaphore = l2_write_semaphore.clone();
        if let Ok(permit) = semaphore.try_acquire_owned() {
            let cache = cache.clone();
            tokio::spawn(async move {
                let _permit = permit;
                cache.insert_l2_with_generation(key, Ok(entry), generation).await;
            });
        } else {
            warn_limited!("small_object_cache: L2 write dropped — semaphore saturated");
        }
    }
}

/// Concatenate a list of `Bytes` chunks into one. Avoids the copy when there's a single
/// chunk (the common case for a small object that fits a single SDK read).
fn concat_chunks(chunks: Vec<Bytes>) -> Bytes {
    if chunks.len() == 1 {
        return chunks.into_iter().next().expect("len == 1");
    }
    let total: usize = chunks.iter().map(|b| b.len()).sum();
    let mut buf = BytesMut::with_capacity(total);
    for chunk in chunks {
        buf.extend_from_slice(&chunk);
    }
    buf.freeze()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SmallObjectCacheEntry {
    /// `None` for metadata-only entries (written by `stat` / `list_stat` cache fills),
    /// `Some(_)` for body entries written after a successful body read.
    pub bytes: Option<Bytes>,
    pub resource_info: ResourceInfo,
}

impl CachedSize for SmallObjectCacheEntry {
    fn cached_size(&self) -> u32 {
        let bytes_total = self.bytes.as_ref().map(Bytes::len).unwrap_or(0);
        let native_metadata = storage_core_util::api_types::native_metadata_size(&self.resource_info.native_metadata);
        let raw =
            std::mem::size_of::<Self>() + bytes_total + self.resource_info.encoded_identity.len() + native_metadata;
        u32::try_from(raw).unwrap_or(u32::MAX)
    }
}

impl CachedValue for SmallObjectCacheEntry {
    // Bumped to 4: `resource_info` now carries native_metadata (postcard-safe, prost-encoded
    // values), a wire-incompatible shape change for any Redis (L2) entries written under
    // schema = 3.
    const SCHEMA: u32 = 4;
}

impl From<SmallObjectCacheEntry> for Result<BoxStream<ReadPart>> {
    fn from(val: SmallObjectCacheEntry) -> Self {
        let resource_info = val.resource_info;
        match val.bytes {
            Some(bytes) => Ok(Box::pin(futures::stream::iter([
                Ok(ReadPart::ResourceInfo(resource_info)),
                Ok(ReadPart::Chunk(bytes)),
            ]))),
            None => Ok(Box::pin(futures::stream::iter([Ok(ReadPart::ResourceInfo(
                resource_info,
            ))]))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_and_meta_keys_are_partitioned_by_api_version() {
        // A v1beta fill (whose proto cannot carry native_metadata) must not collide with a v1alpha
        // fill for the same identity, or a v1alpha hit would return an empty map.
        let id = "https://account.blob.core.windows.net/container/blob";
        assert_ne!(body_key(id, ApiVersion::V1Alpha), body_key(id, ApiVersion::V1Beta));
        assert_ne!(meta_key(id, ApiVersion::V1Alpha), meta_key(id, ApiVersion::V1Beta));
        assert_ne!(body_key(id, ApiVersion::V1Alpha), meta_key(id, ApiVersion::V1Alpha));
    }

    use std::{
        collections::HashSet,
        sync::atomic::{AtomicUsize, Ordering},
        sync::Mutex,
    };

    use storage_core_protos::google::protobuf::Timestamp;
    use tokio::sync::{Barrier, Notify};

    #[test]
    fn identity_errors_distinguish_client_input_from_backend_output() {
        let client_error = canonical_identity("not a URL").unwrap_err();
        assert_eq!(client_error.tonic_status().code(), tonic::Code::InvalidArgument);

        let backend_error = canonical_backend_identity("not a URL").unwrap_err();
        assert_eq!(backend_error.tonic_status().code(), tonic::Code::Internal);
        assert!(
            backend_error
                .to_string()
                .contains("backend returned an invalid resource identity"),
            "{backend_error}"
        );
    }

    #[derive(Clone)]
    enum GatedResponse {
        Body { declared_size: u64, chunks: Vec<Bytes> },
        Redirect,
        StreamError { code: tonic::Code },
    }

    /// Backend used by the concurrency tests. Its first call remains blocked until the
    /// test releases it, giving every other caller time to join the same cache fill.
    #[derive(Clone)]
    struct GatedBackend {
        calls: Arc<AtomicUsize>,
        first_call_started: Arc<Notify>,
        release_first_call: Arc<Notify>,
        hints: Arc<Mutex<Vec<Option<ResourceInfo>>>>,
        response: GatedResponse,
    }

    impl GatedBackend {
        fn new(response: GatedResponse) -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                first_call_started: Arc::new(Notify::new()),
                release_first_call: Arc::new(Notify::new()),
                hints: Arc::new(Mutex::new(Vec::new())),
                response,
            }
        }

        async fn read(&self, resource_identity: String, hint: Option<ResourceInfo>) -> Result<BoxStream<ReadPart>> {
            self.hints.lock().expect("hints mutex").push(hint);
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                self.first_call_started.notify_one();
                self.release_first_call.notified().await;
            }

            match &self.response {
                GatedResponse::Body { declared_size, chunks } => {
                    let mut parts = vec![Ok(ReadPart::ResourceInfo(ResourceInfo {
                        native_metadata: Default::default(),
                        encoded_identity: resource_identity,
                        data_object_size: Some(*declared_size),
                        last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                    }))];
                    parts.extend(chunks.iter().cloned().map(|chunk| Ok(ReadPart::Chunk(chunk))));
                    Ok(Box::pin(futures::stream::iter(parts)))
                },
                GatedResponse::Redirect => Ok(implicit_redirect_stream(resource_identity, call)),
                GatedResponse::StreamError { code } => {
                    let parts = vec![
                        Ok(ReadPart::ResourceInfo(ResourceInfo {
                            native_metadata: Default::default(),
                            encoded_identity: resource_identity,
                            data_object_size: Some(3),
                            last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                        })),
                        Ok(ReadPart::Chunk(Bytes::from_static(b"abc"))),
                        Err(Box::new(CodedError(*code)) as StorageError),
                    ];
                    Ok(Box::pin(futures::stream::iter(parts)))
                },
            }
        }

        fn recorded_hints(&self) -> Vec<Option<ResourceInfo>> {
            self.hints.lock().expect("hints mutex").clone()
        }
    }

    #[derive(Debug)]
    struct CodedError(tonic::Code);

    impl std::fmt::Display for CodedError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "backend {:?} error", self.0)
        }
    }

    impl std::error::Error for CodedError {}

    impl StorageErrorExt for CodedError {
        fn tonic_status(&self) -> tonic::Status {
            tonic::Status::new(self.0, self.to_string())
        }

        fn http_status(&self) -> http::StatusCode {
            match self.0 {
                tonic::Code::NotFound => http::StatusCode::NOT_FOUND,
                tonic::Code::PermissionDenied => http::StatusCode::FORBIDDEN,
                _ => http::StatusCode::INTERNAL_SERVER_ERROR,
            }
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    #[derive(Debug)]
    struct NotFoundError;

    impl std::fmt::Display for NotFoundError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("object not found")
        }
    }

    impl std::error::Error for NotFoundError {}

    impl StorageErrorExt for NotFoundError {
        fn tonic_status(&self) -> tonic::Status {
            tonic::Status::not_found(self.to_string())
        }

        fn http_status(&self) -> http::StatusCode {
            http::StatusCode::NOT_FOUND
        }

        fn get_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    async fn test_service(
        namespace: &str,
        small_object_size: ByteSize,
    ) -> (Arc<SmallObjectCache<()>>, Arc<LayerCache<SmallObjectCacheEntry>>) {
        let config = Config {
            small_object_size,
            ..Config::default()
        };
        test_service_with_config(namespace, config, None).await
    }

    async fn test_service_with_config(
        namespace: &str,
        config: Config,
        generation: Option<SecretsGeneration>,
    ) -> (Arc<SmallObjectCache<()>>, Arc<LayerCache<SmallObjectCacheEntry>>) {
        let args: LayerCacheArgs = config.clone().into();
        let cache = Arc::new(
            LayerCache::new(&args, "test_small_object_cache", namespace, None, generation)
                .await
                .expect("LayerCache build"),
        );
        let service = Arc::new(SmallObjectCache::new(
            (),
            cache.clone(),
            config,
            Arc::new(Semaphore::new(1)),
        ));
        (service, cache)
    }

    /// Drive `callers` reads together and hold the first backend call while every
    /// sibling read enrolls behind the in-progress body-key fill.
    async fn run_gated_reads(
        service: Arc<SmallObjectCache<()>>,
        backend: GatedBackend,
        resource_identity: &str,
        callers: usize,
    ) -> Vec<BoxStream<ReadPart>> {
        run_gated_read_results(service, backend, resource_identity, callers)
            .await
            .into_iter()
            .map(|result| result.expect("read should succeed"))
            .collect()
    }

    async fn run_gated_read_results(
        service: Arc<SmallObjectCache<()>>,
        backend: GatedBackend,
        resource_identity: &str,
        callers: usize,
    ) -> Vec<Result<BoxStream<ReadPart>>> {
        let reads = (0..callers).map(|_| {
            let service = service.clone();
            let backend = backend.clone();
            let resource_identity = resource_identity.to_string();
            async move {
                service
                    .read_internal(
                        resource_identity.clone(),
                        ApiVersion::V1Beta,
                        0,
                        move |hint| async move { backend.read(resource_identity, hint).await },
                    )
                    .await
            }
        });
        let reads = futures::future::join_all(reads);
        tokio::pin!(reads);

        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                _ = backend.first_call_started.notified() => {},
                _ = &mut reads => panic!("reads completed before the gated backend call started"),
            }
        })
        .await
        .expect("first backend call should start while reads are being polled");
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            1,
            "all callers should wait behind the first in-progress cache fill"
        );

        backend.release_first_call.notify_one();
        tokio::time::timeout(Duration::from_secs(5), &mut reads)
            .await
            .expect("reads should finish after releasing the backend")
            .into_iter()
            .collect()
    }

    async fn body_bytes(mut stream: BoxStream<ReadPart>) -> Bytes {
        let mut saw_resource_info = false;
        let mut body = BytesMut::new();
        while let Some(part) = stream.next().await {
            match part.expect("read part") {
                ReadPart::ResourceInfo(_) => saw_resource_info = true,
                ReadPart::Chunk(chunk) => body.extend_from_slice(&chunk),
                ReadPart::Redirect(_) => panic!("body response must not redirect"),
            }
        }
        assert!(saw_resource_info, "body response should include ResourceInfo");
        body.freeze()
    }

    fn expect_read_error(result: Result<BoxStream<ReadPart>>, context: &str) -> StorageError {
        match result {
            Ok(_) => panic!("{context}"),
            Err(error) => error,
        }
    }

    fn implicit_redirect_stream(resource_identity: String, call: usize) -> BoxStream<ReadPart> {
        let parts: Vec<Result<ReadPart>> = vec![
            Ok(ReadPart::ResourceInfo(ResourceInfo {
                native_metadata: Default::default(),
                encoded_identity: resource_identity,
                data_object_size: Some(3),
                last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
            })),
            Ok(ReadPart::Redirect(storage_core_util::api_types::Redirect {
                redirect_target_url: format!("https://example.invalid/object?call={call}"),
                method: "GET".to_string(),
                additional_headers: Vec::new(),
            })),
        ];
        Box::pin(futures::stream::iter(parts))
    }

    fn complete_body_stream(resource_identity: String, bytes: Bytes) -> BoxStream<ReadPart> {
        let size = bytes.len() as u64;
        Box::pin(futures::stream::iter([
            Ok(ReadPart::ResourceInfo(ResourceInfo {
                native_metadata: Default::default(),
                encoded_identity: resource_identity,
                data_object_size: Some(size),
                last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
            })),
            Ok(ReadPart::Chunk(bytes)),
        ]))
    }

    async fn redirect_url(mut stream: BoxStream<ReadPart>) -> String {
        let mut saw_resource_info = false;
        let mut redirect_url = None;
        while let Some(part) = stream.next().await {
            match part.expect("read part") {
                ReadPart::ResourceInfo(_) => saw_resource_info = true,
                ReadPart::Redirect(redirect) => redirect_url = Some(redirect.redirect_target_url),
                ReadPart::Chunk(_) => panic!("redirect response must not contain a body chunk"),
            }
        }
        assert!(saw_resource_info, "redirect response should retain ResourceInfo");
        redirect_url.expect("redirect response should retain Redirect")
    }

    #[tokio::test]
    async fn saturated_l2_write_semaphore_does_not_drop_l1_fill() {
        let config = Config {
            small_object_size: ByteSize::kib(1),
            ..Config::default()
        };
        let args: LayerCacheArgs = config.clone().into();
        let cache = Arc::new(
            LayerCache::new(
                &args,
                "test_small_object_cache",
                "test_small_object_cache_saturated_l2_write",
                None,
                None,
            )
            .await
            .expect("LayerCache build"),
        );
        let semaphore = Arc::new(Semaphore::new(1));
        let _held_permit = semaphore.clone().try_acquire_owned().expect("initial semaphore permit");
        let service = SmallObjectCache::new((), cache.clone(), config, semaphore);

        let resource_identity = "https://example.com/test-resource-identity".to_string();
        let stream_identity = resource_identity.clone();
        let stream = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |_hint| async move {
                    let parts: Vec<Result<ReadPart>> = vec![
                        Ok(ReadPart::ResourceInfo(ResourceInfo {
                            native_metadata: Default::default(),
                            encoded_identity: stream_identity,
                            data_object_size: Some(3),
                            last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                        })),
                        Ok(ReadPart::Chunk(Bytes::from_static(b"abc"))),
                    ];
                    Ok(Box::pin(futures::stream::iter(parts)) as BoxStream<ReadPart>)
                },
            )
            .await
            .expect("read_internal");

        let mut body = Vec::new();
        futures::pin_mut!(stream);
        while let Some(part) = stream.next().await {
            if let ReadPart::Chunk(bytes) = part.expect("stream part") {
                body.extend_from_slice(&bytes);
            }
        }
        assert_eq!(body, b"abc");

        let cached = cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("L1 entry should be inserted even when L2 writes are saturated")
            .expect("cached value should not be an error");
        assert_eq!(cached.bytes, Some(Bytes::from_static(b"abc")));
        assert_eq!(cached.resource_info.encoded_identity, resource_identity);
    }

    #[tokio::test]
    async fn metadata_entry_hints_backend_and_body_entry_satisfies_next_read() {
        let config = Config {
            small_object_size: ByteSize::kib(1),
            ..Config::default()
        };
        let args: LayerCacheArgs = config.clone().into();
        let cache = Arc::new(
            LayerCache::new(
                &args,
                "test_small_object_cache",
                "test_small_object_cache_split_keyspace",
                None,
                None,
            )
            .await
            .expect("LayerCache build"),
        );
        let semaphore = Arc::new(Semaphore::new(1));
        let service = SmallObjectCache::new((), cache.clone(), config, semaphore.clone());
        let resource_identity = "https://example.com/test-resource-identity".to_string();
        let generation = cache.generation_snapshot();

        insert_cache_entry(
            &cache,
            &semaphore,
            meta_key(&resource_identity, ApiVersion::V1Beta),
            SmallObjectCacheEntry {
                bytes: None,
                resource_info: ResourceInfo {
                    native_metadata: Default::default(),
                    encoded_identity: resource_identity.clone(),
                    data_object_size: Some(3),
                    last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                },
            },
            generation,
        )
        .await;

        let stream_identity = resource_identity.clone();
        let stream = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |hint| async move {
                    let hint = hint.expect("metadata entry should be passed to the backend as a hint");
                    assert_eq!(hint.encoded_identity, stream_identity);
                    assert_eq!(hint.data_object_size, Some(3));
                    let parts: Vec<Result<ReadPart>> = vec![
                        Ok(ReadPart::ResourceInfo(hint)),
                        Ok(ReadPart::Chunk(Bytes::from_static(b"abc"))),
                    ];
                    Ok(Box::pin(futures::stream::iter(parts)) as BoxStream<ReadPart>)
                },
            )
            .await
            .expect("metadata hint must not satisfy a body read");

        let first_body = stream
            .filter_map(|part| async move {
                match part.expect("stream part") {
                    ReadPart::Chunk(bytes) => Some(bytes),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .await;
        assert_eq!(first_body, vec![Bytes::from_static(b"abc")]);

        let cached_body = cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("body read should populate the body key")
            .expect("cached value should not be an error");
        assert_eq!(cached_body.bytes, Some(Bytes::from_static(b"abc")));
        let cached_meta = cache
            .get(&meta_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("metadata entry should remain in its own keyspace")
            .expect("cached value should not be an error");
        assert!(cached_meta.bytes.is_none());

        let cached_stream = service
            .read_internal(resource_identity, ApiVersion::V1Beta, 0, |_hint| async move {
                Err(Box::new(Error::internal("backend must not run on a body-key hit")) as StorageError)
            })
            .await
            .expect("body-key hit should satisfy the read");
        let cached_body = cached_stream
            .filter_map(|part| async move {
                match part.expect("cached stream part") {
                    ReadPart::Chunk(bytes) => Some(bytes),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .await;
        assert_eq!(cached_body, vec![Bytes::from_static(b"abc")]);
    }

    #[tokio::test]
    async fn concurrent_small_reads_coalesce_and_cache_complete_body() {
        const CALLERS: usize = 8;
        let (service, cache) = test_service("coalesced_small_body", ByteSize::kib(1)).await;
        let backend = GatedBackend::new(GatedResponse::Body {
            declared_size: 6,
            chunks: vec![Bytes::from_static(b"abc"), Bytes::from_static(b"def")],
        });
        let resource_identity = "coalesced-small-body".to_string();

        let streams = run_gated_reads(service.clone(), backend.clone(), &resource_identity, CALLERS).await;
        let mut bodies = Vec::with_capacity(CALLERS);
        for stream in streams {
            bodies.push(body_bytes(stream).await);
        }

        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(bodies, vec![Bytes::from_static(b"abcdef"); CALLERS]);

        let cached_backend = backend.clone();
        let cached_identity = resource_identity.clone();
        let cached_stream = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |hint| async move { cached_backend.read(cached_identity, hint).await },
            )
            .await
            .expect("subsequent read should be served from the body cache");
        assert_eq!(body_bytes(cached_stream).await, Bytes::from_static(b"abcdef"));
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            1,
            "a body-cache hit must not call the backend"
        );

        let cached = cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("successful coalesced read should populate the body cache")
            .expect("body cache entry should not be an error");
        assert_eq!(cached.bytes, Some(Bytes::from_static(b"abcdef")));
    }

    #[tokio::test]
    async fn concurrent_redirects_self_fetch_and_are_never_cached() {
        const CALLERS: usize = 6;
        let (service, cache) = test_service("coalesced_redirects", ByteSize::kib(1)).await;
        let backend = GatedBackend::new(GatedResponse::Redirect);
        let resource_identity = "coalesced-redirect".to_string();

        let streams = run_gated_reads(service.clone(), backend.clone(), &resource_identity, CALLERS).await;
        let mut urls = HashSet::with_capacity(CALLERS + 1);
        for stream in streams {
            assert!(
                urls.insert(redirect_url(stream).await),
                "every caller needs its own redirect URL"
            );
        }

        assert_eq!(urls.len(), CALLERS);
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            CALLERS,
            "the initializer caller and every coalesced waiter should fetch independently"
        );
        let hints = backend.recorded_hints();
        assert_eq!(hints.len(), CALLERS);
        assert!(
            hints[0].is_none(),
            "the initializer should begin without a metadata hint"
        );
        assert!(
            hints[1..].iter().all(|hint| hint
                .as_ref()
                .is_some_and(|info| info.encoded_identity == resource_identity)),
            "every self-fetching waiter should receive the metadata published by the initializer"
        );
        assert!(
            cache
                .get(&body_key(&resource_identity, ApiVersion::V1Beta))
                .await
                .is_none(),
            "redirect responses must never populate the body cache"
        );

        let later_backend = backend.clone();
        let later_identity = resource_identity.clone();
        let later = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |hint| async move { later_backend.read(later_identity, hint).await },
            )
            .await
            .expect("later redirect read should call the backend again");
        assert!(
            urls.insert(redirect_url(later).await),
            "later read should receive a fresh redirect URL"
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), CALLERS + 1);
        assert!(cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .is_none());

        let metadata = cache
            .get(&meta_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("complete redirect metadata should remain available as a hint")
            .expect("metadata hint should not be an error");
        assert!(
            metadata.bytes.is_none(),
            "a redirect must never be represented as cached body bytes"
        );
    }

    #[tokio::test]
    async fn concurrent_large_body_reads_stream_through_without_caching() {
        const CALLERS: usize = 2;
        let (service, cache) = test_service("concurrent_large_body", ByteSize::b(4)).await;
        let backend = GatedBackend::new(GatedResponse::Body {
            declared_size: 10,
            chunks: vec![Bytes::from_static(b"large-"), Bytes::from_static(b"body")],
        });
        let resource_identity = "large-body".to_string();

        let streams = run_gated_reads(service, backend.clone(), &resource_identity, CALLERS).await;
        for stream in streams {
            assert_eq!(body_bytes(stream).await, Bytes::from_static(b"large-body"));
        }

        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            CALLERS,
            "a coalesced waiter for an uncacheable large object must perform its own read"
        );
        let hints = backend.recorded_hints();
        assert_eq!(hints.len(), CALLERS);
        assert!(
            hints[0].is_none(),
            "the initializer should begin without a metadata hint"
        );
        assert!(
            hints[1..].iter().all(|hint| hint
                .as_ref()
                .is_some_and(|info| info.encoded_identity == resource_identity)),
            "large-body waiters should consume the initializer's metadata hint"
        );
        assert!(
            cache
                .get(&body_key(&resource_identity, ApiVersion::V1Beta))
                .await
                .is_none(),
            "large bodies must be passed through without a body-cache entry"
        );
        let metadata = cache
            .get(&meta_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .expect("large-object ResourceInfo should be retained as a metadata hint")
            .expect("metadata hint should not be an error");
        assert!(metadata.bytes.is_none());
    }

    #[tokio::test]
    async fn not_found_is_not_retained_and_followup_read_can_succeed() {
        let (service, cache) = test_service("uncached_not_found", ByteSize::kib(1)).await;
        let backend_calls = Arc::new(AtomicUsize::new(0));
        let resource_identity = "missing-object".to_string();

        let first_calls = backend_calls.clone();
        let first = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |_hint| async move {
                    first_calls.fetch_add(1, Ordering::SeqCst);
                    Err(Box::new(NotFoundError) as StorageError)
                },
            )
            .await;
        let first_error = match first {
            Ok(_) => panic!("missing object should return NotFound"),
            Err(error) => error,
        };
        assert_eq!(first_error.tonic_status().code(), tonic::Code::NotFound);
        assert!(
            cache
                .get(&body_key(&resource_identity, ApiVersion::V1Beta))
                .await
                .is_none(),
            "NotFound must not poison the shared body key"
        );

        let second_calls = backend_calls.clone();
        let second_identity = resource_identity.clone();
        let second = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |_hint| async move {
                    second_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::pin(futures::stream::iter([
                        Ok(ReadPart::ResourceInfo(ResourceInfo {
                            native_metadata: Default::default(),
                            encoded_identity: second_identity,
                            data_object_size: Some(3),
                            last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                        })),
                        Ok(ReadPart::Chunk(Bytes::from_static(b"new"))),
                    ])) as BoxStream<ReadPart>)
                },
            )
            .await
            .expect("read after object creation should reach the backend");
        assert_eq!(body_bytes(second).await, Bytes::from_static(b"new"));
        assert_eq!(
            backend_calls.load(Ordering::SeqCst),
            2,
            "the followup read must retry after NotFound"
        );
        assert!(cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .is_some());
    }

    #[tokio::test]
    async fn access_denied_from_one_caller_does_not_poison_an_authorized_caller() {
        let (service, cache) = test_service("uncached_access_denied", ByteSize::kib(1)).await;
        let resource_identity = "shared-object".to_string();
        let calls = Arc::new(AtomicUsize::new(0));

        let denied_calls = calls.clone();
        let denied = expect_read_error(
            service
                .read_internal(
                    resource_identity.clone(),
                    ApiVersion::V1Beta,
                    0,
                    move |_hint| async move {
                        denied_calls.fetch_add(1, Ordering::SeqCst);
                        Err(Box::new(CodedError(tonic::Code::PermissionDenied)) as StorageError)
                    },
                )
                .await,
            "the first caller should be denied",
        );
        assert_eq!(denied.tonic_status().code(), tonic::Code::PermissionDenied);
        assert!(cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .is_none());

        let authorized_calls = calls.clone();
        let authorized_identity = resource_identity.clone();
        let authorized = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |_hint| async move {
                    authorized_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(complete_body_stream(
                        authorized_identity,
                        Bytes::from_static(b"allowed"),
                    ))
                },
            )
            .await
            .expect("an authorized caller must reach the backend");
        assert_eq!(body_bytes(authorized).await, Bytes::from_static(b"allowed"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn internal_error_configuration_cannot_retain_uncacheable_sentinel() {
        let config = Config {
            errors: vec!["Internal".to_string()],
            small_object_size: ByteSize::kib(1),
            ..Config::default()
        };
        let (service, cache) = test_service_with_config("internal_sentinel", config, None).await;
        let backend = GatedBackend::new(GatedResponse::Redirect);
        let resource_identity = "internal-sentinel-object".to_string();

        let first = run_gated_reads(service.clone(), backend.clone(), &resource_identity, 1)
            .await
            .pop()
            .expect("first redirect stream");
        assert!(redirect_url(first).await.contains("call=0"));
        assert!(cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .is_none());

        let later_backend = backend.clone();
        let later_identity = resource_identity.clone();
        let second = service
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Beta,
                0,
                move |hint| async move { later_backend.read(later_identity, hint).await },
            )
            .await
            .expect("the sentinel must not be returned from a retained error");
        assert!(redirect_url(second).await.contains("call=1"));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        assert!(cache
            .get(&body_key(&resource_identity, ApiVersion::V1Beta))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn midstream_errors_are_shared_with_waiters_but_never_retained() {
        const CALLERS: usize = 4;
        for (namespace, code) in [
            ("midstream_not_found", tonic::Code::NotFound),
            ("midstream_internal", tonic::Code::Internal),
        ] {
            let (service, cache) = test_service(namespace, ByteSize::kib(1)).await;
            let backend = GatedBackend::new(GatedResponse::StreamError { code });
            let resource_identity = format!("midstream-{code:?}");

            let results = run_gated_read_results(service.clone(), backend.clone(), &resource_identity, CALLERS).await;
            for result in results {
                let error = expect_read_error(result, "a candidate body with a stream error must fail to open");
                assert_eq!(error.tonic_status().code(), code);
            }
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
            assert!(cache
                .get(&body_key(&resource_identity, ApiVersion::V1Beta))
                .await
                .is_none());

            let later_backend = backend.clone();
            let later_identity = resource_identity.clone();
            let later = expect_read_error(
                service
                    .read_internal(
                        resource_identity.clone(),
                        ApiVersion::V1Beta,
                        0,
                        move |hint| async move { later_backend.read(later_identity, hint).await },
                    )
                    .await,
                "a later read should retry and see the backend error again",
            );
            assert_eq!(later.tonic_status().code(), code);
            assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
            assert!(cache
                .get(&body_key(&resource_identity, ApiVersion::V1Beta))
                .await
                .is_none());
        }
    }

    #[tokio::test]
    async fn leader_cancellation_allows_a_waiter_to_take_over_the_fill() {
        const WAITERS: usize = 4;
        let (service, _cache) = test_service("leader_cancellation", ByteSize::kib(1)).await;
        let backend = GatedBackend::new(GatedResponse::Body {
            declared_size: 3,
            chunks: vec![Bytes::from_static(b"abc")],
        });
        let resource_identity = "cancelled-leader".to_string();

        let leader = {
            let service = service.clone();
            let backend = backend.clone();
            let identity = resource_identity.clone();
            tokio::spawn(async move {
                service
                    .read_internal(identity.clone(), ApiVersion::V1Beta, 0, move |hint| async move {
                        backend.read(identity, hint).await
                    })
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(5), backend.first_call_started.notified())
            .await
            .expect("leader backend call should start");

        let barrier = Arc::new(Barrier::new(WAITERS + 1));
        let waiters = (0..WAITERS)
            .map(|_| {
                let service = service.clone();
                let backend = backend.clone();
                let identity = resource_identity.clone();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    service
                        .read_internal(identity.clone(), ApiVersion::V1Beta, 0, move |hint| async move {
                            backend.read(identity, hint).await
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();
        barrier.wait().await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);

        leader.abort();
        let join_error = match leader.await {
            Ok(_) => panic!("leader task should be cancelled"),
            Err(error) => error,
        };
        assert!(join_error.is_cancelled());
        backend.release_first_call.notify_waiters();

        let waiter_results = tokio::time::timeout(Duration::from_secs(5), futures::future::join_all(waiters))
            .await
            .expect("waiters should complete after leader cancellation");
        for result in waiter_results {
            let stream = result
                .expect("waiter task should join")
                .expect("waiter read should succeed");
            assert_eq!(body_bytes(stream).await, Bytes::from_static(b"abc"));
        }
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            2,
            "one waiter should take over with its own backend initializer"
        );
    }

    #[tokio::test]
    async fn unknown_size_response_is_exact_passthrough_without_cache_entries() {
        let (service, cache) = test_service("unknown_size", ByteSize::kib(1)).await;
        let identity = "unknown-size".to_string();
        let response_identity = identity.clone();
        let mut stream = service
            .read_internal(identity.clone(), ApiVersion::V1Beta, 0, move |_hint| async move {
                Ok(Box::pin(futures::stream::iter([
                    Ok(ReadPart::ResourceInfo(ResourceInfo {
                        native_metadata: Default::default(),
                        encoded_identity: response_identity,
                        data_object_size: None,
                        last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                    })),
                    Ok(ReadPart::Chunk(Bytes::from_static(b"abc"))),
                ])) as BoxStream<ReadPart>)
            })
            .await
            .expect("unknown-size response should pass through");
        assert!(matches!(
            stream.next().await,
            Some(Ok(ReadPart::ResourceInfo(ResourceInfo {
                data_object_size: None,
                ..
            })))
        ));
        assert!(
            matches!(stream.next().await, Some(Ok(ReadPart::Chunk(ref bytes))) if bytes == &Bytes::from_static(b"abc"))
        );
        assert!(stream.next().await.is_none());
        assert!(cache.get(&body_key(&identity, ApiVersion::V1Beta)).await.is_none());
        assert!(cache.get(&meta_key(&identity, ApiVersion::V1Beta)).await.is_none());
    }

    #[tokio::test]
    async fn empty_response_is_exact_passthrough_without_cache_entries() {
        let (service, cache) = test_service("empty_response", ByteSize::kib(1)).await;
        let identity = "empty-response".to_string();
        let mut stream = service
            .read_internal(identity.clone(), ApiVersion::V1Beta, 0, |_hint| async move {
                Ok(Box::pin(futures::stream::empty()) as BoxStream<ReadPart>)
            })
            .await
            .expect("empty response should pass through");
        assert!(stream.next().await.is_none());
        assert!(cache.get(&body_key(&identity, ApiVersion::V1Beta)).await.is_none());
        assert!(cache.get(&meta_key(&identity, ApiVersion::V1Beta)).await.is_none());
    }

    #[tokio::test]
    async fn non_metadata_first_part_is_exact_passthrough_without_cache_entries() {
        let (service, cache) = test_service("chunk_first", ByteSize::kib(1)).await;
        let identity = "chunk-first".to_string();
        let mut stream = service
            .read_internal(identity.clone(), ApiVersion::V1Beta, 0, |_hint| async move {
                Ok(
                    Box::pin(futures::stream::iter([Ok(ReadPart::Chunk(Bytes::from_static(b"abc")))]))
                        as BoxStream<ReadPart>,
                )
            })
            .await
            .expect("headerless response should pass through");
        assert!(
            matches!(stream.next().await, Some(Ok(ReadPart::Chunk(ref bytes))) if bytes == &Bytes::from_static(b"abc"))
        );
        assert!(stream.next().await.is_none());
        assert!(cache.get(&body_key(&identity, ApiVersion::V1Beta)).await.is_none());
        assert!(cache.get(&meta_key(&identity, ApiVersion::V1Beta)).await.is_none());
    }

    #[tokio::test]
    async fn backend_identity_mismatch_is_passthrough_and_not_body_cached() {
        let (service, cache) = test_service("identity_mismatch", ByteSize::kib(1)).await;
        let request_identity = "request-identity".to_string();
        let backend_identity = "backend-identity".to_string();
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let calls = calls.clone();
            let response_identity = backend_identity.clone();
            let stream = service
                .read_internal(
                    request_identity.clone(),
                    ApiVersion::V1Beta,
                    0,
                    move |_hint| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(complete_body_stream(response_identity, Bytes::from_static(b"abc")))
                    },
                )
                .await
                .expect("identity-rewritten response should pass through");
            assert_eq!(body_bytes(stream).await, Bytes::from_static(b"abc"));
        }

        assert_eq!(calls.load(Ordering::SeqCst), 2, "mismatched bodies must be refetched");
        assert!(cache
            .get(&body_key(&request_identity, ApiVersion::V1Beta))
            .await
            .is_none());
        assert!(cache
            .get(&body_key(&backend_identity, ApiVersion::V1Beta))
            .await
            .is_none());
        assert!(cache
            .get(&meta_key(&request_identity, ApiVersion::V1Beta))
            .await
            .is_none());
        let metadata = cache
            .get(&meta_key(&backend_identity, ApiVersion::V1Beta))
            .await
            .expect("backend identity should receive a metadata hint")
            .expect("metadata hint should not be an error");
        assert!(metadata.bytes.is_none());
    }

    #[tokio::test]
    async fn uncacheable_metadata_uses_request_entry_generation() {
        let initial = Figment::new().join(("endpoint", "initial"));
        let replacement = Figment::new().join(("endpoint", "replacement"));
        let generation = SecretsGeneration::new(Some(&initial));
        let config = Config {
            small_object_size: ByteSize::b(4),
            ..Config::default()
        };
        let (service, cache) = test_service_with_config("request_generation", config, Some(generation.clone())).await;
        let backend = GatedBackend::new(GatedResponse::Body {
            declared_size: 10,
            chunks: vec![Bytes::from_static(b"large-body")],
        });
        let identity = "generation-pinned".to_string();
        let old_generation = cache.generation_snapshot();

        let read = {
            let service = service.clone();
            let backend = backend.clone();
            let identity = identity.clone();
            tokio::spawn(async move {
                service
                    .read_internal(identity.clone(), ApiVersion::V1Beta, 0, move |hint| async move {
                        backend.read(identity, hint).await
                    })
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(5), backend.first_call_started.notified())
            .await
            .expect("backend fill should start");
        generation.update(Some(&replacement));
        assert_ne!(cache.generation_snapshot(), old_generation);
        backend.release_first_call.notify_one();

        let stream = read
            .await
            .expect("read task should join")
            .expect("read should pass through");
        assert_eq!(body_bytes(stream).await, Bytes::from_static(b"large-body"));
        let key = meta_key(&identity, ApiVersion::V1Beta);
        assert!(
            cache.get(&key).await.is_none(),
            "old-policy metadata must be hidden now"
        );
        let old_entry = cache
            .get_two_tier_with_generation(&key, old_generation)
            .await
            .expect("metadata should be written to the request-entry partition")
            .expect("metadata hint should not be an error");
        assert_eq!(old_entry.resource_info.encoded_identity, identity);
    }

    #[cfg(feature = "stat_cache")]
    #[allow(deprecated)]
    mod read_from_address {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use crate::{
            layer::Layer,
            stat_cache::{Config as StatCacheConfig, StatCacheLayer},
            traits::{
                async_trait,
                fileobject::v1beta::{
                    read_from_address_response, read_response, Chunk, DownloadPreference, FileObject, Metadata,
                    ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, Redirect,
                    ResourceIdentity, ResourceInfo as ResourceInfoV1Beta, StatRequest, StatResponse,
                },
                Request,
            },
        };
        use storage_core_protos::google::protobuf::Timestamp;

        use super::*;

        /// Backend that responds to every `read_from_address` with `[ResourceInfo, Redirect]`.
        #[derive(Clone, Default)]
        struct RedirectBackend {
            stat_calls: Arc<AtomicUsize>,
            read_from_address_calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl FileObject for RedirectBackend {
            fn inner(&self) -> Option<&dyn FileObject> {
                None
            }

            fn backend(&self) -> &'static str {
                "redirect-test"
            }

            async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
                self.stat_calls.fetch_add(1, Ordering::SeqCst);
                Ok(StatResponse {
                    resource_info: Some(resource_info()),
                })
            }

            async fn read_from_address(
                &self,
                request: Request<ReadFromAddressRequest>,
            ) -> Result<BoxStream<ReadFromAddressResponse>> {
                let call = self.read_from_address_calls.fetch_add(1, Ordering::SeqCst);
                if request.get_ref().download_preference == Some(DownloadPreference::Body.into()) {
                    let responses: Vec<Result<ReadFromAddressResponse>> = vec![
                        Ok(ReadFromAddressResponse {
                            reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(
                                small_resource_info(),
                            )),
                        }),
                        Ok(ReadFromAddressResponse {
                            reply_type: Some(read_from_address_response::ReplyType::Chunk(Chunk {
                                chunk: Bytes::from_static(b"abc"),
                            })),
                        }),
                    ];
                    return Ok(Box::pin(futures::stream::iter(responses)));
                }
                let responses: Vec<Result<ReadFromAddressResponse>> = vec![
                    Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(resource_info())),
                    }),
                    Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Redirect(Redirect {
                            redirect_target_url: format!("https://example.invalid/object?call={call}"),
                            method: "GET".to_string(),
                            additional_headers: Vec::new(),
                        })),
                    }),
                ];
                Ok(Box::pin(futures::stream::iter(responses)))
            }
        }

        /// Backend that responds with `[ResourceInfo, Err]` to exercise passthrough errors.
        #[derive(Clone, Default)]
        struct ErrorBackend {
            stat_calls: Arc<AtomicUsize>,
            read_from_address_calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl FileObject for ErrorBackend {
            fn inner(&self) -> Option<&dyn FileObject> {
                None
            }

            fn backend(&self) -> &'static str {
                "error-test"
            }

            async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
                self.stat_calls.fetch_add(1, Ordering::SeqCst);
                Ok(StatResponse {
                    resource_info: Some(resource_info()),
                })
            }

            async fn read_from_address(
                &self,
                _request: Request<ReadFromAddressRequest>,
            ) -> Result<BoxStream<ReadFromAddressResponse>> {
                self.read_from_address_calls.fetch_add(1, Ordering::SeqCst);
                let responses: Vec<Result<ReadFromAddressResponse>> = vec![
                    Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(resource_info())),
                    }),
                    Err(Box::new(Error::internal("backend boom")) as StorageError),
                ];
                Ok(Box::pin(futures::stream::iter(responses)))
            }
        }

        /// Identity-read backend that would need a live stat when the request does not
        /// carry a metadata hint, mirroring the behavior of the SDK backends.
        #[derive(Clone, Default)]
        struct HintAwareBodyBackend {
            stat_calls: Arc<AtomicUsize>,
            read_calls: Arc<AtomicUsize>,
            read_hints: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl FileObject for HintAwareBodyBackend {
            fn inner(&self) -> Option<&dyn FileObject> {
                None
            }

            fn backend(&self) -> &'static str {
                "hint-aware-body-test"
            }

            async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
                self.stat_calls.fetch_add(1, Ordering::SeqCst);
                Ok(StatResponse {
                    resource_info: Some(small_resource_info()),
                })
            }

            async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
                self.read_calls.fetch_add(1, Ordering::SeqCst);
                if let Some(hint) = request.extensions().get::<ResourceInfoV1Beta>() {
                    self.read_hints.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(hint, &small_resource_info());
                } else {
                    // Model the live metadata lookup that the real SDK backends perform
                    // when no ResourceInfo extension was supplied.
                    self.stat_calls.fetch_add(1, Ordering::SeqCst);
                }

                let responses: Vec<Result<ReadResponse>> = vec![
                    Ok(ReadResponse {
                        reply_type: Some(read_response::ReplyType::Metadata(Metadata {
                            data_object_size: Some(3),
                            last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                        })),
                    }),
                    Ok(ReadResponse {
                        reply_type: Some(read_response::ReplyType::Chunk(Chunk {
                            chunk: Bytes::from_static(b"abc"),
                        })),
                    }),
                ];
                Ok(Box::pin(futures::stream::iter(responses)))
            }
        }

        fn resource_info() -> ResourceInfoV1Beta {
            ResourceInfoV1Beta {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: "https://example.com/test-resource-identity".to_string(),
                }),
                metadata: Some(Metadata {
                    data_object_size: Some(50 * 1024 * 1024 + 1),
                    last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                }),
            }
        }

        fn small_resource_info() -> ResourceInfoV1Beta {
            ResourceInfoV1Beta {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: "https://example.com/test-resource-identity".to_string(),
                }),
                metadata: Some(Metadata {
                    data_object_size: Some(3),
                    last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                }),
            }
        }

        fn read_request_with(preference: DownloadPreference) -> Request<ReadFromAddressRequest> {
            tonic::Request::new(ReadFromAddressRequest {
                resource_address: "s3://example-bucket/object".to_string(),
                download_preference: Some(preference.into()),
            })
            .into()
        }

        /// Layer the small_object_cache + stat_cache on top of a given `FileObject`. Each
        /// caller supplies a unique `namespace` to keep parallel tests from sharing cache
        /// entries.
        async fn layered<B: FileObject + Clone + 'static>(backend: B, namespace: &'static str) -> impl FileObject {
            let small_object_cache_layer = SmallObjectCacheLayer::new(Config::default(), None, Some(namespace))
                .await
                .expect("small object cache layer")
                .expect("small object cache enabled");
            let stat_cache_layer = StatCacheLayer::new(StatCacheConfig::default(), None, Some(namespace))
                .await
                .expect("stat cache layer")
                .expect("stat cache enabled");
            stat_cache_layer.layer(small_object_cache_layer.layer(backend))
        }

        #[test_log::test(tokio::test)]
        async fn stat_then_read_uses_metadata_hint_and_fills_body_cache() {
            let backend = HintAwareBodyBackend::default();
            let small_object_cache_layer = SmallObjectCacheLayer::new(
                Config {
                    small_object_size: ByteSize::kib(1),
                    ..Config::default()
                },
                None,
                Some("stat_then_identity_read_hint"),
            )
            .await
            .expect("small object cache layer")
            .expect("small object cache enabled");
            let cache = small_object_cache_layer.cache.clone();
            let service = small_object_cache_layer.layer(backend.clone());

            service
                .stat(
                    tonic::Request::new(StatRequest {
                        resource_address: "s3://example-bucket/object".to_string(),
                    })
                    .into(),
                )
                .await
                .expect("stat should populate the identity-keyed metadata hint");
            assert_eq!(backend.stat_calls.load(Ordering::SeqCst), 1);

            let mut first = service
                .read(
                    tonic::Request::new(ReadRequest {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: "https://example.com/test-resource-identity".to_string(),
                        }),
                        download_preference: Some(DownloadPreference::Body.into()),
                    })
                    .into(),
                )
                .await
                .expect("identity read should use the stat-derived hint");
            let mut body = BytesMut::new();
            while let Some(response) = first.next().await {
                if let Some(read_response::ReplyType::Chunk(chunk)) = response.expect("read response").reply_type {
                    body.extend_from_slice(&chunk.chunk);
                }
            }
            assert_eq!(body.freeze(), Bytes::from_static(b"abc"));
            assert_eq!(backend.read_calls.load(Ordering::SeqCst), 1);
            assert_eq!(backend.read_hints.load(Ordering::SeqCst), 1);
            assert_eq!(
                backend.stat_calls.load(Ordering::SeqCst),
                1,
                "the metadata hint should prevent a second backend stat during read"
            );

            let cached = cache
                .get(&body_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta,
                ))
                .await
                .expect("hinted read should fill the body cache")
                .expect("body entry should not be an error");
            assert_eq!(cached.bytes, Some(Bytes::from_static(b"abc")));
        }

        #[test_log::test(tokio::test)]
        async fn identity_path_spelling_aliases_share_body_cache() {
            const LOOSE: &str = "https://example.com/!$%&'()*+,:;=@[]^|?version-id=v%2B1";
            const STRICT: &str =
                "https://example.com/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C?version-id=v%2B1";

            let backend = HintAwareBodyBackend::default();
            let layer = SmallObjectCacheLayer::new(Config::default(), None, Some("canonical_identity_body_key"))
                .await
                .expect("small object cache layer")
                .expect("small object cache enabled");
            let cache = layer.cache.clone();
            let service = layer.layer(backend.clone());

            for identity in [LOOSE, STRICT] {
                let mut stream = service
                    .read(
                        tonic::Request::new(ReadRequest {
                            resource_identity: Some(ResourceIdentity {
                                encoded_identity: identity.to_string(),
                            }),
                            download_preference: Some(DownloadPreference::Body.into()),
                        })
                        .into(),
                    )
                    .await
                    .expect("read");
                while stream.next().await.is_some() {}
            }

            assert_eq!(backend.read_calls.load(Ordering::SeqCst), 1);
            let entry = cache
                .get(&body_key(STRICT, ApiVersion::V1Beta))
                .await
                .expect("strict body key")
                .expect("body entry");
            assert_eq!(entry.resource_info.encoded_identity, STRICT);
            assert!(cache.get(&body_key(LOOSE, ApiVersion::V1Beta)).await.is_none());
        }

        /// Dropping an uncacheable implicit-redirect response before polling its Redirect
        /// must not leave any redirect-shaped state that can satisfy a later read.
        /// `Unspecified` exercises the sentinel path rather than the explicit bypass.
        #[test_log::test(tokio::test)]
        async fn dropped_uncacheable_redirect_read_starts_fresh_backend_read() {
            let backend = RedirectBackend::default();
            let service = layered(backend.clone(), "dropped_uncacheable_redirect_starts_fresh").await;

            {
                let mut first_stream = service
                    .read_from_address(read_request_with(DownloadPreference::Unspecified))
                    .await
                    .expect("first read_from_address stream");

                assert!(matches!(
                    first_stream.next().await,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(_)),
                    }))
                ));
                // Drop before polling the redirect.
            }

            let mut second_stream = service
                .read_from_address(read_request_with(DownloadPreference::Unspecified))
                .await
                .expect("second read_from_address stream");

            assert!(matches!(
                second_stream.next().await,
                Some(Ok(ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(_)),
                }))
            ));
            let redirect = second_stream.next().await;
            assert!(
                matches!(
                    redirect,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Redirect(_)),
                    }))
                ),
                "second read should produce a redirect, got {redirect:?}"
            );
            assert_eq!(
                backend.read_from_address_calls.load(Ordering::SeqCst),
                2,
                "second read should call the backend because redirects are uncacheable"
            );
        }

        /// An explicit Redirect request must bypass an already-populated body entry.
        #[test_log::test(tokio::test)]
        async fn explicit_redirect_bypasses_body_cache_coalescing() {
            let backend = RedirectBackend::default();
            let service = layered(backend.clone(), "explicit_redirect_bypasses_body_cache").await;

            let mut body = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("body fill stream");
            let mut saw_body = false;
            while let Some(part) = body.next().await {
                if matches!(
                    part.expect("body response").reply_type,
                    Some(read_from_address_response::ReplyType::Chunk(_))
                ) {
                    saw_body = true;
                }
            }
            assert!(saw_body, "first read should populate a body entry");

            let mut redirect = service
                .read_from_address(read_request_with(DownloadPreference::Redirect))
                .await
                .expect("explicit redirect stream");
            let mut saw_redirect = false;
            while let Some(part) = redirect.next().await {
                match part.expect("redirect response").reply_type {
                    Some(read_from_address_response::ReplyType::Redirect(_)) => saw_redirect = true,
                    Some(read_from_address_response::ReplyType::Chunk(_)) => {
                        panic!("explicit Redirect must not be satisfied by cached body bytes")
                    },
                    _ => {},
                }
            }
            assert!(saw_redirect, "explicit request should return a fresh redirect");

            assert_eq!(
                backend.read_from_address_calls.load(Ordering::SeqCst),
                2,
                "explicit Redirect must call the backend despite the cached body"
            );
        }

        /// An implicit-redirect response (Unspecified preference, backend chooses redirect)
        /// must not populate the body cache, so a subsequent read calls the backend.
        #[test_log::test(tokio::test)]
        async fn implicit_redirect_is_not_cached() {
            let backend = RedirectBackend::default();
            let service = layered(backend.clone(), "implicit_redirect_is_not_cached").await;

            {
                let mut stream = service
                    .read_from_address(read_request_with(DownloadPreference::Unspecified))
                    .await
                    .expect("first stream");
                // Drain the complete passthrough response.
                assert!(stream.next().await.is_some());
                assert!(stream.next().await.is_some());
            }

            let _ = service
                .read_from_address(read_request_with(DownloadPreference::Unspecified))
                .await
                .expect("second stream");

            assert_eq!(
                backend.read_from_address_calls.load(Ordering::SeqCst),
                2,
                "implicit Redirect should not populate the body cache"
            );
        }

        /// Large responses are never inserted in the body cache. Each read must receive
        /// its own complete response, including its request-specific redirect URL.
        #[test_log::test(tokio::test)]
        async fn large_object_reads_each_receive_full_redirect_response() {
            let backend = RedirectBackend::default();
            let service = layered(backend.clone(), "large_object_reads_each_receive_full_redirect").await;

            // Large responses return from read_internal with the fetched ResourceInfo
            // stashed in front of the untouched remainder of each backend stream.
            let mut first = service
                .read_from_address(read_request_with(DownloadPreference::Unspecified))
                .await
                .expect("first stream");
            let mut second = service
                .read_from_address(read_request_with(DownloadPreference::Unspecified))
                .await
                .expect("second stream");

            let first_info = first.next().await;
            assert!(
                matches!(
                    first_info,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(_)),
                    }))
                ),
                "first read should see ResourceInfo, got {first_info:?}"
            );
            let first_redirect = first.next().await;
            assert!(
                matches!(
                    first_redirect,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Redirect(_)),
                    }))
                ),
                "first read should see Redirect, got {first_redirect:?}"
            );

            let second_info = second.next().await;
            assert!(
                matches!(
                    second_info,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(_)),
                    }))
                ),
                "second read should see ResourceInfo, got {second_info:?}"
            );
            let second_redirect = second.next().await;
            assert!(
                matches!(
                    second_redirect,
                    Some(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Redirect(_)),
                    }))
                ),
                "second read should see Redirect, got {second_redirect:?}"
            );

            assert_eq!(
                backend.read_from_address_calls.load(Ordering::SeqCst),
                2,
                "uncacheable reads should each call the backend"
            );
        }

        /// Backend that reports `data_object_size` honestly but emits chunks totalling
        /// `actual_bytes`. Used to exercise the body-vs-declared-size integrity check.
        #[derive(Clone)]
        struct SizedBodyBackend {
            read_from_address_calls: Arc<AtomicUsize>,
            declared_size: u64,
            chunks: Vec<&'static [u8]>,
        }

        #[async_trait]
        impl FileObject for SizedBodyBackend {
            fn inner(&self) -> Option<&dyn FileObject> {
                None
            }
            fn backend(&self) -> &'static str {
                "sized-body-test"
            }
            async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
                Ok(StatResponse {
                    resource_info: Some(ResourceInfoV1Beta {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: "https://example.com/test-resource-identity".to_string(),
                        }),
                        metadata: Some(Metadata {
                            data_object_size: Some(self.declared_size),
                            last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                        }),
                    }),
                })
            }
            async fn read_from_address(
                &self,
                _request: Request<ReadFromAddressRequest>,
            ) -> Result<BoxStream<ReadFromAddressResponse>> {
                self.read_from_address_calls.fetch_add(1, Ordering::SeqCst);
                let mut responses: Vec<Result<ReadFromAddressResponse>> = vec![Ok(ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(
                        ResourceInfoV1Beta {
                            resource_identity: Some(ResourceIdentity {
                                encoded_identity: "https://example.com/test-resource-identity".to_string(),
                            }),
                            metadata: Some(Metadata {
                                data_object_size: Some(self.declared_size),
                                last_modified_timestamp: Some(Timestamp { seconds: 1, nanos: 0 }),
                            }),
                        },
                    )),
                })];
                for chunk in &self.chunks {
                    responses.push(Ok(ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Chunk(
                            crate::traits::fileobject::v1beta::Chunk {
                                chunk: Bytes::from_static(chunk),
                            },
                        )),
                    }));
                }
                Ok(Box::pin(futures::stream::iter(responses)))
            }
        }

        /// A backend that emits more bytes than its declared `data_object_size` must not
        /// cache the mismatched body. The runner receives its original response from the
        /// local stash, while complete ResourceInfo remains available as a metadata hint.
        #[test_log::test(tokio::test)]
        async fn response_exceeding_declared_size_is_not_cached_as_body() {
            let backend = SizedBodyBackend {
                read_from_address_calls: Arc::new(AtomicUsize::new(0)),
                declared_size: 4,
                // Declared 4 bytes, sends 6 — the integrity check must fire on chunk 2.
                chunks: vec![b"abcd", b"ef"],
            };
            let calls = backend.read_from_address_calls.clone();

            let small_object_cache_layer = SmallObjectCacheLayer::new(Config::default(), None, Some("oversized_body"))
                .await
                .expect("layer")
                .expect("enabled");
            let cache = small_object_cache_layer.cache.clone();
            let stat_cache_layer = StatCacheLayer::new(StatCacheConfig::default(), None, Some("oversized_body"))
                .await
                .expect("stat layer")
                .expect("stat enabled");
            let service = stat_cache_layer.layer(small_object_cache_layer.layer(backend));

            // Drain the original response recovered from the runner's local stash.
            let mut stream = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("first stream");
            while stream.next().await.is_some() {}
            drop(stream);

            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let entry = cache
                .get(&meta_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta,
                ))
                .await
                .expect("complete ResourceInfo should be cached as a metadata hint")
                .expect("cached value must not be an error");
            assert!(entry.bytes.is_none());
            assert!(
                cache
                    .get(&body_key(
                        "https://example.com/test-resource-identity",
                        ApiVersion::V1Beta
                    ))
                    .await
                    .is_none(),
                "mismatched response must not be cached as a body entry"
            );
        }

        /// A clean EOF before the declared size must pass through exactly but
        /// never be retained as a complete body.
        #[test_log::test(tokio::test)]
        async fn response_shorter_than_declared_size_is_not_cached_as_body() {
            let backend = SizedBodyBackend {
                read_from_address_calls: Arc::new(AtomicUsize::new(0)),
                declared_size: 6,
                chunks: vec![b"abcd"],
            };
            let calls = backend.read_from_address_calls.clone();
            let small_object_cache_layer = SmallObjectCacheLayer::new(Config::default(), None, Some("undersized_body"))
                .await
                .expect("layer")
                .expect("enabled");
            let cache = small_object_cache_layer.cache.clone();
            let stat_cache_layer = StatCacheLayer::new(StatCacheConfig::default(), None, Some("undersized_body"))
                .await
                .expect("stat layer")
                .expect("stat enabled");
            let service = stat_cache_layer.layer(small_object_cache_layer.layer(backend));

            let mut first = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("short first stream");
            let mut declared_size = None;
            let mut body = BytesMut::new();
            while let Some(part) = first.next().await {
                match part.expect("short response part").reply_type {
                    Some(read_from_address_response::ReplyType::ResourceInfo(info)) => {
                        declared_size = info.metadata.and_then(|metadata| metadata.data_object_size);
                    },
                    Some(read_from_address_response::ReplyType::Chunk(chunk)) => {
                        body.extend_from_slice(&chunk.chunk);
                    },
                    _ => {},
                }
            }
            assert_eq!(declared_size, Some(6));
            assert_eq!(body.freeze(), Bytes::from_static(b"abcd"));
            assert!(cache
                .get(&body_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta
                ))
                .await
                .is_none());
            let metadata = cache
                .get(&meta_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta,
                ))
                .await
                .expect("complete ResourceInfo should remain as a hint")
                .expect("metadata hint should not be an error");
            assert!(metadata.bytes.is_none());

            let mut second = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("short second stream");
            while second.next().await.is_some() {}
            assert_eq!(calls.load(Ordering::SeqCst), 2, "short body must be refetched");
            assert!(cache
                .get(&body_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta
                ))
                .await
                .is_none());
        }

        /// A zero-byte body must round-trip as a body cache entry — otherwise we'd
        /// always re-fetch zero-length files from the backend even though we have
        /// authoritative knowledge that they're empty.
        #[test_log::test(tokio::test)]
        async fn zero_byte_body_is_cached_as_body_entry() {
            let backend = SizedBodyBackend {
                read_from_address_calls: Arc::new(AtomicUsize::new(0)),
                declared_size: 0,
                chunks: vec![],
            };
            let calls = backend.read_from_address_calls.clone();

            let small_object_cache_layer = SmallObjectCacheLayer::new(Config::default(), None, Some("zero_byte_body"))
                .await
                .expect("layer")
                .expect("enabled");
            let cache = small_object_cache_layer.cache.clone();
            let stat_cache_layer = StatCacheLayer::new(StatCacheConfig::default(), None, Some("zero_byte_body"))
                .await
                .expect("stat layer")
                .expect("stat enabled");
            let service = stat_cache_layer.layer(small_object_cache_layer.layer(backend));

            let mut stream = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("first stream");
            while stream.next().await.is_some() {}
            drop(stream);

            let cached = cache
                .get(&body_key(
                    "https://example.com/test-resource-identity",
                    ApiVersion::V1Beta,
                ))
                .await;
            let entry = cached
                .expect("zero-byte read must populate the cache")
                .expect("cached value must not be an error");
            assert_eq!(
                entry.bytes,
                Some(Bytes::new()),
                "zero-byte body must cache as Some(empty Bytes), not None"
            );

            let mut second = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("cached zero-byte read");
            let mut saw_empty_chunk = false;
            while let Some(response) = second.next().await {
                if let Some(read_from_address_response::ReplyType::Chunk(chunk)) =
                    response.expect("cached zero-byte response").reply_type
                {
                    assert!(chunk.chunk.is_empty());
                    saw_empty_chunk = true;
                }
            }
            assert!(
                saw_empty_chunk,
                "cached zero-byte body should round-trip as an empty chunk"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "the second zero-byte read should be served from the body cache"
            );
        }

        /// A passthrough stream error must not populate the body cache, so the next read
        /// retries against the backend.
        #[test_log::test(tokio::test)]
        async fn stream_error_is_not_cached() {
            let backend = ErrorBackend::default();
            let service = layered(backend.clone(), "stream_error_is_not_cached").await;

            {
                let mut stream = service
                    .read_from_address(read_request_with(DownloadPreference::Body))
                    .await
                    .expect("first stream");
                assert!(matches!(stream.next().await, Some(Ok(_))));
                assert!(matches!(stream.next().await, Some(Err(_))));
            }

            let _ = service
                .read_from_address(read_request_with(DownloadPreference::Body))
                .await
                .expect("second stream");

            assert_eq!(
                backend.read_from_address_calls.load(Ordering::SeqCst),
                2,
                "second read after error should hit the backend"
            );
        }
    }
}
