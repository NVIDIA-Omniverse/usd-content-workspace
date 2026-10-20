// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! PostgreSQL connection pool construction with optional TLS.
//!
//! The PostgreSQL password may be a short-lived token read from a JSON token
//! file that an external sidecar rotates periodically. Because
//! `deadpool-postgres` bakes the password into the pool manager at creation, a
//! rotated token requires building a fresh pool and swapping it in atomically;
//! existing connections stay valid because PostgreSQL validates credentials
//! only at connection time.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use deadpool_postgres::{
    Config as DeadpoolConfig, ManagerConfig, Pool, PoolConfig, RecyclingMethod, Runtime,
    SslMode as DeadpoolSslMode,
};
use rustls::RootCertStore;
use serde::Deserialize;
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{error, info, warn};

use crate::config::{PostgresConfig, PostgresSslMode};

/// Error returned while building a `rustls`-backed connector for PostgreSQL.
#[derive(Debug)]
pub struct TlsConnectorError(String);

impl std::fmt::Display for TlsConnectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TlsConnectorError {}

impl TlsConnectorError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Error returned while reading a short-lived PostgreSQL password from a token
/// file.
#[derive(Debug)]
pub enum PostgresTokenError {
    Io(std::io::Error),
    Parse(String),
    Empty,
    Expired(DateTime<Utc>),
}

impl std::fmt::Display for PostgresTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "read token file: {e}"),
            Self::Parse(msg) => write!(f, "parse token file: {msg}"),
            Self::Empty => f.write_str("token file access_token is empty"),
            Self::Expired(at) => write!(f, "token expired at {at}"),
        }
    }
}

impl std::error::Error for PostgresTokenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PostgresTokenError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Error returned while building the snapshot-store connection pool.
#[derive(Debug)]
pub enum PoolBuildError {
    Tls(TlsConnectorError),
    Deadpool(deadpool_postgres::CreatePoolError),
    Token(PostgresTokenError),
}

impl std::fmt::Display for PoolBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls(e) => write!(f, "TLS configuration: {e}"),
            Self::Deadpool(e) => write!(f, "connection pool: {e}"),
            Self::Token(e) => write!(f, "password token: {e}"),
        }
    }
}

impl std::error::Error for PoolBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tls(e) => Some(e),
            Self::Deadpool(e) => Some(e),
            Self::Token(e) => Some(e),
        }
    }
}

impl From<TlsConnectorError> for PoolBuildError {
    fn from(e: TlsConnectorError) -> Self {
        Self::Tls(e)
    }
}

impl From<deadpool_postgres::CreatePoolError> for PoolBuildError {
    fn from(e: deadpool_postgres::CreatePoolError) -> Self {
        Self::Deadpool(e)
    }
}

impl From<PostgresTokenError> for PoolBuildError {
    fn from(e: PostgresTokenError) -> Self {
        Self::Token(e)
    }
}

/// Build a `rustls`-backed TLS connector for tokio-postgres using the
/// operating system's native root store.
pub fn build_tls_connector() -> Result<MakeRustlsConnect, TlsConnectorError> {
    let mut roots = RootCertStore::empty();

    let loaded = rustls_native_certs::load_native_certs();
    for err in &loaded.errors {
        warn!(error = %err, "failed to load a native root certificate");
    }
    if loaded.certs.is_empty() {
        return Err(TlsConnectorError::new(
            "no native root certificates could be loaded",
        ));
    }
    for cert in loaded.certs {
        roots
            .add(cert)
            .map_err(|e| TlsConnectorError::new(format!("add native cert: {e}")))?;
    }

    let tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| TlsConnectorError::new(format!("TLS protocol versions: {e}")))?
    .with_root_certificates(roots)
    .with_no_client_auth();

    Ok(MakeRustlsConnect::new(tls_config))
}

/// Build the PostgreSQL connection pool, resolving the password from the token
/// file when configured and otherwise from the static `password` field.
///
/// When `ssl_mode` is `Disable`, the pool uses plaintext connections. Any other mode builds
/// a `rustls`-backed connector using the operating system's native trust store.
pub async fn create_snapshot_pool(pg: &PostgresConfig) -> Result<Pool, PoolBuildError> {
    let password = resolve_initial_password(pg).await?;
    build_pool(pg, password)
}

