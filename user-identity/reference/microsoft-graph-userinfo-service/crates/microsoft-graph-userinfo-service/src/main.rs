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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use clap::Parser;
use tonic::transport::Server as GrpcServer;
use tower::Layer;
use tower_http::cors::CorsLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tracing::info;

use microsoft_graph_userinfo_service::access::AccessLog;
use microsoft_graph_userinfo_service::api::{self, AppState};
use microsoft_graph_userinfo_service::cache::{CacheConfig, CachedProvider};
use microsoft_graph_userinfo_service::config::{AuthMode, Config, PostgresConfig, StartupMode};
use microsoft_graph_userinfo_service::entra::client::{EntraClient, TokenCredential};
use microsoft_graph_userinfo_service::entra::provider::{
    EntraAppRoleAssignments, EntraDeltaSync, EntraProvider,
};
use microsoft_graph_userinfo_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_server::CapabilitiesServiceServer;
use microsoft_graph_userinfo_service::grpc::capabilities::CapabilitiesServiceImpl;
use microsoft_graph_userinfo_service::grpc::access_layer::AccessLogGrpcLayer;
use microsoft_graph_userinfo_service::grpc::proto;
use microsoft_graph_userinfo_service::grpc::proto::user_info_service_server::UserInfoServiceServer;
use microsoft_graph_userinfo_service::grpc::service::UserInfoServiceImpl;
use microsoft_graph_userinfo_service::models::MAX_PAGE_SIZE;
use microsoft_graph_userinfo_service::permission::{PermissionChecker, PermissionConfig};
use microsoft_graph_userinfo_service::postgres::{create_snapshot_pool, spawn_token_refresher};
use microsoft_graph_userinfo_service::provider::{AppRoleAssignmentsSource, IdentityProvider};
use microsoft_graph_userinfo_service::snapshot::SnapshotStore;
use microsoft_graph_userinfo_service::snapshot::postgres::PgSnapshotStore;
use microsoft_graph_userinfo_service::static_directory::{
    StaticDirectory, StaticDirectoryProvider,
};
use microsoft_graph_userinfo_service::telemetry;

#[tokio::main]
async fn main() {
    let config = Config::parse();

    if let Err(e) = config.validate() {
        eprintln!("configuration error: {e}");
        std::process::exit(1);
    }

    let _otel_providers = telemetry::init_telemetry(&config);

    let credential = match config.entra.azure_auth_mode {
        AuthMode::ClientSecret => {
            let secret = config
                .entra
                .azure_client_secret
                .clone()
                .expect("AZURE_CLIENT_SECRET is required when auth mode is client-secret");
            info!("using client-secret authentication");
            TokenCredential::ClientSecret(secret)
        }
        AuthMode::WorkloadIdentity => {
            info!(
                token_file = config.entra.azure_federated_token_file,
                "using workload identity federation"
            );
            TokenCredential::WorkloadIdentity {
                token_file: config.entra.azure_federated_token_file.clone(),
            }
        }
    };

    let client = Arc::new(EntraClient::new(
        config.entra.azure_tenant_id.clone(),
        config.entra.azure_client_id.clone(),
        credential,
        config.entra.graph_max_retries,
    ));

    match config.startup_mode {
        StartupMode::Writer => run_writer(config, client).await,
        StartupMode::Reader => run_reader(config, client).await,
    }
}

