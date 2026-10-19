// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{sync::Arc, time::Duration};

use super::error::Error;
use super::format::{manifest_cache_key, Manifest, ManifestEntryValue, WorkspaceKey};
use crate::layer::Layer;
use crate::traits::error::{status_with_default_error_info, StorageError, StorageErrorExt};
use bytesize::ByteSize;
use figment::Figment;
use fred::clients::Pool;
use serde::{Deserialize, Serialize};
use storage_core_util::cache::{
    CacheErrorPolicy, CachedSize, CachedValue, LayerBuildError, LayerCache, LayerCacheArgs, RedisConfig,
    SecretsGeneration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Enable the optional Redis L2 tier when a shared `[redis]` section and client are present.
    #[serde(default)]
    pub l2_enabled: bool,
    #[serde(default = "default_error_on_non_manifest_objects")]
    pub error_on_non_manifest_objects: bool,
    #[serde(default = "default_capacity")]
    pub capacity: ByteSize,
    #[serde(default = "default_time_to_live", with = "humantime_serde")]
    pub time_to_live: Duration,
    #[serde(default = "default_page_size_limit")]
    pub page_size_limit: ByteSize,
    #[serde(default = "default_capacity_warn_pct")]
    pub capacity_warn_pct: u8,
}

fn default_enabled() -> bool {
    true
}

fn default_error_on_non_manifest_objects() -> bool {
    false
}

fn default_capacity() -> ByteSize {
    ByteSize::gib(1)
}

fn default_time_to_live() -> Duration {
    Duration::from_secs(60 * 15)
}

fn default_page_size_limit() -> ByteSize {
    ByteSize::mib(3)
}

fn default_capacity_warn_pct() -> u8 {
    storage_core_util::cache::default_capacity_warn_pct()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            l2_enabled: false,
            error_on_non_manifest_objects: default_error_on_non_manifest_objects(),
            capacity: default_capacity(),
            time_to_live: default_time_to_live(),
            page_size_limit: default_page_size_limit(),
            capacity_warn_pct: default_capacity_warn_pct(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "manifest";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

impl From<Config> for LayerCacheArgs {
    fn from(val: Config) -> Self {
        LayerCacheArgs {
            capacity: val.capacity,
            time_to_live: val.time_to_live,
            errors: vec![],
            invalidate_on_update: true,
            capacity_warn_pct: val.capacity_warn_pct,
            tombstone_ttl: None,
            lease_ttl: None,
            l2_enabled: val.l2_enabled,
        }
    }
}

/// Conservative size estimate for a `Manifest`. Drives the L1 weigher;
/// per-entry size is dominated by the relative path string + the
/// `ResourceInfo` payload.
impl CachedSize for Manifest {
    fn cached_size(&self) -> u32 {
        let mut total = std::mem::size_of::<Self>();
        for (k, v) in self.iter() {
            total += std::mem::size_of::<String>() + k.len();
            total += match v {
                ManifestEntryValue::ResourceIdentity(s) => std::mem::size_of::<ManifestEntryValue>() + s.len(),
                ManifestEntryValue::ResourceInfo(info) => {
                    std::mem::size_of::<ManifestEntryValue>()
                        + info.resource_identity.len()
                        + info.modified_time.as_ref().map(|s| s.len()).unwrap_or(0)
                },
            };
        }
        u32::try_from(total).unwrap_or(u32::MAX)
    }
}

// NOTE: `Manifest` intentionally does NOT implement `CachedValue` — the cached value is
// `CachedManifest` (below), so only its `CachedSize` (used by `CachedManifest::Loaded`) is needed.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ManifestState {
    Loaded(Manifest),
    Absent,
    Invalid,
}

/// The cacheable projection of [`ManifestState`]. `Absent` is intentionally **not representable**
/// here: `get_manifest_state` must never cache an absent manifest (a workspace read before its
/// `.manifest` is written would otherwise stay invisible for the whole TTL, including on replicas
/// that never saw the write). Making `Absent` unrepresentable in the stored type enforces that
/// *structurally* — no `LayerCache::insert`, from any holder of the cache `Arc`, can reintroduce a
/// cached `Absent`; that guarantee no longer relies on every call site being disciplined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum CachedManifest {
    Loaded(Manifest),
    Invalid,
}

impl From<CachedManifest> for ManifestState {
    fn from(cached: CachedManifest) -> Self {
        match cached {
            CachedManifest::Loaded(manifest) => ManifestState::Loaded(manifest),
            CachedManifest::Invalid => ManifestState::Invalid,
        }
    }
}

impl CachedSize for CachedManifest {
    fn cached_size(&self) -> u32 {
        match self {
            CachedManifest::Loaded(manifest) => manifest
                .cached_size()
                .saturating_add(std::mem::size_of::<Self>() as u32),
            CachedManifest::Invalid => std::mem::size_of::<Self>() as u32,
        }
    }
}

