// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{sync::Arc, time::Duration};

use bytesize::ByteSize;
use figment::Figment;
use fred::clients::Pool;
use serde::{Deserialize, Serialize};

use storage_core_builder::layer::Layer;
use storage_core_protos::google::protobuf::Timestamp;
use storage_core_util::cache::{
    CachedSize, CachedValue, LayerBuildError, LayerCache, LayerCacheArgs, RedisConfig, SecretsGeneration,
};

/// Configuration for the stat cache.
///
/// The configuration key is `stat_cache` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [stat_cache]
/// enabled = true
/// capacity = "1GB"
/// time_to_live = "15m"
/// errors = ["AccessDenied", "NotFound"]
/// invalidate_on_update = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the stat cache.
    /// Default is true
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Enable the optional Redis L2 tier when a shared `[redis]` section and client are present.
    /// Default is false
    #[serde(default)]
    pub l2_enabled: bool,

    /// Capacity size in bytes that the cache can store before applying eviction policy.
    /// Default is 1GB
    #[serde(default = "default_capacity")]
    pub capacity: ByteSize,

    /// Maximum time that objects can exist in the cache. They may be evicted sooner if the cache is full
    /// Default is 15 minutes
    #[serde(default = "default_time_to_live", with = "humantime_serde")]
    pub time_to_live: Duration,

    /// List of errors to allow in the cache. @see storage_service_util::error::SdkError
    /// Default is AccessDenied,NotFound
    #[serde(default = "default_errors")]
    pub errors: Vec<String>,

    /// When enabled, any write, copy, or delete operation will invalidate the cache for the affected object.
    /// Default is false
    #[serde(default)]
    pub invalidate_on_update: bool,

    /// Log a warning when cache utilization exceeds this percentage (0-100, 0 to disable). Default: 95.
    #[serde(default = "default_capacity_warn_pct")]
    pub capacity_warn_pct: u8,
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

fn default_capacity_warn_pct() -> u8 {
    storage_core_util::cache::default_capacity_warn_pct()
}

impl Config {
    pub const CONFIG_KEY: &str = "stat_cache";
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
            invalidate_on_update: false,
            capacity_warn_pct: default_capacity_warn_pct(),
            l2_enabled: false,
        }
    }
}

impl From<Config> for LayerCacheArgs {
    fn from(val: Config) -> Self {
        LayerCacheArgs {
            capacity: val.capacity,
            time_to_live: val.time_to_live,
            errors: val.errors,
            invalidate_on_update: val.invalidate_on_update,
            capacity_warn_pct: val.capacity_warn_pct,
            tombstone_ttl: None,
            lease_ttl: None,
            l2_enabled: val.l2_enabled,
        }
    }
}

/// Layer for the stat cache.
pub struct StatCacheLayer {
    pub cache: Arc<LayerCache<CachedStatResponse>>,
}

/// Errors returned during construction of a `StatCacheLayer`.
pub type BuildError = LayerBuildError;

impl StatCacheLayer {
    pub const LAYER_NAME: &'static str = "stat_cache";

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
        let namespace = match namespace_prefix {
            Some(prefix) => format!("{prefix}:{}", Self::LAYER_NAME),
            None => Self::LAYER_NAME.to_owned(),
        };
        let args: LayerCacheArgs = config.into();
        let cache = LayerCache::new(&args, Self::LAYER_NAME, namespace, redis, generation).await?;
        Ok(Some(Self { cache: Arc::new(cache) }))
    }

    /// Creates a new StatCacheLayer from a Figment struct, with no L2 cache.
    pub async fn from_figment(figment: &Figment) -> Result<Option<Self>, BuildError> {
        Self::from_figment_with_redis(figment, None).await
    }

    /// Creates a new StatCacheLayer from a Figment struct, optionally enabling
    /// the L2 (Redis) cache when `[redis]` is configured, `redis` is `Some`,
    /// and `stat_cache.l2_enabled = true`.
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

impl<S> Layer<S> for StatCacheLayer {
    type Service = StatCache<S>;
    fn layer(&self, inner: S) -> Self::Service {
        StatCache::new(inner, self.cache.clone())
    }
}

/// A more generic struct to store in the cache so that it can serve both v1alpha and v1beta versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedStatResponse {
    pub encoded_identity: String,
    pub data_object_size: Option<u64>,
    #[serde(with = "storage_core_util::serde_helpers::option_timestamp")]
    pub last_modified_timestamp: Option<Timestamp>,
    /// FileObject-native metadata surfaced by the backend stat (already gated on
    /// `native_metadata_enabled` at the source). Client-controlled strings, so it is
    /// accounted for in `cached_size`.
    #[serde(default, with = "storage_core_util::serde_helpers::native_metadata_map")]
    pub native_metadata: std::collections::HashMap<String, storage_core_protos::google::protobuf::Value>,
}

impl CachedSize for CachedStatResponse {
    fn cached_size(&self) -> u32 {
        let native_metadata = storage_core_util::api_types::native_metadata_size(&self.native_metadata);
        let body = self.encoded_identity.len()
            + std::mem::size_of::<Option<u64>>()
            + std::mem::size_of::<Option<Timestamp>>()
            + native_metadata;
        u32::try_from(std::mem::size_of::<Self>() + body).unwrap_or(u32::MAX)
    }
}