/// Writer mode: continuously synchronize the directory into the snapshot store
/// and expose only health endpoints. Requires caching and PostgreSQL.
async fn run_writer(config: Config, client: Arc<EntraClient>) {
    info!("running in writer mode");

    assert!(
        config.cache.enabled,
        "writer mode requires caching to be enabled (CACHE_ENABLED=true)"
    );

    let snapshot_store = create_snapshot_store(
        &config.postgres,
        config.cache.users_enabled,
        config.cache.groups_enabled,
        config.cache.memberships_enabled,
        config.app_role_assignments.enabled,
    )
    .await
    .unwrap_or_else(|| {
        panic!("writer mode requires PostgreSQL to be enabled (POSTGRES_ENABLED=true)")
    });

    let inner = Arc::new(EntraProvider::new(client.clone(), MAX_PAGE_SIZE));
    let delta_source = Arc::new(EntraDeltaSync::new(
        client.clone(),
        config.cache.memberships_enabled,
    ));

    // Enterprise-application app-role assignment scoping. The startup probe
    // fails closed: if the assignments cannot be read (e.g. a 403 when the
    // configured service principal is not the app's own and Application.Read.All
    // is missing), the writer aborts rather than serving the full directory.
    let app_role_assignments: Option<Arc<dyn AppRoleAssignmentsSource>> =
        if config.app_role_assignments.enabled {
            info!("enterprise-application app-role assignment scoping enabled");
            let source = Arc::new(EntraAppRoleAssignments::new(
                client,
                config
                    .app_role_assignments
                    .service_principal_object_id
                    .clone(),
                config.entra.azure_client_id.clone(),
                config.app_role_assignments.include_service_principals,
                MAX_PAGE_SIZE,
            ));
            source
                .probe()
                .await
                .unwrap_or_else(|e| panic!("app-role assignment startup probe failed: {e}"));
            info!("app-role assignment startup probe succeeded");
            Some(source)
        } else {
            None
        };

    let cache_config = CacheConfig {
        sync_interval: config.cache.sync_interval,
        sync_jitter_max: config.cache.sync_jitter_max,
        users_enabled: config.cache.users_enabled,
        groups_enabled: config.cache.groups_enabled,
        memberships_enabled: config.cache.memberships_enabled,
    };
    let cached = Arc::new(CachedProvider::new(
        delta_source,
        inner,
        Some(snapshot_store),
        cache_config,
        app_role_assignments,
    ));
    let ready = cached.ready();
    let sync_handle = cached.clone();
    tokio::spawn(async move { sync_handle.run_sync_loop().await });

    let app = api::health_router(ready);

    let http_listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port))
        .await
        .expect("failed to bind HTTP listener");

    info!("writer health server listening on port {}", config.port);

    axum::serve(http_listener, app.into_make_service())
        .await
        .expect("HTTP server error");
}

