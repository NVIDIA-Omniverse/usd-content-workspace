// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Configuration for the L2 (Redis/Valkey) cache and helpers that construct
//! a connected `fred` client from a user-supplied address or figment block.

use std::time::Duration;

use figment::Figment;
use fred::clients::Pool;
use fred::interfaces::ClientLike;
use fred::types::config::{Config, PerformanceConfig, ReconnectPolicy, ServerConfig};
use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// Default Redis command timeout in milliseconds. Bounds single-command blast
/// radius for callers that use `RedisConfig::default()` or omit the field from
/// config. Set `command_timeout_ms = 0` explicitly to disable the timeout and
/// use fred's unbounded default.
///
/// 250ms (not a tighter value) so a single large value — a `list_cache`
/// listing or a small-object body sharing a pooled connection — does not
/// trip a false timeout under load. The connection pool spreads commands so
/// head-of-line blocking is rare, but the tail still needs headroom.
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 250;

/// Default number of connections in the Redis client pool. fred multiplexes
/// commands over each connection, so a single connection head-of-line-blocks
/// every small command queued behind a large value. A pool of connections
/// round-robins commands so one slow/large command cannot stall the rest.
const DEFAULT_POOL_SIZE: usize = 8;

/// Default connect timeout in milliseconds. Caps `wait_for_connect` so a
/// misconfigured address fails fast rather than hanging service startup.
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;

/// Default capacity of the fred subscriber broadcast channel. The default is
/// 32, which is easy to overflow when O(N) `invalidate_entries_if` fans out;
/// 1024 leaves headroom without a meaningful memory cost.
const DEFAULT_BROADCAST_CHANNEL_CAPACITY: usize = 1024;

/// Default reconnect policy: exponential backoff with unlimited attempts.
/// In fred, `max_attempts = 0` means retry forever.
const DEFAULT_RECONNECT_MAX_ATTEMPTS: u32 = 0;
const DEFAULT_RECONNECT_MIN_DELAY_MS: u32 = 100;
const DEFAULT_RECONNECT_MAX_DELAY_MS: u32 = 30_000;
const DEFAULT_RECONNECT_BASE: u32 = 2;
const DEFAULT_RECONNECT_JITTER_MS: u32 = 50;
/// Upper bound on the configured reconnect cap, to catch a units typo (e.g. `30000000` meaning to
/// write `30000`) that would otherwise strand a reconnect for hours. 10 minutes is far above any
/// sane backoff cap for a cache client.
const MAX_RECONNECT_MAX_DELAY_MS: u32 = 600_000;

/// Figment configuration for the optional L2 (Redis) cache. The figment key
/// is `redis`. When this section is absent, layers run in L1-only mode.
/// Layers also require their own `l2_enabled = true` setting before using the
/// shared Redis client.
///
/// Example:
/// ```toml
/// [redis]
/// address = "redis://127.0.0.1:6379"
/// # or: address = "127.0.0.1:6379"
/// namespace = "my-service"      # required prefix for all L2 keys
/// command_timeout_ms = 250      # caps individual Redis commands
/// connect_timeout_ms = 10000    # caps initial wait_for_connect
/// pool_size = 8                 # number of multiplexed connections
/// reconnect_max_attempts = 0    # 0 retries forever
/// reconnect_min_delay_ms = 100
/// reconnect_max_delay_ms = 30000
/// reconnect_base = 2
/// reconnect_jitter_ms = 50
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis/Valkey address. Accepts either a `redis://...` / `rediss://...`
    /// URL or a bare `host:port` pair.
    pub address: String,

    /// Namespace prefix applied to every key written to L2. Required when the
    /// service publishes a Redis client to its layers — a missing namespace
    /// would let two services share a keyspace and serve each other's
    /// cached responses. Optional only because Some test rigs build
    /// `RedisConfig` ad-hoc; the runtime check enforces it where it matters.
    #[serde(default)]
    pub namespace: Option<String>,

    /// Per-command timeout in milliseconds. Defaults to 250ms to bound the
    /// blast radius of a degraded-but-reachable Redis while leaving tail
    /// headroom for large values on a pooled connection. Recommended range:
    /// 100–500ms. Set explicitly to `0` to disable the timeout (the fred
    /// default).
    #[serde(default = "default_command_timeout_ms")]
    pub command_timeout_ms: u64,

    /// Connect timeout in milliseconds; caps `wait_for_connect`. Default: 10s.
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,

    /// Number of connections in the client pool. fred pipelines commands over
    /// each connection, so more connections reduce head-of-line blocking when
    /// large values (listings, object bodies) share the pool with small
    /// metadata commands. Clamped to at least 1. Default: 8.
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    /// Capacity of the fred subscriber broadcast channel. Larger values reduce
    /// the chance that a fan-out of invalidations overflows the channel; the
    /// trade-off is a small fixed memory cost per subscriber. Default: 1024.
    #[serde(default = "default_broadcast_channel_capacity")]
    pub broadcast_channel_capacity: usize,

    /// Maximum reconnect attempts after an established Redis connection drops.
    /// `0` means retry forever. Default: 0.
    #[serde(default = "default_reconnect_max_attempts")]
    pub reconnect_max_attempts: u32,

    /// Initial exponential reconnect delay in milliseconds. Default: 100ms.
    #[serde(default = "default_reconnect_min_delay_ms")]
    pub reconnect_min_delay_ms: u32,

    /// Maximum exponential reconnect delay in milliseconds. Default: 30s.
    #[serde(default = "default_reconnect_max_delay_ms")]
    pub reconnect_max_delay_ms: u32,

    /// Exponential reconnect multiplier. Must be greater than 1. Default: 2.
    #[serde(default = "default_reconnect_base")]
    pub reconnect_base: u32,

    /// Jitter added to each reconnect delay in milliseconds. Default: 50ms.
    #[serde(default = "default_reconnect_jitter_ms")]
    pub reconnect_jitter_ms: u32,
}

