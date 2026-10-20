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

use arc_swap::ArcSwap;
use clap::Parser;
use deadpool_postgres::Pool;
use permission_auth::AuthCache;
use permission_db::config_file::{ConfigFileMetadataStorage, ConfigFilePolicyStorage};
use permission_db::postgres::{
    PostgresDatabaseOptions, PostgresMetadataStorage, PostgresPolicyStorage,
};
use permission_db::{ConnectionPool, MetadataStorage, PolicyStorage};
use permission_service::cache::Caches;
use permission_service::config::{DatabaseType, ServiceConfig};
use permission_service::events::consumer::spawn_permission_change_consumer;
use permission_service::grpc::start_grpc_service;
use permission_service::http;
use permission_service::metrics;
use permission_service::metrics::Metrics;
use permission_service::pool::{
    InstrumentedMetadataStorage, InstrumentedPolicyStorage, InstrumentedPool,
};
use permission_service::service::authorization::{set_system_metadata, set_system_policies};
use permission_service::state::AppState;
use permission_service::traces;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::{join, spawn};

mod database;

fn main() {
    let config = ServiceConfig::parse();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.service.worker_threads)
        .thread_stack_size(config.service.thread_stack_size)
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_service(config));
}

async fn run_service(mut config: ServiceConfig) {
    let _tracing = traces::configure_tracing(&config);

    let (meter_provider, meter) = metrics::build_meter(&config);
    let auth_cache = Arc::new(AuthCache::new(
        config.auth.token_cache_size,
        config.auth.token_ttl,
        config.auth.invalid_token_cache_size,
        config.auth.invalid_token_cache_ttl,
    ));
    let caches = Caches::new(&config.cache, auth_cache);
    let metrics = Metrics::new(meter_provider, meter, &caches);

    let client = Arc::new(
        config
            .client_timeouts
            .idp
            .build_client()
            .unwrap_or_else(|err| {
                tracing::error!("Failed to build IdP HTTP client: {}", err);
                std::process::exit(1);
            }),
    );
    if config.auth.enabled {
        if let Err(err) = config.auth.load(&client).await {
            tracing::error!("Failed to initialize authentication: {}", err);
            std::process::exit(1);
        }
    } else {
        tracing::warn!(
            "!!! AUTHENTICATION IS DISABLED WITH `ENABLE_AUTH` ENVIRONMENT VARIABLE EQUAL TO {} !!! \
            Use only for development.",
            config.auth.enabled
        );
    }

    tracing::info!(
        "Running with the following configuration: \n{:#?}\n",
        config
    );

    let (policies, metadata): (Arc<dyn PolicyStorage>, Arc<dyn MetadataStorage>) =
        match config.database.database_type {
            DatabaseType::Postgres => {
                let pool = create_database_pool(&config).await;
                database::spawn_token_refresher(&config, pool.clone());
                let db_pool: Arc<dyn ConnectionPool> =
                    Arc::new(InstrumentedPool::new(pool, &metrics.meter));
                let db_runtime = create_database_runtime(&config);
                let options = PostgresDatabaseOptions { temporary: false };

                let dbname = config.postgres.dbname.as_deref().unwrap();

                let mut policies = PostgresPolicyStorage::new(
                    db_pool.clone(),
                    db_runtime.clone(),
                    dbname,
                    options.clone(),
                );
                policies.initialize().await.unwrap();
                set_system_policies(&mut policies, &config).await;

                let metadata = PostgresMetadataStorage::new(
                    db_pool.clone(),
                    db_runtime.clone(),
                    dbname,
                    options.clone(),
                    config.auth.principal_id_claim.clone(),
                );
                metadata.initialize().await.unwrap();
                set_system_metadata(&metadata, &config).await;

                (Arc::new(policies), Arc::new(metadata))
            }
            DatabaseType::ConfigFile => {
                let policy_path = PathBuf::from(&config.database.init.policies_file);
                let metadata_path = PathBuf::from(&config.database.init.metadata_file);

                let policies = ConfigFilePolicyStorage::new(policy_path);
                policies.initialize().await.unwrap();

                let metadata = ConfigFileMetadataStorage::new(
                    metadata_path,
                    config.auth.principal_id_claim.clone(),
                );
                metadata.initialize().await.unwrap();
                set_system_metadata(&metadata, &config).await;

                (Arc::new(policies), Arc::new(metadata))
            }
        };

    let policies: Arc<dyn PolicyStorage> =
        Arc::new(InstrumentedPolicyStorage::new(policies, &metrics));
    let metadata: Arc<dyn MetadataStorage> =
        Arc::new(InstrumentedMetadataStorage::new(metadata, &metrics));

    let state = AppState::new(caches, config, policies, metadata, metrics, client).await;

    spawn_permission_change_consumer(state.clone());

    let http_state = state.clone();
    let http = spawn(async move {
        start_http_service(http_state).await;
    });

    let grpc = spawn(async move {
        start_grpc_service(state).await.unwrap();
    });

    let (_, _) = join!(http, grpc);
}

/// Resolves the initial Postgres password and builds the connection pool behind
/// an `ArcSwap` so a rotated token can replace it at runtime. The password is
/// either a static secret/file or a short-lived token read from a token file.
#[tracing::instrument(level = "info", skip(config))]
pub(crate) async fn create_database_pool(config: &ServiceConfig) -> Arc<ArcSwap<Pool>> {
    let password = resolve_initial_password(config).await;

    let pool = database::build_postgres_pool(config, &password).unwrap_or_else(|err| {
        tracing::error!("Failed to build Postgres connection pool: {}", err);
        std::process::exit(1);
    });
    Arc::new(ArcSwap::from_pointee(pool))
}

/// Resolves the initial Postgres password. A credential file (token or password
/// file) may be mounted by a sidecar a few seconds after the container starts,
/// so file reads are retried for a bounded time before failing closed. Inline
/// passwords and misconfiguration fail immediately since waiting cannot help.
async fn resolve_initial_password(config: &ServiceConfig) -> String {
    let postgres = &config.postgres;
    let waits_for_file = postgres.token_file.is_some() || postgres.password_file.is_some();
    let deadline = Instant::now() + postgres.credential_wait_timeout;

    loop {
        match read_initial_password(config).await {
            Ok(password) => return password,
            Err(err) if waits_for_file && Instant::now() < deadline => {
                tracing::warn!(
                    "Postgres credential file not ready yet ({}); retrying in {:?}",
                    err,
                    postgres.credential_wait_interval
                );
                tokio::time::sleep(postgres.credential_wait_interval).await;
            }
            Err(err) => {
                tracing::error!("Failed to resolve Postgres password: {}", err);
                std::process::exit(1);
            }
        }
    }
}

async fn read_initial_password(
    config: &ServiceConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(path) = &config.postgres.token_file {
        database::read_postgres_token(path).await
    } else {
        config.postgres.get_password()
    }
}

#[tracing::instrument(level = "info", skip(config))]
pub(crate) fn create_database_runtime(config: &ServiceConfig) -> Arc<tokio::runtime::Runtime> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("postgres")
        .worker_threads(config.postgres.worker_threads)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime for PostgresPolicyStorage");
    Arc::new(runtime)
}

pub(crate) async fn start_http_service(state: AppState) {
    let router = http::configure_router(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    tracing::info!("Listening HTTP on http://0.0.0.0:3000");
    axum::serve(listener, router).await.unwrap();
}

#[cfg(test)]
mod tests;
