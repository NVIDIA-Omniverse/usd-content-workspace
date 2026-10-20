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

//! Postgres connection pool construction and short-lived token rotation.
//!
//! The Postgres password may be a short-lived token read from a periodically
//! remounted file (JSON, same format as the outbound service-identity
//! access-token file). Because `deadpool-postgres` bakes the password into the
//! pool manager at creation, a rotated token requires building a fresh pool and
//! swapping it in atomically; existing connections stay valid since Postgres
//! validates credentials only at connection time.

use arc_swap::ArcSwap;
use deadpool_postgres::{
    Config, ManagerConfig, Pool, PoolConfig, RecyclingMethod, Runtime, SslMode,
};
use permission_auth::service_identity::parse_access_token_file;
use permission_service::config::ServiceConfig;
use rustls::{ClientConfig, RootCertStore};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::spawn;
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;

/// Spawns a background task that watches the configured Postgres token file and
/// rebuilds the pool when the token rotates. Does nothing when no token file is
/// configured (static password modes).
pub(crate) fn spawn_token_refresher(config: &ServiceConfig, pool: Arc<ArcSwap<Pool>>) {
    let token_file = match config.postgres.token_file.clone() {
        Some(path) => path,
        None => return,
    };
    let poll_interval = config.postgres.token_file_poll_interval;
    let config = config.clone();
    spawn(async move {
        run_token_watch_loop(config, token_file, poll_interval, pool).await;
    });
}

async fn run_token_watch_loop(
    config: ServiceConfig,
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
            Ok(token) => match build_postgres_pool(&config, &token) {
                Ok(new_pool) => {
                    pool.store(Arc::new(new_pool));
                    last_modified = current_modified;
                    tracing::info!(
                        "Postgres token file {:?} changed on disk, connection pool rebuilt",
                        token_file
                    );
                }
                Err(err) => {
                    tracing::error!(
                        "Failed to rebuild Postgres pool after token rotation: {}; keeping current pool",
                        err
                    );
                }
            },
            Err(err) => {
                tracing::error!(
                    "Failed to reload Postgres token file {:?}: {}; keeping current pool",
                    token_file,
                    err
                );
            }
        }
    }
}

async fn file_modified_time(path: &Path) -> Option<SystemTime> {
    tokio::fs::metadata(path)
        .await
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Reads the access token (used as the Postgres password) from a token file in
/// the service-identity access-token JSON format.
pub(crate) async fn read_postgres_token(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let contents = tokio::fs::read_to_string(path).await?;
    Ok(parse_access_token_file(&contents)?.access_token)
}

/// Builds a deadpool `Pool` for the given password. The password may be a static
/// secret or a short-lived token; callers rebuild with a fresh password to
/// rotate credentials. Returns an error instead of panicking so callers can fail
/// closed at startup or keep the current pool on a failed rotation.
pub(crate) fn build_postgres_pool(
    config: &ServiceConfig,
    password: &str,
) -> Result<Pool, Box<dyn std::error::Error>> {
    let mut pg_config = Config::new();
    pg_config.dbname = config.postgres.dbname.clone();
    pg_config.host = config.postgres.host.clone();
    pg_config.port = Some(config.postgres.port);
    pg_config.user = config.postgres.user.clone();
    pg_config.password = Some(password.to_string());
    pg_config.options = Some("-c timezone=UTC".to_string());
    pg_config.ssl_mode = config.postgres.ssl_mode;
    pg_config.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    pg_config.pool = Some(PoolConfig::new(config.postgres.connections));

    let ssl_mode = config.postgres.ssl_mode.unwrap_or(SslMode::Disable);
    let pool = match ssl_mode {
        SslMode::Disable => pg_config.create_pool(Some(Runtime::Tokio1), NoTls)?,
        _ => {
            let connector = build_postgres_tls_connector()?;
            pg_config.create_pool(Some(Runtime::Tokio1), connector)?
        }
    };
    Ok(pool)
}

/// Builds a rustls-backed TLS connector for the Postgres pool, seeded with
/// the platform's native certificate store. Returns an error if no trust
/// anchors can be loaded so callers can fail closed instead of silently
/// downgrading to plaintext when the operator asked for TLS.
fn build_postgres_tls_connector() -> Result<MakeRustlsConnect, Box<dyn std::error::Error>> {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for err in &native.errors {
        tracing::warn!("Skipping unreadable native CA certificate: {}", err);
    }
    if native.certs.is_empty() {
        return Err("no native CA certificates available for Postgres TLS".into());
    }
    for cert in native.certs {
        roots.add(cert)?;
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls_config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();

    Ok(MakeRustlsConnect::new(tls_config))
}
