// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc, time::Duration};

use bytesize::ByteSize;
use figment::Figment;
use fred::clients::Pool;
use serde::{Deserialize, Serialize};

use storage_core_builder::layer::Layer;
use storage_core_builder::traits::metadata::v1alpha::Value;
use storage_core_util::cache::{
    CachedSize, CachedValue, LayerBuildError, LayerCache, LayerCacheArgs, RedisConfig, SecretsGeneration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub enabled: bool,
    /// Enable the optional Redis L2 tier when a shared `[redis]` section and client are present.
    #[serde(default)]
    pub l2_enabled: bool,
    #[serde(default = "default_capacity")]
    pub capacity: ByteSize,
    #[serde(default = "default_time_to_live", with = "humantime_serde")]
    pub time_to_live: Duration,
    #[serde(default = "default_errors")]
    pub errors: Vec<String>,
    #[serde(default)]
    pub invalidate_on_update: bool,
    #[serde(default = "default_normalize_keys")]
    pub normalize_keys: bool,
    #[serde(default = "default_capacity_warn_pct")]
    pub capacity_warn_pct: u8,
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

fn default_normalize_keys() -> bool {
    true
}

fn default_capacity_warn_pct() -> u8 {
    storage_core_util::cache::default_capacity_warn_pct()
}

impl Config {
    pub const CONFIG_KEY: &str = "metadata_cache";
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
            enabled: false,
            l2_enabled: false,
            capacity: default_capacity(),
            time_to_live: default_time_to_live(),
            errors: default_errors(),
            invalidate_on_update: false,
            normalize_keys: default_normalize_keys(),
            capacity_warn_pct: default_capacity_warn_pct(),
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

/// Layer for the metadata cache.
pub struct MetadataCacheLayer {
    pub cache: Arc<LayerCache<CachedMetadata>>,
    pub normalize_keys: bool,
}

/// Errors returned during construction of a `MetadataCacheLayer`.
pub type BuildError = LayerBuildError;

impl MetadataCacheLayer {
    pub const LAYER_NAME: &'static str = "metadata_cache";

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
        let normalize_keys = config.normalize_keys;
        let namespace = match namespace_prefix {
            Some(prefix) => format!("{prefix}:{}", Self::LAYER_NAME),
            None => Self::LAYER_NAME.to_owned(),
        };
        let args: LayerCacheArgs = config.into();
        let cache = LayerCache::new(&args, Self::LAYER_NAME, namespace, redis, generation).await?;
        Ok(Some(Self {
            cache: Arc::new(cache),
            normalize_keys,
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

impl<S> Layer<S> for MetadataCacheLayer {
    type Service = MetadataCache<S>;
    fn layer(&self, inner: S) -> Self::Service {
        MetadataCache::new(inner, self.cache.clone(), self.normalize_keys)
    }
}

/// A single cached metadata entry for one key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMetadataEntry {
    #[serde(with = "storage_core_util::serde_helpers::option_proto_value")]
    pub value: Option<Value>,
    pub etag: String,
}

/// Cached metadata snapshot for a single URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMetadata(pub HashMap<String, CachedMetadataEntry>);

impl CachedSize for CachedMetadata {
    fn cached_size(&self) -> u32 {
        // Coarse: per-entry estimate covers the keys, etag, and an opaque
        // 256-byte allowance for a typical proto Value. Refine if the metric
        // shows it under-counting.
        let mut total = std::mem::size_of::<Self>();
        for (k, v) in self.0.iter() {
            total += std::mem::size_of::<String>() + k.len();
            total += std::mem::size_of::<CachedMetadataEntry>() + v.etag.len() + 256;
        }
        u32::try_from(total).unwrap_or(u32::MAX)
    }
}

impl CachedValue for CachedMetadata {
    const SCHEMA: u32 = 1;
}

pub struct MetadataCache<S> {
    pub inner: S,
    pub cache: Arc<LayerCache<CachedMetadata>>,
    pub normalize_keys: bool,
}

impl<S> MetadataCache<S> {
    fn new(inner: S, cache: Arc<LayerCache<CachedMetadata>>, normalize_keys: bool) -> Self {
        Self {
            inner,
            cache,
            normalize_keys,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use storage_core_builder::layer::Layer;
    use storage_core_builder::traits::{
        async_trait,
        error::Result,
        metadata::v1alpha::{
            update_metadata_batch_entry_result, GetMetadataRequest, GetMetadataResponse, Metadata,
            UpdateMetadataBatchEntryResult, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse,
            UpdateMetadataRequest, UpdateMetadataResponse,
        },
        Request,
    };

    use super::*;

    #[test]
    fn cached_metadata_round_trips_through_json() {
        let mut map = HashMap::new();
        map.insert(
            "key".to_string(),
            CachedMetadataEntry {
                value: None,
                etag: "etag-1".to_string(),
            },
        );
        let cm = CachedMetadata(map);
        let json = serde_json::to_string(&cm).expect("serialize");
        let back: CachedMetadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.0.len(), 1);
        assert_eq!(back.0.get("key").unwrap().etag, "etag-1");
    }

    #[derive(Clone)]
    struct CountingMetadata {
        gets: Arc<AtomicUsize>,
        batches: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Metadata for CountingMetadata {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }

        async fn get_metadata(&self, _request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            Ok(GetMetadataResponse {
                user_metadata: HashMap::new(),
            })
        }

        async fn update_metadata(&self, _request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
            Ok(UpdateMetadataResponse {
                etag: "etag".to_string(),
            })
        }

        async fn update_metadata_batch(
            &self,
            request: Request<UpdateMetadataBatchRequest>,
        ) -> Result<UpdateMetadataBatchResponse> {
            self.batches.fetch_add(1, Ordering::SeqCst);
            Ok(UpdateMetadataBatchResponse {
                results: request
                    .get_ref()
                    .requests
                    .iter()
                    .enumerate()
                    .map(|(request_index, _)| UpdateMetadataBatchEntryResult {
                        outcome: Some(update_metadata_batch_entry_result::Outcome::Success(
                            UpdateMetadataResponse {
                                etag: "etag".to_string(),
                            },
                        )),
                        request_index: request_index as u32,
                    })
                    .collect(),
            })
        }
    }

    #[tokio::test]
    async fn path_spelling_aliases_share_cache_and_cross_invalidate() {
        const LOOSE: &str = "https://example.com/!$%&'()*+,:;=@[]^|?version-id=v%2B1";
        const STRICT: &str =
            "https://example.com/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C?version-id=v%2B1";

        let gets = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(AtomicUsize::new(0));
        let config = Config {
            enabled: true,
            invalidate_on_update: true,
            ..Config::default()
        };
        let layer = MetadataCacheLayer::new(config, None, None)
            .await
            .expect("build")
            .expect("enabled");
        let service = layer.layer(CountingMetadata {
            gets: gets.clone(),
            batches,
        });

        for uri in [LOOSE, STRICT] {
            service
                .get_metadata(Request::new(GetMetadataRequest {
                    uri: uri.to_string(),
                    user_metadata_keys: Vec::new(),
                }))
                .await
                .expect("get metadata");
        }
        assert_eq!(gets.load(Ordering::SeqCst), 1);

        service
            .update_metadata(Request::new(UpdateMetadataRequest {
                uri: STRICT.to_string(),
                ..Default::default()
            }))
            .await
            .expect("update metadata");
        service
            .get_metadata(Request::new(GetMetadataRequest {
                uri: LOOSE.to_string(),
                user_metadata_keys: Vec::new(),
            }))
            .await
            .expect("refill metadata");
        assert_eq!(gets.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn invalid_batch_uri_does_not_fail_transport_rpc() {
        let gets = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(AtomicUsize::new(0));
        let layer = MetadataCacheLayer::new(
            Config {
                enabled: true,
                ..Config::default()
            },
            None,
            Some("metadata_invalid_batch_uri"),
        )
        .await
        .expect("build")
        .expect("enabled");
        let service = layer.layer(CountingMetadata {
            gets,
            batches: batches.clone(),
        });

        let response = service
            .update_metadata_batch(Request::new(UpdateMetadataBatchRequest {
                requests: vec![
                    UpdateMetadataRequest {
                        uri: "not a URL".to_string(),
                        ..Default::default()
                    },
                    UpdateMetadataRequest {
                        uri: "https://example.com/object".to_string(),
                        ..Default::default()
                    },
                ],
            }))
            .await
            .expect("the inner batch response should be returned");

        assert_eq!(batches.load(Ordering::SeqCst), 1);
        assert_eq!(response.results.len(), 2);
    }
}
