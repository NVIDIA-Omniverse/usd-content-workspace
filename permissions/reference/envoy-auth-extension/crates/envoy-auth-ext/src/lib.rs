// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

// mod controllers;
mod handlers;
pub mod settings;

use opentelemetry::metrics::Meter;
use permission_auth::AuthCache;
use settings::{AppSettings, IdpClientSettings};
use std::error::Error;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::oneshot;
use tonic::transport::server::Router;
use tonic::transport::Server;
use tonic_observability::rpc_map::{RpcMap, RpcMapOptimize};
use tonic_observability::server::metric_a66::TonicMetricLayer;
use tonic_observability::server::trace::TonicTraceLayer;
use tower::layer::util::Stack;

const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("envoy_auth_descriptor");

#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<AuthCache>,
    pub client: Arc<reqwest::Client>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("client", &self.client)
            .finish()
    }
}

pub fn build_idp_client(settings: &IdpClientSettings) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(settings.connect_timeout)
        .timeout(settings.request_timeout)
        .pool_idle_timeout(settings.pool_idle_timeout)
        .build()
}
type GrpcServer = Router<
    Stack<
        Stack<TonicMetricLayer, Stack<TonicTraceLayer, tower::layer::util::Identity>>,
        tower::layer::util::Identity,
    >,
>;

/// Publicly-exposed struct for instantiating a new scheduler gRPC server. Provides options for:
/// - Creating the server instance.
/// - Starting the server instance immediately within a pre-existing tokio runtime (useful when creating a binary crate
///   from which the service can be spun up).
/// - Returning a future that, when executed, will run the server instance (provides more granular control over how the
///   server instance actually begins to run, which can be useful when writing integration tests).
#[derive(Debug)]
pub struct AuthServer {
    bound_address: std::net::SocketAddr,
    listener: TcpListener,
    grpc_server: GrpcServer,
}