/// Reader mode: serve the HTTP and gRPC read APIs. Reads from the snapshot store
/// (periodically reloaded into memory) when caching is enabled, or live from the
/// identity provider when caching is disabled. Never writes to the store.
async fn run_reader(config: Config, client: Arc<EntraClient>) {
    info!("running in reader mode");

    let static_directory = Arc::new(match &config.static_directory_file {
        Some(path) => {
            info!(path = %path.display(), "loading static directory overlay");
            StaticDirectory::load(path)
                .unwrap_or_else(|e| panic!("failed to load static directory file: {e}"))
        }
        None => StaticDirectory::default(),
    });

    let (base_provider, ready): (Arc<dyn IdentityProvider>, Arc<AtomicBool>) = if config
        .cache
        .enabled
    {
        info!("cache enabled, serving from snapshot store with periodic reload");
        let snapshot_store = create_snapshot_store(
            &config.postgres,
            config.cache.users_enabled,
            config.cache.groups_enabled,
            config.cache.memberships_enabled,
            config.app_role_assignments.enabled,
        )
        .await
        .unwrap_or_else(|| {
            panic!(
                "reader mode with caching enabled requires PostgreSQL to be enabled (POSTGRES_ENABLED=true)"
            )
        });

        let inner = Arc::new(EntraProvider::new(client.clone(), MAX_PAGE_SIZE));
        let delta_source = Arc::new(EntraDeltaSync::new(
            client,
            config.cache.memberships_enabled,
        ));
        let cache_config = CacheConfig {
            sync_interval: config.cache.sync_interval,
            sync_jitter_max: config.cache.sync_jitter_max,
            users_enabled: config.cache.users_enabled,
            groups_enabled: config.cache.groups_enabled,
            memberships_enabled: config.cache.memberships_enabled,
        };
        // The reader never fetches assignments; it serves the already-scoped
        // snapshot (including SP-users merged by the snapshot store on load).
        let cached = Arc::new(CachedProvider::new(
            delta_source,
            inner,
            Some(snapshot_store),
            cache_config,
            None,
        ));
        let ready = cached.ready();
        cached.initialize_reader().await;
        info!("reader ready; snapshot reloads from the store in the background");
        let reload_interval = config.cache.reader_reload_interval;
        let reload_handle = cached.clone();
        tokio::spawn(async move { reload_handle.run_reader_loop(reload_interval).await });
        (cached, ready)
    } else {
        info!("cache disabled, using direct provider");
        let provider = Arc::new(EntraProvider::new(client, MAX_PAGE_SIZE));
        let ready = Arc::new(AtomicBool::new(true));
        (provider, ready)
    };

    // Overlay the static directory on top of whichever provider serves the data,
    // so operator-supplied entities appear regardless of caching configuration.
    let provider: Arc<dyn IdentityProvider> = if static_directory.is_empty() {
        base_provider
    } else {
        info!("static directory overlay active");
        Arc::new(StaticDirectoryProvider::new(
            base_provider,
            static_directory,
        ))
    };

    let permission_checker = if config.permission.enabled {
        info!(
            url = %config.permission.url,
            service_name = %config.permission.service_name,
            "permission checking enabled"
        );
        let permission_config = PermissionConfig {
            url: config.permission.url,
            service_name: config.permission.service_name,
            cache_ttl: config.permission.cache_ttl,
            cache_max_capacity: config.permission.cache_max_capacity,
        };
        Some(Arc::new(PermissionChecker::new(permission_config)))
    } else {
        info!("permission checking disabled");
        None
    };

    let access = Arc::new(AccessLog::new(
        config.access_log.enabled,
        config.access_log.headers.0.clone(),
    ));
    let forward_headers = Arc::new(config.forward_headers.clone());

    let state = AppState {
        provider: provider.clone(),
        ready,
        permission_checker: permission_checker.clone(),
        access: access.clone(),
        forward_headers: forward_headers.clone(),
    };

    let app = NormalizePathLayer::trim_trailing_slash()
        .layer(api::router(state.clone()).layer(CorsLayer::permissive()));

    let http_listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port))
        .await
        .expect("failed to bind HTTP listener");

    info!("HTTP server listening on port {}", config.port);

    let grpc_addr = format!("0.0.0.0:{}", config.grpc_port)
        .parse()
        .expect("invalid gRPC address");

    let (health_reporter, health_service) = tonic_health::server::health_reporter();

    health_reporter
        .set_serving::<CapabilitiesServiceServer<CapabilitiesServiceImpl>>()
        .await;

    let ready_flag = state.ready.clone();
    tokio::spawn(async move {
        let svc_name = "nvidia.omniverse.userinfo.v1alpha.UserInfoService";
        loop {
            if ready_flag.load(Ordering::Acquire) {
                health_reporter
                    .set_service_status(svc_name, tonic_health::ServingStatus::Serving)
                    .await;
                break;
            }
            health_reporter
                .set_service_status(svc_name, tonic_health::ServingStatus::NotServing)
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let reflection_service_v1 = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(
            microsoft_graph_userinfo_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::FILE_DESCRIPTOR_SET,
        )
        .build_v1()
        .expect("failed to build v1 reflection service");

    // Older reflection clients (e.g. the Discovery Service, Postman, Insomnia)
    // only speak the v1alpha reflection protocol; without this they receive
    // UNIMPLEMENTED and cannot discover the CapabilitiesService.
    let reflection_service_v1alpha = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(
            microsoft_graph_userinfo_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::FILE_DESCRIPTOR_SET,
        )
        .build_v1alpha()
        .expect("failed to build v1alpha reflection service");

    let user_info_service = UserInfoServiceImpl::new(
        provider,
        permission_checker,
        access.clone(),
        forward_headers,
    );

    let grpc_server = GrpcServer::builder()
        .layer(AccessLogGrpcLayer::new(access.clone()))
        .add_service(health_service)
        .add_service(reflection_service_v1)
        .add_service(reflection_service_v1alpha)
        .add_service(UserInfoServiceServer::new(user_info_service))
        .add_service(CapabilitiesServiceServer::new(CapabilitiesServiceImpl))
        .serve(grpc_addr);

    info!("gRPC server listening on port {}", config.grpc_port);

    tokio::select! {
        result = axum::serve(http_listener, axum::ServiceExt::<axum::extract::Request>::into_make_service(app)) => {
            result.expect("HTTP server error");
        }
        result = grpc_server => {
            result.expect("gRPC server error");
        }
    }
}

async fn create_snapshot_store(
    pg: &PostgresConfig,
    users_enabled: bool,
    groups_enabled: bool,
    memberships_enabled: bool,
    app_role_assignments_enabled: bool,
) -> Option<Arc<dyn SnapshotStore>> {
    if !pg.enabled {
        info!("postgres snapshot store disabled");
        return None;
    }

    info!(
        host = %pg.host,
        port = pg.port,
        dbname = %pg.dbname,
        ssl_mode = ?pg.ssl_mode,
        "connecting to snapshot store"
    );

    let pool = create_snapshot_pool(pg)
        .await
        .expect("failed to create postgres pool");
    let pool = Arc::new(ArcSwap::from_pointee(pool));
    spawn_token_refresher(pg, pool.clone());

    let store = PgSnapshotStore::new(
        pool,
        users_enabled,
        groups_enabled,
        memberships_enabled,
        app_role_assignments_enabled,
    );
    store
        .initialize()
        .await
        .expect("failed to initialize snapshot store");

    Some(Arc::new(store))
}
