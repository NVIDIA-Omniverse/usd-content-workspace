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

use crate::cache::{Caches, EvaluationCache, MetadataCache};
use permission_core::PolicySetCache;
use crate::config::ServiceConfig;
use crate::events::consumer::EventConsumerArgs;
use crate::events::publisher::EventPublishingArgs;
use crate::metrics::Metrics;
use crate::url_remapper::{load_url_remapper_config, UrlRemapper};
use cedar_policy::Authorizer;
use permission_auth::config::load_openid_configuration;
use permission_auth::interceptors::access_token_file::{
    AccessTokenFileInterceptor, AccessTokenFileProvider,
};
use permission_auth::interceptors::client_credentials::{
    ClientCredentialsInterceptor, Token, TokenManager,
};
use permission_auth::interceptors::TokenRefresher;
use permission_auth::AuthCache;
use permission_db::{MetadataStorage, PolicyStorage};
use permission_service_notification_proto::nvidia::omniverse::notifications::publisher::v1beta::event_publishing_service_client::EventPublishingServiceClient;
use permission_service_notification_proto::nvidia::omniverse::notifications::publisher::v1beta::{PublishEventRequest, PublishEventResponse};
use reqwest::Client as HttpClient;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tonic::codegen::InterceptedService;
use tonic::transport::Channel;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

#[tonic::async_trait]
trait EventPublisher: Send + Sync {
    async fn publish_event(
        &self,
        request: PublishEventRequest,
    ) -> Result<tonic::Response<PublishEventResponse>, tonic::Status>;
}

macro_rules! impl_event_publisher {
    ($t:ty) => {
        #[tonic::async_trait]
        impl EventPublisher for EventPublishingServiceClient<$t> {
            async fn publish_event(
                &self,
                request: PublishEventRequest,
            ) -> Result<tonic::Response<PublishEventResponse>, tonic::Status> {
                let mut client = self.clone();
                (&mut client).publish_event(request).await
            }
        }
    };
}

impl_event_publisher!(Channel);
impl_event_publisher!(InterceptedService<Channel, ClientCredentialsInterceptor>);
impl_event_publisher!(InterceptedService<Channel, AccessTokenFileInterceptor>);

#[derive(Clone)]
pub struct EventClient {
    publisher: Arc<dyn EventPublisher>,
    refresher: Option<Arc<dyn TokenRefresher>>,
}

impl EventClient {
    pub async fn publish_event(
        &self,
        request: PublishEventRequest,
    ) -> Result<tonic::Response<PublishEventResponse>, tonic::Status> {
        self.publisher.publish_event(request).await
    }

    pub fn refresher(&self) -> Option<&dyn TokenRefresher> {
        self.refresher.as_deref()
    }
}

/// Outbound service-identity auth shared by the event publisher and the event
/// consumer. Built once so that a single background refresh/watch loop keeps the
/// bearer token fresh for every outbound gRPC client, and so the consumer can
/// attach a currently-valid token each time it re-opens its long-lived stream.
#[derive(Clone)]
pub struct ServiceIdentity {
    auth: ServiceIdentityAuth,
    refresher: Option<Arc<dyn TokenRefresher>>,
}

#[derive(Clone)]
enum ServiceIdentityAuth {
    AccessTokenFile(Arc<AccessTokenFileProvider>),
    ClientCredentials(Arc<Mutex<Option<Token>>>),
    Anonymous,
}

impl ServiceIdentity {
    /// Bearer token currently usable for an outbound request, if any.
    pub(crate) fn bearer_token(&self) -> Option<String> {
        match &self.auth {
            ServiceIdentityAuth::AccessTokenFile(provider) => provider.get_token(),
            ServiceIdentityAuth::ClientCredentials(handle) => {
                handle.lock().ok().and_then(|guard| {
                    guard
                        .as_ref()
                        .filter(|token| !token.is_expired())
                        .map(|token| token.access_token.clone())
                })
            }
            ServiceIdentityAuth::Anonymous => None,
        }
    }

    /// Whether the current token is missing or close enough to expiry that a
    /// long-lived stream opened with it should be re-established with a fresh one.
    pub(crate) fn is_expiring_soon(&self) -> bool {
        match &self.auth {
            ServiceIdentityAuth::AccessTokenFile(provider) => provider.get_token().is_none(),
            ServiceIdentityAuth::ClientCredentials(handle) => match handle.lock() {
                Ok(guard) => guard.as_ref().is_none_or(|token| token.is_expiring_soon()),
                Err(_) => true,
            },
            ServiceIdentityAuth::Anonymous => false,
        }
    }

    /// Whether this identity attaches a bearer token to outbound requests.
    pub(crate) fn is_authenticated(&self) -> bool {
        !matches!(self.auth, ServiceIdentityAuth::Anonymous)
    }