/// Build the PostgreSQL connection pool with an explicit password, used when a
/// rotated token replaces the previous credential.
pub fn create_snapshot_pool_with_password(
    pg: &PostgresConfig,
    password: &str,
) -> Result<Pool, PoolBuildError> {
    build_pool(pg, Some(password.to_string()))
}

/// Spawn a background task that watches the configured token file and rebuilds
/// the pool when the password rotates. Does nothing when no token file is
/// configured (static password mode).
pub fn spawn_token_refresher(pg: &PostgresConfig, pool: Arc<ArcSwap<Pool>>) {
    let token_file = match &pg.token_file {
        Some(path) => path.clone(),
        None => return,
    };
    let poll_interval = pg.token_file_poll_interval;
    let pg = pg.clone();
    tokio::spawn(async move {
        run_token_watch_loop(pg, token_file, poll_interval, pool).await;
    });
}

fn build_pool(pg: &PostgresConfig, password: Option<String>) -> Result<Pool, PoolBuildError> {
    let cfg = build_pool_config(pg, password);

    let pool = match pg.ssl_mode {
        PostgresSslMode::Disable => cfg.create_pool(Some(Runtime::Tokio1), NoTls)?,
        _ => {
            let connector = build_tls_connector()?;
            cfg.create_pool(Some(Runtime::Tokio1), connector)?
        }
    };

    Ok(pool)
}

/// Build the deadpool-postgres pool configured with the host, port, credentials, and pool
/// size from `pg`.
fn build_pool_config(pg: &PostgresConfig, password: Option<String>) -> DeadpoolConfig {
    let mut cfg = DeadpoolConfig::new();
    cfg.host = Some(pg.host.clone());
    cfg.port = Some(pg.port);
    cfg.dbname = Some(pg.dbname.clone());
    cfg.user = Some(pg.user.clone());
    cfg.password = password;
    cfg.ssl_mode = Some(match pg.ssl_mode {
        PostgresSslMode::Disable => DeadpoolSslMode::Disable,
        PostgresSslMode::Prefer => DeadpoolSslMode::Prefer,
        PostgresSslMode::Require => DeadpoolSslMode::Require,
    });
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });
    cfg.pool = Some(PoolConfig::new(pg.connections));
    cfg
}

/// Resolve the password used to build the pool at startup: the token file when
/// configured, otherwise the static `password` field.
async fn resolve_initial_password(
    pg: &PostgresConfig,
) -> Result<Option<String>, PostgresTokenError> {
    match &pg.token_file {
        Some(path) => {
            let token = wait_for_postgres_token(
                path,
                pg.token_file_wait_timeout,
                pg.token_file_wait_interval,
            )
            .await?;
            Ok(Some(token))
        }
        None => Ok(pg.password.clone()),
    }
}

/// Read the startup token, retrying until the token file becomes readable and
/// valid or `timeout` elapses. A sidecar may mount the file a few seconds after
/// the service starts, so a missing, unreadable, or not-yet-written file is
/// retried rather than treated as fatal until the deadline passes.
async fn wait_for_postgres_token(
    path: &Path,
    timeout: Duration,
    interval: Duration,
) -> Result<String, PostgresTokenError> {
    let deadline = Instant::now() + timeout;
    loop {
        match read_postgres_token(path).await {
            Ok(token) => return Ok(token),
            Err(err) if Instant::now() >= deadline => return Err(err),
            Err(err) => {
                warn!(
                    error = %err,
                    token_file = %path.display(),
                    "postgres token file not ready yet, waiting before retry"
                );
                tokio::time::sleep(interval).await;
            }
        }
    }
}

/// Read the access token (used as the PostgreSQL password) from a token file in
/// the service-identity access-token JSON format.
async fn read_postgres_token(path: &Path) -> Result<String, PostgresTokenError> {
    let contents = tokio::fs::read_to_string(path).await?;
    parse_postgres_token_file(&contents)
}

/// JSON token file holding a short-lived credential and its expiry.
#[derive(Deserialize)]
struct PostgresTokenFile {
    access_token: String,
    expires_on: DateTime<Utc>,
}

