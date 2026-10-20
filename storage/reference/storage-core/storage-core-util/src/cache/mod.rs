// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

mod api_version;
#[allow(clippy::module_inception)]
mod cache;
mod envelope;
mod error;
mod expiry;
mod layer_build_error;
mod layer_cache;
mod lua;
mod pubsub;
mod redis_config;
mod secrets_generation;
mod serializable_error;
mod sized;

pub use api_version::ApiVersion;
pub use cache::{Cache as LowLevelCache, CacheBuilder};
pub use error::Error as LowLevelError;
pub use layer_build_error::LayerBuildError;
pub use layer_cache::{default_capacity_warn_pct, CacheArgs as LayerCacheArgs, CacheErrorPolicy, LayerCache};
pub use redis_config::{
    build_client as build_redis_client, build_client_from_figment as build_redis_client_from_figment,
    build_client_with_config as build_redis_client_with_config, config_from_address as redis_config_from_address,
    BuildClientError as RedisBuildClientError, RedisConfig,
};
pub use secrets_generation::SecretsGeneration;
pub use serializable_error::SerializableCachedError;
pub use sized::{CachedSize, CachedValue};

/// Per-request cache directive carried in the request `http::Extensions` — analogous to an HTTP
/// `Cache-Control` header. Caching layers inspect it to decide whether to serve from / store into
/// their cache for a given request. Deliberately an enum (and `#[non_exhaustive]`) so it can grow
/// real-world directives (e.g. `NoStore`, `MaxAge`) — match on the variant, not on mere presence.
///
/// The manifest layer sets [`CacheControl::NoCache`] on its accesses to the reserved manifest object
/// so the caches below it never hold a second, stale-able copy (which would otherwise survive a
/// manifest rewrite when those caches are deployed with `invalidate_on_update = false`). Insert with
/// `extensions.insert(CacheControl::NoCache)`; check with
/// `extensions.get::<CacheControl>().is_some_and(CacheControl::bypasses_cache)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CacheControl {
    /// Do not serve from or store into the cache for this request — delegate straight through to the
    /// inner service.
    NoCache,
}

impl CacheControl {
    /// Whether this directive requires the caching layers to bypass their cache entirely (no serve,
    /// no store) for the request.
    pub fn bypasses_cache(&self) -> bool {
        matches!(self, CacheControl::NoCache)
    }
}