fn default_command_timeout_ms() -> u64 {
    DEFAULT_COMMAND_TIMEOUT_MS
}

fn default_connect_timeout_ms() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_MS
}

fn default_pool_size() -> usize {
    DEFAULT_POOL_SIZE
}

fn default_broadcast_channel_capacity() -> usize {
    DEFAULT_BROADCAST_CHANNEL_CAPACITY
}

fn default_reconnect_max_attempts() -> u32 {
    DEFAULT_RECONNECT_MAX_ATTEMPTS
}

fn default_reconnect_min_delay_ms() -> u32 {
    DEFAULT_RECONNECT_MIN_DELAY_MS
}

fn default_reconnect_max_delay_ms() -> u32 {
    DEFAULT_RECONNECT_MAX_DELAY_MS
}

fn default_reconnect_base() -> u32 {
    DEFAULT_RECONNECT_BASE
}

fn default_reconnect_jitter_ms() -> u32 {
    DEFAULT_RECONNECT_JITTER_MS
}

impl RedisConfig {
    pub const CONFIG_KEY: &str = "redis";

    pub fn namespace_prefix(&self) -> Option<&str> {
        self.namespace.as_deref().map(str::trim).filter(|ns| !ns.is_empty())
    }

    pub fn required_namespace(&self) -> Result<&str, BuildClientError> {
        self.namespace_prefix().ok_or(BuildClientError::MissingNamespace)
    }

    pub fn command_timeout(&self) -> Duration {
        Duration::from_millis(self.command_timeout_ms)
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }

    /// Pool size, clamped to a minimum of 1 (an empty pool cannot be built).
    pub fn effective_pool_size(&self) -> usize {
        self.pool_size.max(1)
    }

