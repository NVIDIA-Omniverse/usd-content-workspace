// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, sync::Arc, time::Duration};

use ahash::RandomState;
use bytesize::ByteSize;
use figment::Figment;
use fred::clients::Pool;
use get_size2::GetSize;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use storage_core_util::{info_limited, warn_limited};
use url::Url;

use storage_core_builder::{layer::Layer, traits::Result};
use storage_core_protos::google::protobuf::Timestamp;
use storage_core_util::cache::{
    CachedSize, CachedValue, LayerBuildError, LayerCache, LayerCacheArgs, RedisConfig, SecretsGeneration,
};

use super::Error;

/// Configuration for the list cache.
///
/// The configuration key is `list_cache` when extracting from a Figment struct.
///
/// Note is not recommended to use < 512MB capacity in production environments.
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
    #[serde(default = "default_errors")]
    pub errors: Vec<String>,
    #[serde(default)]
    pub invalidate_on_update: bool,
    #[serde(default = "default_page_size_limit")]
    pub page_size_limit: ByteSize,
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
    Duration::from_secs(15 * 60)
}

fn default_errors() -> Vec<String> {
    vec!["AccessDenied".to_string(), "NotFound".to_string()]
}

fn default_page_size_limit() -> ByteSize {
    ByteSize::mib(3)
}

fn default_capacity_warn_pct() -> u8 {
    storage_core_util::cache::default_capacity_warn_pct()
}

impl Config {
    pub const CONFIG_KEY: &str = "list_cache";
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
            page_size_limit: default_page_size_limit(),
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

/// Layer for the list cache.
pub struct ListCacheLayer {
    cache: Arc<CacheMap>,
    page_size_limit: ByteSize,
}

/// Errors returned during construction of a `ListCacheLayer`.
pub type BuildError = LayerBuildError;

impl ListCacheLayer {
    pub const LAYER_NAME: &'static str = "list_cache";

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

        if config.capacity < ByteSize::mib(512) {
            warn_limited!("Cache capacity is less than 512 MiB. This is not recommended for production environments.");
        }

        if config.page_size_limit > ByteSize::mib(3) {
            info_limited!("Page size limit is greater than 3 MiB. This is not recommended for production environments unless gRPC is configured to allow larger packets.");
        }

        let page_size_limit = config.page_size_limit;
        let namespace = match namespace_prefix {
            Some(prefix) => format!("{prefix}:{}", Self::LAYER_NAME),
            None => Self::LAYER_NAME.to_owned(),
        };
        let args: LayerCacheArgs = config.into();
        let cache = LayerCache::new(&args, Self::LAYER_NAME, namespace, redis, generation).await?;