impl CachedValue for CachedStatResponse {
    // v2: added native_metadata (postcard-safe, prost-encoded values) to the cached value.
    const SCHEMA: u32 = 2;
}

pub struct StatCache<S> {
    pub inner: S,
    pub cache: Arc<LayerCache<CachedStatResponse>>,
}

impl<S> StatCache<S> {
    fn new(inner: S, cache: Arc<LayerCache<CachedStatResponse>>) -> Self {
        Self { inner, cache }
    }
}

#[cfg(test)]
mod nocache_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use storage_core_builder::layer::Layer;
    use storage_core_builder::traits::{
        async_trait,
        error::Result,
        fileobject::v1beta::{
            DeleteRequest, DeleteResponse, FileObject, Metadata, ResourceIdentity, ResourceInfo, StatRequest,
            StatResponse,
        },
        Request,
    };
    use storage_core_util::cache::CacheControl;

    use super::{Config, StatCacheLayer};

    #[derive(Clone)]
    struct CountingStat {
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl FileObject for CountingStat {
        fn inner(&self) -> Option<&dyn FileObject> {
            None
        }

        async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(StatResponse {
                resource_info: Some(ResourceInfo {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: request.get_ref().resource_address.clone(),
                    }),
                    metadata: Some(Metadata {
                        data_object_size: Some(1),
                        last_modified_timestamp: None,
                    }),
                }),
            })
        }

        async fn delete(&self, _request: Request<DeleteRequest>) -> Result<DeleteResponse> {
            Ok(DeleteResponse {})
        }
    }

    fn stat_request(address: &str, no_cache: bool) -> Request<StatRequest> {
        let mut request = Request::new(StatRequest {
            resource_address: address.to_string(),
        });
        if no_cache {
            request.extensions_mut().insert(CacheControl::NoCache);
        }
        request
    }

    #[tokio::test]
    async fn nocache_extension_bypasses_stat_cache() {
        let count = Arc::new(AtomicUsize::new(0));
        let layer = StatCacheLayer::new(Config::default(), None, None)
            .await
            .expect("build")
            .expect("enabled");
        let service = layer.layer(CountingStat { count: count.clone() });

        // Without NoCache, repeated stats of the same key are served from the cache → one inner call.
        service
            .stat(stat_request("https://example.com/a", false))
            .await
            .expect("stat");
        service
            .stat(stat_request("https://example.com/a", false))
            .await
            .expect("stat");
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // With NoCache, each stat bypasses the cache entirely → one inner call per request.
        service
            .stat(stat_request("https://example.com/a", true))
            .await
            .expect("stat");
        service
            .stat(stat_request("https://example.com/a", true))
            .await
            .expect("stat");
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    /// End-to-end: a secrets hot-reload that changes the endpoint policy must
    /// repartition the stat cache, so a value cached under the previous policy
    /// is no longer served — exercised through the real `StatCache` layer.
    #[tokio::test]
    async fn secrets_generation_change_repartitions_stat_cache() {
        use storage_core_util::cache::SecretsGeneration;

        let count = Arc::new(AtomicUsize::new(0));
        let generation = SecretsGeneration::new(None);
        let layer = StatCacheLayer::new_with_generation(Config::default(), None, None, Some(generation.clone()))
            .await
            .expect("build")
            .expect("enabled");
        let service = layer.layer(CountingStat { count: count.clone() });

        // First stat populates the cache; the second is served from it → one inner call.
        service
            .stat(stat_request("https://example.com/a", false))
            .await
            .expect("stat");
        service
            .stat(stat_request("https://example.com/a", false))
            .await
            .expect("stat");
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // A secrets hot-reload changes the policy (e.g. the endpoint is revoked or added).
        generation.update(Some(
            &figment::Figment::new().join(("https://a.example".to_string(), "key".to_string())),
        ));

        // The entry cached under the previous generation is unreachable, so the inner
        // service is invoked again instead of serving the now-stale cached response.
        service
            .stat(stat_request("https://example.com/a", false))
            .await
            .expect("stat");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn path_spelling_aliases_share_cache_and_cross_invalidate() {
        const LOOSE: &str = "https://example.com/!$%&'()*+,:;=@[]^|?version-id=v%2B1";
        const STRICT: &str =
            "https://example.com/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C?version-id=v%2B1";

        let count = Arc::new(AtomicUsize::new(0));
        let config = Config {
            invalidate_on_update: true,
            ..Config::default()
        };
        let layer = StatCacheLayer::new(config, None, None)
            .await
            .expect("build")
            .expect("enabled");
        let service = layer.layer(CountingStat { count: count.clone() });

        let first = service.stat(stat_request(LOOSE, false)).await.expect("loose stat");
        let second = service.stat(stat_request(STRICT, false)).await.expect("strict stat");
        assert_eq!(count.load(Ordering::SeqCst), 1);

        for response in [first, second] {
            let identity = response
                .resource_info
                .and_then(|info| info.resource_identity)
                .expect("identity");
            assert_eq!(identity.encoded_identity, STRICT);
        }

        service
            .delete(Request::new(DeleteRequest {
                resource_address: STRICT.to_string(),
                ..Default::default()
            }))
            .await
            .expect("delete");
        service.stat(stat_request(LOOSE, false)).await.expect("refill");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