    pub fn reconnect_policy(&self) -> Result<ReconnectPolicy, BuildClientError> {
        if self.reconnect_min_delay_ms == 0 {
            return Err(BuildClientError::InvalidReconnectConfig {
                message: "reconnect_min_delay_ms must be greater than 0".to_string(),
            });
        }
        if self.reconnect_max_delay_ms < self.reconnect_min_delay_ms {
            return Err(BuildClientError::InvalidReconnectConfig {
                message: "reconnect_max_delay_ms must be greater than or equal to reconnect_min_delay_ms".to_string(),
            });
        }
        if self.reconnect_base <= 1 {
            return Err(BuildClientError::InvalidReconnectConfig {
                message: "reconnect_base must be greater than 1".to_string(),
            });
        }
        if self.reconnect_max_delay_ms > MAX_RECONNECT_MAX_DELAY_MS {
            return Err(BuildClientError::InvalidReconnectConfig {
                message: format!(
                    "reconnect_max_delay_ms ({}) exceeds the {}ms ceiling; likely a units typo (milliseconds)",
                    self.reconnect_max_delay_ms, MAX_RECONNECT_MAX_DELAY_MS
                ),
            });
        }

        let mut policy = ReconnectPolicy::new_exponential(
            self.reconnect_max_attempts,
            self.reconnect_min_delay_ms,
            self.reconnect_max_delay_ms,
            self.reconnect_base,
        );
        policy.set_jitter(self.reconnect_jitter_ms);
        Ok(policy)
    }
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            address: String::new(),
            namespace: None,
            command_timeout_ms: default_command_timeout_ms(),
            connect_timeout_ms: default_connect_timeout_ms(),
            pool_size: default_pool_size(),
            broadcast_channel_capacity: default_broadcast_channel_capacity(),
            reconnect_max_attempts: default_reconnect_max_attempts(),
            reconnect_min_delay_ms: default_reconnect_min_delay_ms(),
            reconnect_max_delay_ms: default_reconnect_max_delay_ms(),
            reconnect_base: default_reconnect_base(),
            reconnect_jitter_ms: default_reconnect_jitter_ms(),
        }
    }
}

/// Errors returned when building a Redis client from configuration.
#[derive(Debug, Snafu)]
pub enum BuildClientError {
    #[snafu(display("redis address parse error: {source}"))]
    Parse { source: fred::error::Error },

    #[snafu(display("redis connect failed: {source}"))]
    Connect { source: fred::error::Error },

    #[snafu(display("redis connect timed out after {timeout_ms}ms"))]
    ConnectTimeout { timeout_ms: u64 },

    #[snafu(display("redis figment extract failed: {source}"))]
    Figment { source: figment::Error },

    #[snafu(display("redis namespace is required when Redis L2 is configured"))]
    MissingNamespace,

    #[snafu(display("redis reconnect config is invalid: {message}"))]
    InvalidReconnectConfig { message: String },
}

/// Parse `address` into a `fred::types::config::Config`. Accepts either a
/// `redis://` / `rediss://` URL or a bare `host:port` pair.
pub fn config_from_address(address: &str) -> Result<Config, fred::error::Error> {
    if address.starts_with("redis://") || address.starts_with("rediss://") {
        Config::from_url(address)
    } else {
        let (host, port) = parse_host_port(address)?;
        Ok(Config {
            server: ServerConfig::new_centralized(host, port),
            ..Config::default()
        })
    }
}

fn parse_host_port(address: &str) -> Result<(String, u16), fred::error::Error> {
    let (host, port_str) = address.rsplit_once(':').ok_or_else(|| {
        fred::error::Error::new(
            fred::error::ErrorKind::Config,
            format!("invalid redis address (expected host:port or redis:// URL): {address}"),
        )
    })?;
    let port: u16 = port_str.parse().map_err(|_| {
        fred::error::Error::new(
            fred::error::ErrorKind::Config,
            format!("invalid port in redis address: {address}"),
        )
    })?;
    Ok((host.to_owned(), port))
}

/// Build a connected `fred` client from a user-supplied address with default
/// timeout configuration. The client is connected before this function
/// returns.
pub async fn build_client(address: &str) -> Result<Pool, fred::error::Error> {
    let cfg = RedisConfig {
        address: address.to_owned(),
        ..RedisConfig::default()
    };
    build_client_with_config(&cfg).await.map_err(|e| match e {
        BuildClientError::Parse { source } | BuildClientError::Connect { source } => source,
        BuildClientError::ConnectTimeout { timeout_ms } => fred::error::Error::new(
            fred::error::ErrorKind::Timeout,
            format!("connect timed out after {timeout_ms}ms"),
        ),
        BuildClientError::Figment { .. } => fred::error::Error::new(fred::error::ErrorKind::Config, "figment"),
        BuildClientError::MissingNamespace => {
            fred::error::Error::new(fred::error::ErrorKind::Config, "missing redis namespace")
        },
        BuildClientError::InvalidReconnectConfig { message } => fred::error::Error::new(
            fred::error::ErrorKind::Config,
            format!("invalid reconnect config: {message}"),
        ),
    })
}