impl CachedValue for CachedManifest {
    // Bumped 2 -> 3 alongside no-longer-caching `Absent`: a previous binary cached `Absent` (as a
    // schema-2 `ManifestState`), and on L2 (Redis) those entries outlive a rolling upgrade. The
    // schema is part of the L2 envelope, so a mismatch is treated as a miss (see `cache::Cache`);
    // bumping it makes *new* pods ignore any stale schema-2 entry instead of serving it for its TTL.
    // (Old pods keep serving their local-L1 copies until drained or the TTL expires — inherent to a
    // rolling deploy.)
    const SCHEMA: u32 = 3;
}

/// Crate-private sentinel used solely to keep [`ManifestState::Absent`] out of the manifest-state
/// cache. `get_manifest_state` fills the cache under [`CacheErrorPolicy::Never`], returning this
/// error in place of `Absent`, then maps it straight back to `Absent`. Because it is an error filled
/// under `Never`, moka coalesces concurrent readers onto one backend read but never stores it.
///
/// It is deliberately *not* a [`Error`] variant: keeping it private means it can never leak onto the
/// public error surface, and external callers of the public `get_manifest_state` cannot construct it
/// (which would otherwise be silently coerced to `Absent`). Never surfaced to a client.
#[derive(Debug)]
struct AbsentSentinel;

impl std::fmt::Display for AbsentSentinel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("workspace manifest absent (internal sentinel)")
    }
}

impl std::error::Error for AbsentSentinel {}

impl StorageErrorExt for AbsentSentinel {
    fn tonic_status(&self) -> tonic::Status {
        // Never returned to a client; Internal is a safe default should it ever escape.
        status_with_default_error_info(tonic::Code::Internal, self.to_string(), "AbsentSentinel")
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::INTERNAL_SERVER_ERROR
    }

    fn get_size(&self) -> usize {
        0
    }
}