    /// Forces the shared token source to refresh, e.g. after a request was
    /// rejected with `Unauthenticated` or before re-opening an expiring stream.
    pub(crate) async fn refresh(&self) {
        if let Some(refresher) = &self.refresher {
            refresher.refresh().await;
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub auth_cache: Arc<AuthCache>,
    pub authorizer: Arc<Authorizer>,
    pub config: Arc<ServiceConfig>,
    pub client: Arc<HttpClient>,
    pub policies: Arc<dyn PolicyStorage>,
    pub metadata: Arc<dyn MetadataStorage>,
    pub metadata_cache: Arc<MetadataCache>,
    pub metrics: Arc<Metrics>,
    pub evaluation_cache: Arc<EvaluationCache>,
    pub policy_set_cache: Arc<PolicySetCache>,
    pub event_client: Option<EventClient>,
    /// Shared outbound service identity, present when publishing or consuming is
    /// enabled. Reused by the event consumer to authenticate its subscription.
    pub service_identity: Option<ServiceIdentity>,
    /// Forward URL-prefix remapper applied to resource IDs. `None` when the
    /// `enable-url-remapper` feature flag is disabled.
    pub url_remapper: Option<Arc<UrlRemapper>>,
}

impl AppState {
    pub async fn new(
        caches: Caches,
        config: ServiceConfig,
        policies: Arc<dyn PolicyStorage>,
        metadata: Arc<dyn MetadataStorage>,
        metrics: Metrics,
        client: Arc<reqwest::Client>,
    ) -> AppState {
        let service_identity = if config.publishing.enabled || config.consumer.enabled {
            Some(build_service_identity(&config, &client).await)
        } else {
            None
        };

        let event_client = if config.publishing.enabled {
            let endpoint = build_event_publishing_endpoint(&config.publishing)
                .expect("Failed to build endpoint for Event Aggregation Service");
            match endpoint.connect().await {
                Ok(channel) => {
                    let identity = service_identity
                        .as_ref()
                        .expect("service identity is built when publishing is enabled");
                    tracing::info!(
                        "Connected to Event Aggregation Service at {}",
                        config.publishing.event_publishing_grpc_endpoint
                    );
                    Some(build_event_client(channel, identity))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to connect to Event Aggregation Service at {}: {}",
                        config.publishing.event_publishing_grpc_endpoint,
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        let url_remapper = if config.url_remapper.enabled {
            let remapper_config = load_url_remapper_config(&config.url_remapper.file);
            UrlRemapper::from_config(remapper_config)
                .unwrap_or_else(|e| {
                    tracing::error!("Invalid URL remapper configuration: {}", e);
                    std::process::exit(1);
                })
                .map(Arc::new)
        } else {
            None
        };

        AppState {
            auth_cache: caches.auth,
            authorizer: Arc::new(Authorizer::new()),
            client,
            config: Arc::new(config.clone()),
            policies,
            metadata,
            metadata_cache: caches.metadata,
            evaluation_cache: caches.evaluation,
            policy_set_cache: caches.policy_set,
            metrics: Arc::new(metrics),
            event_client,
            service_identity,
            url_remapper,
        }
    }
}

/// Builds the shared outbound service identity and spawns the single background
/// loop that keeps its token fresh. Called once when publishing or consuming is
/// enabled so both the publisher and the consumer reuse the same token source.
async fn build_service_identity(
    config: &ServiceConfig,
    client: &Arc<HttpClient>,
) -> ServiceIdentity {
    if config.access_token_file.enabled {
        let provider = Arc::new(AccessTokenFileProvider::new(
            PathBuf::from(&config.access_token_file.file_path),
            config.access_token_file.poll_interval,
        ));
        provider.load().await;

        let watcher = provider.clone();
        tokio::spawn(async move {
            watcher.start_watch_loop().await;
        });

        ServiceIdentity {
            auth: ServiceIdentityAuth::AccessTokenFile(provider.clone()),
            refresher: Some(provider),
        }
    } else if config.client_credentials.enabled {
        let openid_response = load_openid_configuration(
            &config.client_credentials.openid_configuration,
            client,
        )
        .await
        .expect(
            "Failed to load client credentials OpenID configuration for client credentials flow.",
        );

        let token_manager = Arc::new(TokenManager::new(
            client.clone(),
            config.client_credentials.client_id.clone(),
            config
                .client_credentials
                .get_client_secret()
                .expect("Failed to read client credentials client secret"),
            config.client_credentials.client_scope.clone(),
            openid_response.token_endpoint,
            config
                .client_credentials
                .get_extra_headers()
                .expect("Failed to parse client credentials extra headers"),
            config
                .client_credentials
                .get_extra_body_params()
                .expect("Failed to parse client credentials extra body params"),
        ));
        let token_handle = token_manager.get_token_handle();

        let tm = token_manager.clone();
        tokio::spawn(async move {
            tm.start_refresh_loop().await;
        });

        ServiceIdentity {
            auth: ServiceIdentityAuth::ClientCredentials(token_handle),
            refresher: Some(token_manager),
        }
    } else {
        ServiceIdentity {
            auth: ServiceIdentityAuth::Anonymous,
            refresher: None,
        }
    }
}

/// Wraps a connected channel in an `EventPublishingServiceClient` that attaches
/// the shared service-identity token via the matching interceptor.
fn build_event_client(channel: Channel, identity: &ServiceIdentity) -> EventClient {
    match &identity.auth {
        ServiceIdentityAuth::AccessTokenFile(provider) => EventClient {
            publisher: Arc::new(EventPublishingServiceClient::with_interceptor(
                channel,
                AccessTokenFileInterceptor::new(provider.clone()),
            )),
            refresher: identity.refresher.clone(),
        },
        ServiceIdentityAuth::ClientCredentials(handle) => EventClient {
            publisher: Arc::new(EventPublishingServiceClient::with_interceptor(
                channel,
                ClientCredentialsInterceptor::new(handle.clone()),
            )),
            refresher: identity.refresher.clone(),
        },
        ServiceIdentityAuth::Anonymous => EventClient {
            publisher: Arc::new(EventPublishingServiceClient::new(channel)),
            refresher: None,
        },
    }
}

fn build_event_publishing_endpoint(
    publishing: &EventPublishingArgs,
) -> Result<Endpoint, Box<dyn std::error::Error + Send + Sync>> {
    build_grpc_endpoint(
        &publishing.event_publishing_grpc_endpoint,
        publishing.tls_enabled,
        &publishing.tls_ca_certificate_file,
        &publishing.tls_server_name,
    )
}

/// Builds the gRPC endpoint for the Notification Service Consumer API. Shared
/// with [`crate::events::consumer`], which re-connects it on every subscription
/// retry.
pub(crate) fn build_event_consumer_endpoint(
    consumer: &EventConsumerArgs,
) -> Result<Endpoint, Box<dyn std::error::Error + Send + Sync>> {
    build_grpc_endpoint(
        &consumer.event_consumer_grpc_endpoint,
        consumer.tls_enabled,
        &consumer.tls_ca_certificate_file,
        &consumer.tls_server_name,
    )
}

fn build_grpc_endpoint(
    grpc_endpoint: &str,
    tls_enabled: bool,
    tls_ca_certificate_file: &str,
    tls_server_name: &str,
) -> Result<Endpoint, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = Endpoint::new(grpc_endpoint.to_string())?;

    if !tls_enabled {
        return Ok(endpoint);
    }

    let mut tls = ClientTlsConfig::new();
    if tls_ca_certificate_file.is_empty() {
        tls = tls.with_native_roots();
    } else {
        let pem = std::fs::read(tls_ca_certificate_file).map_err(|err| {
            format!(
                "Failed to read Notification Service CA certificate at {}: {}",
                tls_ca_certificate_file, err
            )
        })?;
        tls = tls.ca_certificate(Certificate::from_pem(pem));
    }
    if !tls_server_name.is_empty() {
        tls = tls.domain_name(tls_server_name.to_string());
    }

    Ok(endpoint.tls_config(tls)?)
}

/// In-memory mock for the event publisher that records every published request.
/// Used by tests to verify that policy mutations emit the expected events.
#[doc(hidden)]
pub struct MockEventPublisher {
    requests: std::sync::Mutex<Vec<PublishEventRequest>>,
    notify: tokio::sync::Notify,
}

impl Default for MockEventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEventPublisher {
    pub fn new() -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        }
    }

    pub fn published_requests(&self) -> Vec<PublishEventRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.requests.lock().unwrap().clear();
    }

    /// Waits until at least `n` events have been recorded, with a timeout to avoid hanging.
    pub async fn wait_for_events(&self, n: usize, timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.requests.lock().unwrap().len() >= n {
                return;
            }
            if tokio::time::timeout_at(deadline, self.notify.notified())
                .await
                .is_err()
            {
                panic!(
                    "timed out waiting for {} events (got {})",
                    n,
                    self.requests.lock().unwrap().len()
                );
            }
        }
    }
}

#[tonic::async_trait]
impl EventPublisher for MockEventPublisher {
    async fn publish_event(
        &self,
        request: PublishEventRequest,
    ) -> Result<tonic::Response<PublishEventResponse>, tonic::Status> {
        self.requests.lock().unwrap().push(request.clone());
        self.notify.notify_waiters();
        Ok(tonic::Response::new(PublishEventResponse {
            result: Some(
                permission_service_notification_proto::nvidia::omniverse::notifications::publisher::v1beta::PublishingResult {
                    event: request.event,
                    success: true,
                    failure_reason: String::new(),
                },
            ),
        }))
    }
}

impl EventClient {
    #[doc(hidden)]
    pub fn mock() -> (Self, Arc<MockEventPublisher>) {
        let mock = Arc::new(MockEventPublisher::new());
        let client = EventClient {
            publisher: mock.clone(),
            refresher: None,
        };
        (client, mock)
    }
}