/// Build a connected `fred` client pool from a `RedisConfig`. Applies the
/// configured command timeout via `PerformanceConfig`, opens `pool_size`
/// round-robin connections, and bounds the initial `wait_for_connect` (across
/// all connections) with `connect_timeout_ms`.
pub async fn build_client_with_config(cfg: &RedisConfig) -> Result<Pool, BuildClientError> {
    let config = config_from_address(&cfg.address).map_err(|source| BuildClientError::Parse { source })?;
    // NOTE: do not set `config.fail_fast = true` — it converts transient
    // connection resets during startup or pub/sub init into immediate hard
    // errors instead of letting fred's reconnect policy retry. The earlier
    // tighter default broke service-fixture init under nextest concurrency.
    let perf = PerformanceConfig {
        default_command_timeout: cfg.command_timeout(),
        broadcast_channel_capacity: cfg.broadcast_channel_capacity,
        ..PerformanceConfig::default()
    };
    let reconnect_policy = cfg.reconnect_policy()?;
    // A pool rather than a single client: fred pipelines commands over each
    // connection, so one connection lets a large value (a listing or object
    // body) head-of-line-block the small metadata commands queued behind it.
    // Round-robining across connections keeps a slow command from stalling the
    // rest. `effective_pool_size()` clamps to >=1 so `Pool::new` cannot fail
    // on an empty pool.
    let pool = Pool::new(
        config,
        Some(perf),
        None,
        Some(reconnect_policy),
        cfg.effective_pool_size(),
    )
    .map_err(|source| BuildClientError::Connect { source })?;
    pool.connect();
    let connect = tokio::time::timeout(cfg.connect_timeout(), pool.wait_for_connect())
        .await
        .map_err(|_| BuildClientError::ConnectTimeout {
            timeout_ms: cfg.connect_timeout_ms,
        })?;
    connect.map_err(|source| BuildClientError::Connect { source })?;
    Ok(pool)
}