impl AuthServer {
    #[tracing::instrument(level = "debug")]
    pub async fn new(
        app_settings: AppSettings,
        meter: Meter,
        state: AppState,
    ) -> Result<AuthServer, Box<dyn Error + Send + Sync>> {
        let health_service = handlers::grpc::init_auth_service().await;

        let tonic_settings = &app_settings.server.tonic;

        // NOTE: Must use build_v1alpha() and not build_v1() in order to support many clients using reflection
        let reflection_serv = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
            .build_v1alpha()?;

        let grpc_serv = handlers::grpc::init_grpc_auth_service(&app_settings, &meter, state);

        // Create and bind a TCP listener first - this lets us query the bound port if 0 was passed in.
        let address: std::net::SocketAddr = format!(
            "{}:{}",
            app_settings.server.ip_address, app_settings.server.port
        )
        .parse()?;
        let listener = TcpListener::bind(address).await?;
        let bound_address = listener.local_addr()?;

        tracing::info!("tonic settings are: {:?}", tonic_settings);
        tracing::info!("app settings are: {:?}", app_settings);

        let rpc_map = RpcMap::builder()
            .optimize(RpcMapOptimize::STATIC)
            .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)?
            .add_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)?
            .build();
        let merged_layers = tower::ServiceBuilder::new()
            .layer(TonicTraceLayer::builder().build())
            .layer(
                TonicMetricLayer::builder(meter)
                    .with_rpc_map(rpc_map)
                    .build(),
            )
            .into_inner();

        let grpc_server = Server::builder()
            .initial_connection_window_size(tonic_settings.initial_connection_window_size)
            .initial_stream_window_size(tonic_settings.initial_stream_window_size)
            .layer(merged_layers)
            .add_service(reflection_serv)
            .add_service(health_service)
            .add_service(grpc_serv);

        Ok(Self {
            bound_address,
            listener,
            grpc_server,
        })
    }

    #[tracing::instrument(level = "debug")]
    pub async fn start(self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Self {
            bound_address,
            listener,
            grpc_server,
        } = self;

        let fut = grpc_server
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));

        tracing::info!("gRPC server listening on {}", bound_address);

        // Spawns them both and keeps them alive because neither returns.
        tokio::select! {
            r = fut => { r? }
            _ = signal::ctrl_c() => {
                tracing::info!("closing the auth extension server.");
            }
        }

        Ok(())
    }

    #[tracing::instrument(level = "debug")]
    pub async fn server_future(
        self,
    ) -> (
        impl std::future::Future<Output = Result<(), tonic::transport::Error>>,
        oneshot::Sender<()>,
    ) {
        let Self {
            bound_address,
            listener,
            grpc_server,
        } = self;

        let (tx, rx) = oneshot::channel::<()>();
        let fut = grpc_server.serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            async move {
                let _ = rx.await;
            },
        );

        tracing::info!("gRPC server listening on {}", bound_address);

        (fut, tx)
    }

    pub fn get_port(&self) -> u16 {
        self.bound_address.port()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn idp_client_times_out_stalled_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut stalled_connections = Vec::new();
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                stalled_connections.push(socket);
            }
        });
        let client = build_idp_client(&settings::IdpClientSettings {
            request_timeout: Duration::from_millis(200),
            ..Default::default()
        })
        .unwrap();

        let result = timeout(
            Duration::from_secs(5),
            client.get(format!("http://{address}/keys")).send(),
        )
        .await
        .expect("IdP request was not bounded by the client timeout");

        assert!(result.unwrap_err().is_timeout());
    }

    #[test]
    fn idp_client_settings_default_timeouts() {
        let settings = settings::IdpClientSettings::default();

        assert_eq!(settings.connect_timeout, Duration::from_secs(5));
        assert_eq!(settings.request_timeout, Duration::from_secs(15));
        assert_eq!(settings.pool_idle_timeout, Duration::from_secs(90));
    }

    fn create_test_settings() -> AppSettings {
        AppSettings {
            server: settings::ServerSettings {
                ip_address: "127.0.0.1".to_string(),
                port: 0, // Let OS assign port
                tonic: settings::TonicSettings {
                    initial_connection_window_size: 1024,
                    initial_stream_window_size: 1024,
                    max_decoding_message_size: 4 * 1024 * 1024, // 4MB
                },
                username_claims: vec!["preferred_username".to_string()],
                user_id_header: "user-id".to_string(),
                username_header: "username".to_string(),
                username_source_claim_header: "username-source-claim".to_string(),
            },
            logging: settings::LoggingSettings::default(),
            otel: settings::OtelSettings::default(),
            auth: permission_auth::config::AuthConfig::default(),
            idp_client: settings::IdpClientSettings::default(),
        }
    }

    fn create_test_state() -> AppState {
        AppState {
            cache: Arc::new(AuthCache::new(
                1000,
                Duration::from_secs(1),
                100,
                Duration::from_secs(1),
            )), // Use reasonable cache defaults for tests
            client: Arc::new(reqwest::Client::new()),
        }
    }

    fn create_test_meter() -> Meter {
        opentelemetry::global::meter("test")
    }

    #[tokio::test]
    async fn test_server_creation() {
        let settings = create_test_settings();
        let meter = create_test_meter();
        let state = create_test_state();

        let server = AuthServer::new(settings, meter, state).await;
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_port_binding() {
        let settings = create_test_settings();
        let meter = create_test_meter();
        let state = create_test_state();

        let server = AuthServer::new(settings, meter, state).await.unwrap();
        assert!(server.get_port() > 0);
    }

    #[tokio::test]
    async fn test_health_rpc() {
        let settings = create_test_settings();
        let meter = create_test_meter();
        let state = create_test_state();

        let server = AuthServer::new(settings, meter, state).await.unwrap();
        let port = server.get_port();
        let (server_fut, shutdown_tx) = server.server_future().await;
        let server_handle = tokio::spawn(server_fut);

        let endpoint =
            tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{port}")).unwrap();
        let channel = endpoint.connect().await.unwrap();
        let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
        let response = timeout(
            Duration::from_secs(5),
            client.check(tonic_health::pb::HealthCheckRequest {
                service: "envoy.service.auth.v3.Authorization".to_string(),
            }),
        )
        .await
        .expect("health RPC timed out")
        .unwrap();
        assert_eq!(
            response.into_inner().status,
            tonic_health::pb::health_check_response::ServingStatus::Serving as i32
        );

        shutdown_tx.send(()).expect("failed to stop server");
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_server_shutdown() {
        let settings = create_test_settings();
        let meter = create_test_meter();
        let state = create_test_state();

        let server = AuthServer::new(settings, meter, state).await.unwrap();
        let (server_fut, shutdown_tx) = server.server_future().await;

        // Start the server in a separate task
        let server_handle = tokio::spawn(server_fut);

        // Send shutdown signal
        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");

        // Wait for server to shut down with timeout
        let shutdown_result = timeout(Duration::from_secs(5), server_handle).await;
        assert!(shutdown_result.is_ok());
    }

    #[test]
    fn test_app_state_debug() {
        let state = create_test_state();
        let debug_output = format!("{:?}", state);
        assert!(debug_output.contains("AppState"));
        assert!(debug_output.contains("client"));
    }

    #[test]
    fn test_app_state_clone() {
        let state = create_test_state();
        let _cloned_state = state.clone();

        // Verify that the clone works by checking Arc strong count
        assert_eq!(Arc::strong_count(&state.cache), 2);
        assert_eq!(Arc::strong_count(&state.client), 2);
    }
}