        Ok(Some(Self {
            cache: Arc::new(cache),
            page_size_limit,
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

impl<S> Layer<S> for ListCacheLayer {
    type Service = ListCache<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ListCache::new(inner, self.cache.clone(), self.page_size_limit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum ListItem {
    FolderAddress(String),
    ListItem {
        resource_address: String,
        resource_identity: String,
        data_object_size: Option<u64>,
        #[serde(with = "storage_core_util::serde_helpers::option_timestamp")]
        last_modified_timestamp: Option<Timestamp>,
        /// Backend-native metadata for the listed object (populated by list_stat only when
        /// `native_metadata_enabled`; Azure returns it inline, S3 via the gated HEAD fan-out).
        /// Postcard-safe (prost-encoded values) so it round-trips through the L2 (Redis) cache.
        #[serde(default, with = "storage_core_util::serde_helpers::native_metadata_map")]
        native_metadata: std::collections::HashMap<String, storage_core_protos::google::protobuf::Value>,
    },
}

/// Newtype wrapper over the indexed list map. Local-to-this-crate so we can
/// impl [`CachedSize`] / [`CachedValue`]; orphan rules disallow the impl on
/// the bare `IndexMap<...>` alias.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ListMap(pub(crate) IndexMap<String, ListItem, RandomState>);

impl ListMap {
    pub fn with_hasher(hasher: RandomState) -> Self {
        Self(IndexMap::with_hasher(hasher))
    }
}

impl std::ops::Deref for ListMap {
    type Target = IndexMap<String, ListItem, RandomState>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ListMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Per-entry size for the cached list map. Reuses the existing
/// `get-size2::GetSize` impl on `ListItem` (`v1alpha::filefolder.rs`) to
/// estimate inline + heap retention. The map can hold 20K+ entries — the
/// O(n) iteration runs once per L1 insert and is acceptable since inserts
/// are rare relative to hits.
impl CachedSize for ListMap {
    fn cached_size(&self) -> u32 {
        let mut total = std::mem::size_of::<Self>();
        for (k, v) in self.0.iter() {
            total += std::mem::size_of::<String>() + k.len() + v.get_size();
            // get_size cannot see the prost-encoded native_metadata values; add them explicitly.
            if let ListItem::ListItem { native_metadata, .. } = v {
                total += storage_core_util::api_types::native_metadata_size(native_metadata);
            }
        }
        u32::try_from(total).unwrap_or(u32::MAX)
    }
}

impl CachedValue for ListMap {
    // Bumped to 2: cache::ListItem now carries native_metadata (postcard-safe, prost-encoded values),
    // a wire-incompatible shape change for any Redis (L2) entries written under schema = 1.
    const SCHEMA: u32 = 2;
}

pub(crate) type CacheMap = LayerCache<Arc<ListMap>>;

pub struct ListCache<S> {
    pub inner: S,
    pub(crate) cache: Arc<CacheMap>,
    pub(crate) page_size_limit: ByteSize,
}

impl<S> ListCache<S> {
    fn new(inner: S, cache: Arc<CacheMap>, page_size_limit: ByteSize) -> Self {
        Self {
            inner,
            cache,
            page_size_limit,
        }
    }

    pub async fn invalidate_cache_entry_by_object_url<T>(&self, object_url: T) -> Result<()>
    where
        T: TryInto<Url>,
        T::Error: std::error::Error + 'static,
    {
        let keys = Self::ancestor_keys(object_url)?;
        self.invalidate_ancestors(&keys).await;

        Ok(())
    }

    pub(crate) fn ancestor_keys<T>(object_url: T) -> Result<Vec<String>>
    where
        T: TryInto<Url>,
        T::Error: std::error::Error + 'static,
    {
        let mut keys = Vec::new();
        let mut url = Self::parent_url(object_url)?;
        loop {
            keys.push(Self::key(&url)?.into_owned());
            if url.path() == "/" {
                break;
            }
            url = Self::parent_url(url)?;
        }
        Ok(keys)
    }

    pub(crate) async fn invalidate_ancestors(&self, keys: &[String]) {
        for key in keys {
            self.cache.invalidate(key.as_str()).await;
        }
    }

    pub(crate) fn parent_url<T>(resource_url: T) -> Result<Url>
    where
        T: TryInto<Url>,
        T::Error: std::error::Error + 'static,
    {
        let mut url = resource_url
            .try_into()
            .map_err(|e| Error::invalid_argument(e.to_string()))?;

        url.set_fragment(None);
        url.set_query(None);

        url.path_segments_mut()
            .map_err(|_| Error::invalid_argument("Unable to parse URL path segments: cannot-be-a-base"))?
            .pop_if_empty()
            .pop()
            .push("");

        Ok(url)
    }

    pub(crate) fn key<'a>(resource_url: &'a Url) -> Result<Cow<'a, str>> {
        Ok(percent_encoding::percent_decode_str(resource_url.as_str())
            .decode_utf8()
            .map_err(|e| Error::invalid_argument(e.to_string()))?)
    }

    pub(crate) fn ensure_trailing_slash(folder_address: Option<&String>) -> Result<Url> {
        match folder_address {
            Some(address) => {
                let mut url = Url::parse(address).map_err(|e| Error::invalid_argument(e.to_string()))?;
                if !url.path().ends_with('/') {
                    url.path_segments_mut()
                        .map_err(|_| Error::invalid_argument("Unable to parse URL path segments: cannot-be-a-base"))?
                        .push("");
                }
                url.set_fragment(None);

                if url.query_pairs().any(|(k, _)| k == "version-id") {
                    return Err(Error::invalid_argument("Versioned folder addresses are not supported").into());
                }
                url.set_query(None);
                Ok(url)
            },
            None => Err(Error::invalid_argument("Folder address is required").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parent_url() {
        let url = Url::parse("https://example.com/folder/subfolder/resource.txt").unwrap();
        let parent_url = ListCache::<()>::parent_url(url).unwrap();
        assert_eq!(parent_url.to_string(), "https://example.com/folder/subfolder/");
    }

    #[test]
    fn test_parent_url_with_trailing_slash() {
        let url = Url::parse("https://example.com/folder/subfolder/").unwrap();
        let parent_url = ListCache::<()>::parent_url(url).unwrap();
        assert_eq!(parent_url.to_string(), "https://example.com/folder/");
    }

    #[test]
    fn test_parent_url_with_trailing_slash_slash() {
        let url = Url::parse("https://example.com/folder/subfolder//").unwrap();
        let parent_url = ListCache::<()>::parent_url(url).unwrap();
        assert_eq!(parent_url.to_string(), "https://example.com/folder/subfolder/");
    }

    #[test]
    fn test_parent_url_with_query_params() {
        let url = Url::parse("https://example.com/folder/subfolder/?version-id=123").unwrap();
        let parent_url = ListCache::<()>::parent_url(url).unwrap();
        assert_eq!(parent_url.to_string(), "https://example.com/folder/");
    }

    #[test]
    fn test_parent_url_with_fragment() {
        let url = Url::parse("https://example.com/folder/subfolder/#fragment").unwrap();
        let parent_url = ListCache::<()>::parent_url(url).unwrap();
        assert_eq!(parent_url.to_string(), "https://example.com/folder/");
    }

    #[test]
    fn test_ensure_trailing_slash() {
        let url = Url::parse("https://example.com/folder/subfolder").unwrap();
        let url_with_slash = ListCache::<()>::ensure_trailing_slash(Some(&url.to_string())).unwrap();
        assert_eq!(url_with_slash.to_string(), "https://example.com/folder/subfolder/");
    }

    #[test]
    fn test_ensure_trailing_slash_with_invalid_query_params() {
        let url = Url::parse("https://example.com/folder/subfolder?version-id=123").unwrap();
        let result = ListCache::<()>::ensure_trailing_slash(Some(&url.to_string()));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "invalid argument: Versioned folder addresses are not supported"
        );
    }

    #[test]
    fn test_ensure_trailing_slash_with_query_params() {
        let url = Url::parse("https://example.com/folder/subfolder?query=param").unwrap();
        let url_with_slash = ListCache::<()>::ensure_trailing_slash(Some(&url.to_string())).unwrap();
        assert_eq!(url_with_slash.to_string(), "https://example.com/folder/subfolder/");
    }

    #[test]
    fn unit_ensure_trailing_slash_with_fragment() {
        let url = Url::parse("https://example.com/folder/subfolder?query=param#fraglerock").unwrap();
        let url_with_slash = ListCache::<()>::ensure_trailing_slash(Some(&url.to_string())).unwrap();
        assert_eq!(url_with_slash.to_string(), "https://example.com/folder/subfolder/");
    }

    #[test]
    fn test_ancestor_keys_deep_path() {
        let keys = ListCache::<()>::ancestor_keys("https://example-bucket.s3.us-west-2.amazonaws.com/a/b/c/image.png")
            .unwrap();
        assert_eq!(
            keys,
            vec![
                "https://example-bucket.s3.us-west-2.amazonaws.com/a/b/c/",
                "https://example-bucket.s3.us-west-2.amazonaws.com/a/b/",
                "https://example-bucket.s3.us-west-2.amazonaws.com/a/",
                "https://example-bucket.s3.us-west-2.amazonaws.com/",
            ]
        );
    }

    #[test]
    fn test_ancestor_keys_single_level() {
        let keys =
            ListCache::<()>::ancestor_keys("https://example-bucket.s3.us-west-2.amazonaws.com/image.png").unwrap();
        assert_eq!(keys, vec!["https://example-bucket.s3.us-west-2.amazonaws.com/"]);
    }

    #[test]
    fn test_ancestor_keys_folder_url() {
        let keys = ListCache::<()>::ancestor_keys("https://example-bucket.s3.us-west-2.amazonaws.com/a/b/").unwrap();
        assert_eq!(
            keys,
            vec![
                "https://example-bucket.s3.us-west-2.amazonaws.com/a/",
                "https://example-bucket.s3.us-west-2.amazonaws.com/",
            ]
        );
    }

    #[test]
    fn test_ancestor_keys_encoded_url() {
        let keys = ListCache::<()>::ancestor_keys(
            "https://example-bucket.s3.us-west-2.amazonaws.com/path%20with%20spaces/file.txt",
        )
        .unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(
            keys[0],
            "https://example-bucket.s3.us-west-2.amazonaws.com/path with spaces/"
        );
        assert_eq!(keys[1], "https://example-bucket.s3.us-west-2.amazonaws.com/");
    }

    #[test]
    fn test_ancestor_keys_azure_url() {
        let keys = ListCache::<()>::ancestor_keys(
            "https://exampleaccountprivate.blob.core.windows.net/unittests/folder/subfolder/resource.txt",
        )
        .unwrap();
        assert_eq!(
            keys,
            vec![
                "https://exampleaccountprivate.blob.core.windows.net/unittests/folder/subfolder/",
                "https://exampleaccountprivate.blob.core.windows.net/unittests/folder/",
                "https://exampleaccountprivate.blob.core.windows.net/unittests/",
                "https://exampleaccountprivate.blob.core.windows.net/",
            ]
        );
    }

    #[test]
    fn key_coalesces_loose_and_strict_path_spellings() {
        const LOOSE: &str = "!$%&'()*+,:;=@[]^|";
        const STRICT: &str = "%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";

        let loose = Url::parse(&format!("https://example.com/{LOOSE}/?marker=a%26b")).unwrap();
        let strict = Url::parse(&format!("https://example.com/{STRICT}/?marker=a%26b")).unwrap();

        let loose_key = ListCache::<()>::key(&loose).unwrap();
        let strict_key = ListCache::<()>::key(&strict).unwrap();
        assert_eq!(loose_key, strict_key);
        assert_eq!(ListCache::<()>::key(&strict).unwrap(), strict_key);
    }
}