/// Build a Redis client from the optional `[redis]` figment block. Returns
/// `Ok(None)` when no `[redis]` section is present (L1-only mode); returns
/// `Ok(Some(client))` connected to Redis when present and reachable. Each
/// layer still has to opt in with its own `l2_enabled = true` setting.
///
/// Production wiring should call this once at service startup and pass the
/// resulting `Option<Pool>` to each layer's `from_figment_with_redis`.
pub async fn build_client_from_figment(figment: &Figment) -> Result<Option<Pool>, BuildClientError> {
    let cfg: Option<RedisConfig> = figment.focus(RedisConfig::CONFIG_KEY).extract().ok();
    let Some(cfg) = cfg else {
        return Ok(None);
    };
    if cfg.address.is_empty() {
        return Ok(None);
    }
    cfg.required_namespace()?;
    let client = build_client_with_config(&cfg).await?;
    Ok(Some(client))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_namespace_rejects_missing_or_blank_namespace() {
        let mut cfg = RedisConfig {
            address: "127.0.0.1:6379".to_string(),
            ..RedisConfig::default()
        };

        assert!(matches!(
            cfg.required_namespace(),
            Err(BuildClientError::MissingNamespace)
        ));

        cfg.namespace = Some("   ".to_string());
        assert!(matches!(
            cfg.required_namespace(),
            Err(BuildClientError::MissingNamespace)
        ));
    }

    #[tokio::test]
    async fn build_client_from_figment_rejects_redis_without_namespace() {
        use figment::providers::{Format, Toml};

        let figment = Figment::new().merge(Toml::string(
            r#"
            [redis]
            address = "127.0.0.1:6379"
            "#,
        ));

        let result = build_client_from_figment(&figment).await;
        assert!(matches!(result, Err(BuildClientError::MissingNamespace)));
    }

    #[test]
    fn parses_url_form() {
        let cfg = config_from_address("redis://127.0.0.1:6379").unwrap();
        assert!(cfg.server.is_centralized());
    }

    #[test]
    fn parses_host_port_form() {
        let cfg = config_from_address("127.0.0.1:6379").unwrap();
        assert!(cfg.server.is_centralized());
    }

    #[test]
    fn rejects_missing_port() {
        assert!(config_from_address("127.0.0.1").is_err());
    }

    #[test]
    fn rejects_non_numeric_port() {
        assert!(config_from_address("127.0.0.1:abc").is_err());
    }

    #[test]
    fn default_command_timeout_bounds_commands() {
        let cfg = RedisConfig::default();

        assert_eq!(cfg.command_timeout_ms, 250);
        assert_eq!(cfg.command_timeout(), Duration::from_millis(250));
    }

    #[test]
    fn serde_uses_bounded_command_timeout_when_field_is_absent() {
        let cfg: RedisConfig = serde_json::from_str(r#"{"address":"127.0.0.1:6379"}"#).unwrap();

        assert_eq!(cfg.command_timeout_ms, 250);
    }

    #[test]
    fn default_pool_size_is_multi_connection() {
        let cfg = RedisConfig::default();

        assert_eq!(cfg.pool_size, 8);
        assert_eq!(cfg.effective_pool_size(), 8);
    }

    #[test]
    fn effective_pool_size_clamps_zero_to_one() {
        let cfg = RedisConfig {
            address: "127.0.0.1:6379".to_string(),
            pool_size: 0,
            ..RedisConfig::default()
        };

        assert_eq!(cfg.effective_pool_size(), 1);
    }

    #[test]
    fn serde_uses_default_pool_size_when_field_is_absent() {
        let cfg: RedisConfig = serde_json::from_str(r#"{"address":"127.0.0.1:6379"}"#).unwrap();

        assert_eq!(cfg.pool_size, 8);
    }

    #[test]
    fn serde_preserves_explicit_disabled_command_timeout() {
        let cfg: RedisConfig = serde_json::from_str(r#"{"address":"127.0.0.1:6379","command_timeout_ms":0}"#).unwrap();

        assert_eq!(cfg.command_timeout_ms, 0);
        assert_eq!(cfg.command_timeout(), Duration::from_millis(0));
    }

    #[test]
    fn default_reconnect_policy_retries_forever_with_exponential_backoff() {
        let cfg = RedisConfig::default();
        let policy = cfg.reconnect_policy().expect("default reconnect policy");

        assert_eq!(cfg.reconnect_max_attempts, 0);
        match policy {
            ReconnectPolicy::Exponential {
                max_attempts,
                min_delay,
                max_delay,
                base,
                jitter,
                ..
            } => {
                assert_eq!(max_attempts, 0);
                assert_eq!(min_delay, 100);
                assert_eq!(max_delay, 30_000);
                assert_eq!(base, 2);
                assert_eq!(jitter, 50);
            },
            other => panic!("unexpected reconnect policy: {other:?}"),
        }
    }

    #[test]
    fn serde_preserves_reconnect_overrides() {
        let cfg: RedisConfig = serde_json::from_str(
            r#"{
                "address":"127.0.0.1:6379",
                "reconnect_max_attempts":7,
                "reconnect_min_delay_ms":25,
                "reconnect_max_delay_ms":500,
                "reconnect_base":3,
                "reconnect_jitter_ms":0
            }"#,
        )
        .unwrap();
        let policy = cfg.reconnect_policy().expect("custom reconnect policy");

        match policy {
            ReconnectPolicy::Exponential {
                max_attempts,
                min_delay,
                max_delay,
                base,
                jitter,
                ..
            } => {
                assert_eq!(max_attempts, 7);
                assert_eq!(min_delay, 25);
                assert_eq!(max_delay, 500);
                assert_eq!(base, 3);
                assert_eq!(jitter, 0);
            },
            other => panic!("unexpected reconnect policy: {other:?}"),
        }
    }

    #[test]
    fn reconnect_policy_rejects_invalid_values() {
        let mut cfg = RedisConfig {
            reconnect_min_delay_ms: 0,
            ..Default::default()
        };
        assert!(matches!(
            cfg.reconnect_policy(),
            Err(BuildClientError::InvalidReconnectConfig { .. })
        ));

        cfg = RedisConfig::default();
        cfg.reconnect_min_delay_ms = 500;
        cfg.reconnect_max_delay_ms = 100;
        assert!(matches!(
            cfg.reconnect_policy(),
            Err(BuildClientError::InvalidReconnectConfig { .. })
        ));

        cfg = RedisConfig::default();
        cfg.reconnect_base = 1;
        assert!(matches!(
            cfg.reconnect_policy(),
            Err(BuildClientError::InvalidReconnectConfig { .. })
        ));

        // A likely units typo (milliseconds vs the intended value) is rejected, not silently accepted.
        cfg = RedisConfig::default();
        cfg.reconnect_max_delay_ms = MAX_RECONNECT_MAX_DELAY_MS + 1;
        assert!(matches!(
            cfg.reconnect_policy(),
            Err(BuildClientError::InvalidReconnectConfig { .. })
        ));
    }
}