/// Whether `e` is the never-cached [`AbsentSentinel`]. Filled under `CacheErrorPolicy::Never`, the
/// sentinel is returned unwrapped (never stored or serialized), so a direct downcast suffices.
fn is_absent_sentinel(e: &StorageError) -> bool {
    (e.as_ref() as &(dyn std::error::Error + 'static))
        .downcast_ref::<AbsentSentinel>()
        .is_some()
}

/// Layer for the manifest, with a separately configurable downstream resolver.
#[derive(Clone)]
pub struct ManifestLayer<R = ()> {
    resolver: R,
    pub config: Config,
    // The cached value type (`CachedManifest`) cannot represent `Absent`, so no `insert` — from any
    // holder of this `Arc` — can reintroduce a cached `Absent`. Crate-private alongside that.
    pub(crate) manifests: Arc<LayerCache<CachedManifest>>,
    pub page_size_limit: ByteSize,
}

/// Errors returned during construction of a `ManifestLayer`.
pub type BuildError = LayerBuildError;

impl ManifestLayer {
    pub const LAYER_NAME: &'static str = "manifest";

    pub async fn new(
        config: Config,
        redis: Option<Pool>,
        namespace_prefix: Option<&str>,
    ) -> Result<Option<Self>, BuildError> {
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
    ) -> Result<Option<Self>, BuildError> {
        if !config.enabled {
            return Ok(None);
        }
        let page_size_limit = config.page_size_limit;
        let namespace = match namespace_prefix {
            Some(prefix) => format!("{prefix}:{}", Self::LAYER_NAME),
            None => Self::LAYER_NAME.to_owned(),
        };
        let args: LayerCacheArgs = config.clone().into();
        let cache = LayerCache::new(&args, Self::LAYER_NAME, namespace, redis, generation).await?;
        Ok(Some(Self {
            resolver: (),
            config,
            manifests: Arc::new(cache),
            page_size_limit,
        }))
    }

    pub async fn from_figment(figment: &Figment) -> Result<Option<Self>, BuildError> {
        Self::from_figment_with_redis(figment, None).await
    }

    pub async fn from_figment_with_redis(figment: &Figment, redis: Option<Pool>) -> Result<Option<Self>, BuildError> {
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
    ) -> Result<Option<Self>, BuildError> {
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

impl<R> ManifestLayer<R> {
    /// Replaces the downstream resolver while preserving the shared manifest cache.
    ///
    /// Per-API-version specializations must share this same `Arc<LayerCache>` so writes through
    /// one specialization invalidate manifest state cached by the others.
    pub fn with_resolver<R2>(self, resolver: R2) -> ManifestLayer<R2> {
        ManifestLayer {
            resolver,
            config: self.config,
            manifests: self.manifests,
            page_size_limit: self.page_size_limit,
        }
    }
}

impl<S, R: Clone> Layer<S> for ManifestLayer<R> {
    type Service = ManifestService<S, R>;
    fn layer(&self, inner: S) -> Self::Service {
        ManifestService::new(
            inner,
            self.resolver.clone(),
            self.config.clone(),
            self.manifests.clone(),
            self.page_size_limit,
        )
    }
}

/// Manifest service with separate downstreams for pass-through operations and manifest resolution.
pub struct ManifestService<S, R = ()> {
    pub inner: S,
    pub resolver: R,
    pub config: Config,
    // See `ManifestLayer::manifests`.
    pub(crate) manifests: Arc<LayerCache<CachedManifest>>,
    pub page_size_limit: ByteSize,
}

#[derive(Debug, Clone)]
pub enum AddressClass {
    NotWorkspace,
    ManifestFile { key: WorkspaceKey },
    WorkspacePath { key: WorkspaceKey },
}

impl<S, R> ManifestService<S, R> {
    // Crate-private, matching the sibling cache layers (`ListCache`/`StatCache` keep their raw-cache
    // service constructors non-public): the service struct is built by `ManifestLayer` (via
    // `from_figment`/`Layer::layer`), never constructed directly by callers. `pub(crate)` rather than
    // fully private only because the manifest submodules' tests reach it.
    pub(crate) fn new(
        inner: S,
        resolver: R,
        config: Config,
        manifests: Arc<LayerCache<CachedManifest>>,
        page_size_limit: ByteSize,
    ) -> Self {
        Self {
            inner,
            resolver,
            config,
            manifests,
            page_size_limit,
        }
    }
}
impl<S, R> ManifestService<S, R> {
    pub fn classify_address(&self, resource_address: &str) -> crate::traits::error::Result<AddressClass> {
        let Some(key) = WorkspaceKey::parse(resource_address)? else {
            if self.config.error_on_non_manifest_objects {
                return Err(Error::non_manifest_path(resource_address.to_string()).into());
            }
            return Ok(AddressClass::NotWorkspace);
        };

        if key.is_manifest_file() {
            Ok(AddressClass::ManifestFile { key })
        } else {
            Ok(AddressClass::WorkspacePath { key })
        }
    }

    pub async fn get_manifest_state<F, Fut>(
        &self,
        workspace_key: &WorkspaceKey,
        read_fn: F,
    ) -> crate::traits::error::Result<ManifestState>
    where
        F: FnOnce(WorkspaceKey) -> Fut,
        Fut: std::future::Future<Output = crate::traits::error::Result<ManifestState>> + Send,
    {
        // Key on the normalized workspace path so encoding/Unicode variants share one cache entry
        // (see `manifest_cache_key`); backend reads use the canonical `manifest_path` from the key.
        let cache_key = manifest_cache_key(&workspace_key.workspace_path);
        let workspace_key_clone = workspace_key.clone();

        // `Absent` must never be cached. A workspace read or browsed before its `.manifest` exists
        // would otherwise pin a NotFound for the full TTL — and, across service instances, on every
        // replica that did not process the subsequent write — leaving a freshly written manifest
        // invisible for minutes. The stored type ([`CachedManifest`]) can't even represent `Absent`;
        // the fill maps `Absent` to a private [`AbsentSentinel`] error under `CacheErrorPolicy::Never`,
        // so concurrent readers still coalesce onto one backend read but the result is never retained.
        // `Loaded` / `Invalid` cache normally.
        //
        // Residual (bounded) race: a read that starts after a manifest write but joins a still
        // in-flight *pre-write* fill shares that fill's cohort and can observe `Absent` once. Since
        // the sentinel is an error, moka never stores it, so nothing is pinned — the next read
        // refetches. This is inherent to single-flight coalescing and self-heals immediately, unlike
        // the TTL-length staleness a cached `Absent` produced. (Mutation preflights additionally
        // re-read directly before permitting a write — see `mutation_action_for_path`.)
        let result = self
            .manifests
            .try_get_with_by_ref_with_options(
                &cache_key,
                self.manifests.generation_snapshot(),
                CacheErrorPolicy::Never,
                async move {
                    match read_fn(workspace_key_clone).await? {
                        ManifestState::Loaded(manifest) => Ok(CachedManifest::Loaded(manifest)),
                        ManifestState::Invalid => Ok(CachedManifest::Invalid),
                        ManifestState::Absent => Err(Box::new(AbsentSentinel) as StorageError),
                    }
                },
            )
            .await;

        match result {
            Ok(cached) => Ok(cached.into()),
            Err(e) if is_absent_sentinel(e.as_ref()) => Ok(ManifestState::Absent),
            Err(e) => Err(Box::new(Error::cached_error(e)) as StorageError),
        }
    }

    pub async fn invalidate_manifest_state(&self, workspace_path: &str) {
        self.manifests.invalidate(&manifest_cache_key(workspace_path)).await;
    }

    pub fn invalid_manifest_error() -> Error {
        Error::invalid_manifest_entry("Workspace manifest is invalid; only .manifest can be accessed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_cache_invalidation_is_enabled_and_backend_errors_are_not_cached() {
        let args: LayerCacheArgs = Config::default().into();

        assert!(args.invalidate_on_update);
        assert!(args.errors.is_empty());
    }

    #[test]
    fn manifest_default_ttl_is_minutes_scale() {
        assert_eq!(Config::default().time_to_live, Duration::from_secs(60 * 15));
    }

    #[test]
    fn manifest_state_cache_schema_is_bumped() {
        assert_eq!(<CachedManifest as CachedValue>::SCHEMA, 3);
    }

    #[test]
    fn cached_manifest_cannot_represent_absent() {
        // Compile-time guarantee, spelled out: `CachedManifest` has exactly `Loaded`/`Invalid`, so
        // no code path (including `LayerCache::insert` on any holder of the `Arc`) can cache `Absent`.
        fn assert_total(cached: CachedManifest) -> ManifestState {
            match cached {
                CachedManifest::Loaded(m) => ManifestState::Loaded(m),
                CachedManifest::Invalid => ManifestState::Invalid,
            }
        }
        assert!(matches!(assert_total(CachedManifest::Invalid), ManifestState::Invalid));
    }

    #[tokio::test]
    async fn cloned_layer_with_resolver_shares_manifest_cache() {
        let layer = ManifestLayer::new(Config::default(), None, None)
            .await
            .expect("build")
            .expect("enabled");
        let specialized = layer.clone().with_resolver("resolver");

        assert!(Arc::ptr_eq(&layer.manifests, &specialized.manifests));
    }

    #[tokio::test]
    async fn with_resolver_preserves_config_and_page_size_limit() {
        let config = Config {
            enabled: true,
            l2_enabled: false,
            error_on_non_manifest_objects: true,
            capacity: ByteSize::mib(7),
            time_to_live: Duration::from_secs(123),
            page_size_limit: ByteSize::kib(19),
            capacity_warn_pct: 71,
        };
        let layer = ManifestLayer::new(config.clone(), None, None)
            .await
            .expect("build")
            .expect("enabled");
        let specialized = layer.with_resolver(42_u8);

        assert_eq!(specialized.resolver, 42);
        assert_eq!(specialized.page_size_limit, config.page_size_limit);
        assert_eq!(specialized.config.enabled, config.enabled);
        assert_eq!(specialized.config.l2_enabled, config.l2_enabled);
        assert_eq!(
            specialized.config.error_on_non_manifest_objects,
            config.error_on_non_manifest_objects
        );
        assert_eq!(specialized.config.capacity, config.capacity);
        assert_eq!(specialized.config.time_to_live, config.time_to_live);
        assert_eq!(specialized.config.page_size_limit, config.page_size_limit);
        assert_eq!(specialized.config.capacity_warn_pct, config.capacity_warn_pct);
    }

    #[test]
    fn absent_sentinel_is_detected_and_other_errors_are_not() {
        let sentinel: StorageError = Box::new(AbsentSentinel);
        assert!(is_absent_sentinel(&sentinel));

        // A genuine backend failure reaching the cache fill must not be mistaken for the sentinel,
        // otherwise it would be silently swallowed as `Absent`.
        let real: StorageError = Box::new(Error::internal("not the sentinel"));
        assert!(!is_absent_sentinel(&real));

        let cached_real: StorageError = Box::new(Error::cached_error(Arc::new(Box::new(Error::internal("boom")))));
        assert!(!is_absent_sentinel(&cached_real));
    }

    /// A secrets hot-reload that changes the endpoint policy must repartition the
    /// manifest cache, so a manifest state cached under the previous policy is no
    /// longer served.
    #[tokio::test]
    async fn secrets_generation_change_repartitions_manifest_cache() {
        let generation = SecretsGeneration::new(None);
        let layer = ManifestLayer::new_with_generation(
            Config {
                enabled: true,
                ..Config::default()
            },
            None,
            None,
            Some(generation.clone()),
        )
        .await
        .expect("build")
        .expect("enabled");

        layer
            .manifests
            .insert("k".to_string(), Ok(CachedManifest::Invalid))
            .await;
        assert!(layer.manifests.get_two_tier("k").await.is_some());

        generation.update(Some(
            &Figment::new().join(("https://a.example".to_string(), "key".to_string())),
        ));

        assert!(layer.manifests.get_two_tier("k").await.is_none());
    }
}