/// Parse token-file JSON containing `access_token` and an RFC3339 `expires_on`,
/// returning the trimmed `access_token`. Rejects empty or expired tokens.
fn parse_postgres_token_file(contents: &str) -> Result<String, PostgresTokenError> {
    let parsed: PostgresTokenFile =
        serde_json::from_str(contents).map_err(|e| PostgresTokenError::Parse(e.to_string()))?;

    let access_token = parsed.access_token.trim().to_string();
    if access_token.is_empty() {
        return Err(PostgresTokenError::Empty);
    }
    if parsed.expires_on <= Utc::now() {
        return Err(PostgresTokenError::Expired(parsed.expires_on));
    }

    Ok(access_token)
}

async fn run_token_watch_loop(
    pg: PostgresConfig,
    token_file: std::path::PathBuf,
    poll_interval: Duration,
    pool: Arc<ArcSwap<Pool>>,
) {
    let mut interval = tokio::time::interval(poll_interval);
    let mut last_modified = file_modified_time(&token_file).await;

    loop {
        interval.tick().await;

        let current_modified = file_modified_time(&token_file).await;
        let changed = match (current_modified, last_modified) {
            (Some(current), Some(previous)) => current != previous,
            (Some(_), None) => true,
            _ => false,
        };
        if !changed {
            continue;
        }

        match read_postgres_token(&token_file).await {
            Ok(token) => match create_snapshot_pool_with_password(&pg, &token) {
                Ok(new_pool) => {
                    pool.store(Arc::new(new_pool));
                    last_modified = current_modified;
                    info!(
                        token_file = %token_file.display(),
                        "postgres token rotated, connection pool rebuilt"
                    );
                }
                Err(err) => error!(
                    error = %err,
                    "failed to rebuild postgres pool after token rotation; keeping current pool"
                ),
            },
            Err(err) => error!(
                error = %err,
                token_file = %token_file.display(),
                "failed to reload postgres token file; keeping current pool"
            ),
        }
    }
}

async fn file_modified_time(path: &Path) -> Option<SystemTime> {
    tokio::fs::metadata(path)
        .await
        .and_then(|metadata| metadata.modified())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn token_json(access_token: &str, expires_on: DateTime<Utc>) -> String {
        format!(
            r#"{{"access_token":"{}","expires_on":"{}"}}"#,
            access_token,
            expires_on.to_rfc3339()
        )
    }

    #[test]
    fn parses_valid_token_and_trims() {
        let expires = Utc::now() + ChronoDuration::hours(1);
        let json = token_json("  secret-pw  ", expires);
        assert_eq!(parse_postgres_token_file(&json).unwrap(), "secret-pw");
    }

    #[test]
    fn rejects_expired_token() {
        let expires = Utc::now() - ChronoDuration::minutes(1);
        let json = token_json("secret-pw", expires);
        assert!(matches!(
            parse_postgres_token_file(&json),
            Err(PostgresTokenError::Expired(_))
        ));
    }

    #[test]
    fn rejects_empty_access_token() {
        let expires = Utc::now() + ChronoDuration::hours(1);
        let json = token_json("   ", expires);
        assert!(matches!(
            parse_postgres_token_file(&json),
            Err(PostgresTokenError::Empty)
        ));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(
            parse_postgres_token_file("not json"),
            Err(PostgresTokenError::Parse(_))
        ));
    }

    #[test]
    fn rejects_missing_fields() {
        assert!(matches!(
            parse_postgres_token_file(r#"{"access_token":"secret-pw"}"#),
            Err(PostgresTokenError::Parse(_))
        ));
    }

    fn unique_temp_path() -> std::path::PathBuf {
        let unique = format!(
            "pg-token-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[tokio::test]
    async fn waits_for_token_file_to_appear() {
        let path = unique_temp_path();
        let expires = Utc::now() + ChronoDuration::hours(1);
        let json = token_json("late-pw", expires);

        let writer_path = path.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::fs::write(&writer_path, json).await.unwrap();
        });

        let token =
            wait_for_postgres_token(&path, Duration::from_secs(5), Duration::from_millis(20))
                .await
                .unwrap();

        writer.await.unwrap();
        let _ = tokio::fs::remove_file(&path).await;
        assert_eq!(token, "late-pw");
    }

    #[tokio::test]
    async fn fails_when_token_file_never_appears() {
        let path = unique_temp_path();
        let err =
            wait_for_postgres_token(&path, Duration::from_millis(50), Duration::from_millis(20))
                .await
                .unwrap_err();
        assert!(matches!(err, PostgresTokenError::Io(_)));
    }
}
