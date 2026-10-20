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

//! Shared outbound-gRPC plumbing for `thumbnail-service`.
//!
//! All service-account outbound calls (storage metadata, storage file-object,
//! event publishing, notification stream consumption) flow through the
//! [`OutboundClients`] struct constructed once at startup. The struct holds
//! pre-connected [`Channel`]s plus the [`AuthInterceptor`] selected by the
//! `--enable-client-credentials` / `--enable-access-token-file` runtime mode,
//! and an `Arc<dyn TokenRefresher>` so individual call sites can implement
//! the 401-retry contract.
//!
//! There is no implicit fallback for any of these clients: a binary that
//! needs outbound auth must construct `OutboundClients` at startup with a
//! valid configuration, and pass the `Arc` into the modules that need it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{TokenGate, TOKEN_READY_TIMEOUT};
use crate::RuntimeConfigArgs;
use clap::Args;
use thumbnail_auth::{
    load_openid_configuration, AccessTokenFileArgs, AccessTokenFileInterceptor,
    AccessTokenFileProvider, AuthInterceptor, ClientCredentialsArgs, ClientCredentialsInterceptor,
    TokenManager, TokenRefresher,
};
use tonic::transport::Channel;
use tracing::{error, info};

/// Combined CLI args for outbound-auth configuration. Embed into each binary's
/// top-level `Args` struct with `#[command(flatten)]`. clap enforces the
/// mutual exclusivity of the two modes; see [`ClientCredentialsArgs`] /
/// [`AccessTokenFileArgs`] for the per-flag invariants.
#[derive(Args, Clone, Debug)]
pub struct OutboundAuthArgs {
    #[command(flatten)]
    pub client_credentials: ClientCredentialsArgs,

    #[command(flatten)]
    pub access_token_file: AccessTokenFileArgs,
}

impl OutboundAuthArgs {
    /// Returns `true` when exactly one mode is enabled. clap also enforces
    /// this via `conflicts_with` + `requires_ifs`, so callers that rely on
    /// `clap::Parser::parse()` get the same guarantee for free.
    pub fn one_mode_enabled(&self) -> bool {
        self.client_credentials.enabled ^ self.access_token_file.enabled
    }
}

/// A channel that injects the active span's W3C trace context into every request.
///
/// For call sites that authenticate per request — storage does, via
/// `add_auth_headers` — so no interceptor sits in the stack.
pub type TracedChannel = tonic_observability::client::trace::TonicTraceService<Channel>;

/// A traced channel with the auth interceptor beneath it.
///
/// The trace layer must sit *outside* the interceptor, not around a bare channel:
/// tonic's generated `with_interceptor` requires the inner service's response
/// body to implement `Default`, which the trace layer's body does not, so the
/// composition only type-checks in this order. It is also the right order —
/// the request is traced before auth decorates it, and both headers reach the
/// wire.
pub type TracedAuthedChannel = tonic_observability::client::trace::TonicTraceService<
    tonic::service::interceptor::InterceptedService<Channel, AuthInterceptor>,
>;

/// Apply client-side trace propagation to a service.
///
/// Applied per call site rather than inside `build_channel` because the
/// event-consumer channel must not have it. Its RPCs are long-lived
/// bidirectional streams and the layer opens one span per RPC, while
/// `tracing_subscriber::Registry` keeps a span alive as long as any descendant is
/// open. A stream that stays up for the life of the process would therefore pin
/// its whole ancestor chain, and none of those spans would ever export — losing
/// the traces they appear in, not merely their own. That is strictly worse than
/// not propagating.
///
/// The consumer path gives up nothing real: a durable-stream RPC has no caller
/// trace to continue, and the work it delivers is traced from the span each
/// delivery opens.
pub(crate) fn trace_layer<S>(
    service: S,
) -> tonic_observability::client::trace::TonicTraceService<S> {
    use tower_layer::Layer as _;
    tonic_observability::client::trace::TonicTraceLayer::builder()
        .build()
        .layer(service)
}

/// Compose a channel with the auth interceptor and trace propagation.
///
/// For call sites holding their own channel rather than one from
/// [`OutboundClients`] — the worker's `AssociateThumbnail` client is the one.
pub fn trace_authed(channel: Channel, interceptor: AuthInterceptor) -> TracedAuthedChannel {
    trace_layer(tonic::service::interceptor::InterceptedService::new(
        channel,
        interceptor,
    ))
}

/// Shared pre-connected gRPC channels and the auth interceptor that
/// every outbound call site clones onto its per-call client.
#[derive(Clone)]
pub struct OutboundClients {
    /// Channel to the storage gRPC service (used for FileObject + Metadata).
    pub storage_channel: Channel,

    /// Channel to the notifications event-aggregation service (event publishing).
    pub event_publishing_channel: Option<Channel>,

    /// Channel to the notifications event-consumer service (durable event stream).
    ///
    /// Consumed without trace propagation, unlike the other two: see
    /// [`trace_layer`].
    pub event_consumer_channel: Option<Channel>,

    /// The interceptor selected by [`OutboundAuthArgs`] at startup.
    pub interceptor: AuthInterceptor,

    /// `TokenRefresher` for the selected flow; call sites invoke
    /// `refresher.refresh().await` on `tonic::Code::Unauthenticated` and
    /// retry the original request once.
    pub refresher: Arc<dyn TokenRefresher>,

    /// Startup wait + expiry probe for the same token source as `refresher`.
    pub token_gate: TokenGate,
}

/// Builder configuration for the outbound-gRPC layer.
pub struct OutboundClientsConfig {
    pub auth: OutboundAuthArgs,
    pub storage_endpoint: String,
    /// Optional event-aggregation endpoint. `None` skips that channel; calls
    /// to `publish_event` are then no-ops with a warning log.
    pub event_publishing_endpoint: Option<String>,
    /// Optional event-consumer endpoint. `None` skips that channel; the
    /// event-consumer binaries fail at startup if their flag is required.
    pub event_consumer_endpoint: Option<String>,
}

impl OutboundClients {
    /// Constructs [`OutboundClients`] from the given config. Performs:
    ///   1. Build either [`TokenManager`] (in-process flow, with OIDC discovery)
    ///      or [`AccessTokenFileProvider`] (sidecar flow) based on the args.
    ///   2. Wait until a usable bearer is cached (fail fast on timeout).
    ///   3. Spawn the background refresh / watch loop.
    ///   4. Connect outbound channels only after the token is ready.
    ///   5. Return the bundle for clone + use at each outbound call site.
    pub async fn build(
        cfg: OutboundClientsConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if !cfg.auth.one_mode_enabled() {
            return Err(
                "Exactly one of --enable-client-credentials / --enable-access-token-file \
                 must be set"
                    .into(),
            );
        }

        let (interceptor, token_gate) = if cfg.auth.access_token_file.enabled {
            let path = PathBuf::from(&cfg.auth.access_token_file.file_path);
            let poll = cfg.auth.access_token_file.poll_interval;
            let provider = Arc::new(AccessTokenFileProvider::new(path.clone(), poll));
            let gate = TokenGate::access_token_file(provider.clone(), path);
            gate.wait_until_ready(TOKEN_READY_TIMEOUT).await?;

            let watcher = provider.clone();
            tokio::spawn(async move {
                watcher.start_watch_loop().await;
            });

            let interceptor =
                AuthInterceptor::AccessTokenFile(AccessTokenFileInterceptor::new(provider));
            (interceptor, gate)
        } else {
            let http = Arc::new(reqwest::Client::new());
            let openid =
                load_openid_configuration(&cfg.auth.client_credentials.openid_configuration, &http)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                        format!(
                            "Failed to load OpenID configuration from {}: {}",
                            cfg.auth.client_credentials.openid_configuration, e
                        )
                        .into()
                    })?;

            let client_secret = cfg.auth.client_credentials.get_client_secret().map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("Failed to read client secret: {}", e).into()
                },
            )?;
            let extra_headers = cfg.auth.client_credentials.get_extra_headers().map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("Failed to parse client credentials extra headers: {}", e).into()
                },
            )?;
            let extra_body_params = cfg
                .auth
                .client_credentials
                .get_extra_body_params()
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!(
                        "Failed to parse client credentials extra body params: {}",
                        e
                    )
                    .into()
                })?;

            let token_manager = Arc::new(TokenManager::new(
                http,
                cfg.auth.client_credentials.client_id.clone(),
                client_secret,
                cfg.auth.client_credentials.client_scope.clone(),
                openid.token_endpoint,
                extra_headers,
                extra_body_params,
            ));

            let gate = TokenGate::client_credentials(token_manager.clone());
            // Background refresh must be running before (or while) we wait, so a
            // failed first exchange can still succeed via a later loop tick.
            let tm = token_manager.clone();
            tokio::spawn(async move {
                tm.start_refresh_loop().await;
            });
            gate.wait_until_ready(TOKEN_READY_TIMEOUT).await?;

            let interceptor = AuthInterceptor::ClientCredentials(
                ClientCredentialsInterceptor::new(token_manager.get_token_handle()),
            );
            (interceptor, gate)
        };

        let refresher = token_gate.refresher();

        let storage_channel = build_channel(&cfg.storage_endpoint, "storage").await?;
        let event_publishing_channel = match &cfg.event_publishing_endpoint {
            Some(ep) => Some(build_channel(ep, "event-publishing").await?),
            None => None,
        };
        let event_consumer_channel = match &cfg.event_consumer_endpoint {
            Some(ep) => Some(build_channel(ep, "event-consumer").await?),
            None => None,
        };

        Ok(OutboundClients {
            storage_channel,
            event_publishing_channel,
            event_consumer_channel,
            interceptor,
            refresher,
            token_gate,
        })
    }
}

/// Keepalive for every channel this builder produces.
///
/// The values match `notifications::connection`, which is the tuning already
/// proven against the event-consumer service — the longest-lived and most
/// keepalive-sensitive of the three. The same builder also serves storage and
/// event publishing, so they are deliberately conservative rather than the
/// shorter intervals `storage::create_storage_channel` uses on its own channel:
/// a server that rejects PINGs as too frequent answers with GOAWAY, and one
/// builder cannot know which of three servers it is talking to.
///
/// This is not a defense against an idle-connection reaper. Layer 4 load
/// balancers do not count HTTP/2 PING frames as activity, and a TCP keepalive
/// slower than their idle timeout does not help either. What keepalive buys is
/// noticing a peer that has genuinely gone away. Noticing a *silent* stream
/// without an error is left to peer-aligned reconnect triggers (token near
/// expiry; for WorkerEventQueue, withheld-ACK recycle) rather than an idle teardown.
const HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(300);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(20);
const TCP_KEEPALIVE: Duration = Duration::from_secs(300);

async fn build_channel(
    endpoint: &str,
    name: &str,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{}", endpoint)
    };
    info!("[OUTBOUND] Connecting {} channel to {}", name, url);
    let endpoint = tonic::transport::Endpoint::from_shared(url.clone())
        .map_err(|e| format!("Invalid {} endpoint {}: {}", name, url, e))?
        .connect_timeout(Duration::from_secs(10))
        .http2_keep_alive_interval(HTTP2_KEEPALIVE_INTERVAL)
        .keep_alive_timeout(KEEPALIVE_TIMEOUT)
        .keep_alive_while_idle(true)
        .tcp_keepalive(Some(TCP_KEEPALIVE));
    match endpoint.connect().await {
        Ok(ch) => Ok(ch),
        Err(e) => {
            error!("[OUTBOUND] Failed to connect {} channel: {}", name, e);
            Err(format!("Failed to connect to {} at {}: {}", name, url, e).into())
        }
    }
}

/// Convenience helper used by service binaries with parsed runtime config.
pub async fn build_outbound_clients(
    auth: OutboundAuthArgs,
    runtime: &RuntimeConfigArgs,
) -> Result<Arc<OutboundClients>, String> {
    OutboundClients::build(OutboundClientsConfig {
        auth,
        storage_endpoint: runtime.endpoints.storage_grpc_endpoint.clone(),
        event_publishing_endpoint: runtime
            .endpoints
            .notifications_event_aggregation_endpoint
            .clone(),
        event_consumer_endpoint: runtime
            .endpoints
            .notifications_event_consumer_endpoint
            .clone(),
    })
    .await
    .map(Arc::new)
    .map_err(|e| format!("Failed to build OutboundClients: {e}"))
}

/// Failure modes that can occur when [`OutboundClients::get_auth_token`] is
/// called. Callers use the variant to decide whether the error is retryable
/// (transient OAuth/interceptor failure) or permanent (misconfiguration).
#[derive(Debug)]
pub enum GetAuthTokenError {
    /// The interceptor rejected or errored during the token-injection call.
    /// This is typically a transient OAuth fetch failure and is **retryable**.
    InterceptorCallFailed(tonic::Status),
    /// The interceptor returned a request with no `authorization` metadata
    /// key. This indicates a configuration problem and is **permanent**.
    MissingAuthorizationHeader,
    /// The `authorization` header value is not valid UTF-8. Permanent.
    InvalidHeaderEncoding,
    /// The token is not in `Bearer <non-empty>` form. Permanent.
    InvalidTokenFormat,
}

impl std::fmt::Display for GetAuthTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InterceptorCallFailed(s) => write!(f, "interceptor call failed: {s}"),
            Self::MissingAuthorizationHeader => {
                write!(f, "no 'authorization' header injected by interceptor")
            }
            Self::InvalidHeaderEncoding => {
                write!(f, "'authorization' header is not valid UTF-8")
            }
            Self::InvalidTokenFormat => {
                write!(f, "'authorization' header is not a non-empty Bearer token")
            }
        }
    }
}

impl std::error::Error for GetAuthTokenError {}

impl GetAuthTokenError {
    /// Returns `true` for errors that may resolve on retry (transient
    /// OAuth/network failures). Returns `false` for permanent configuration
    /// errors where retrying the same operation will keep failing.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::InterceptorCallFailed(_))
    }
}

impl OutboundClients {
    /// The storage channel with trace propagation, for `Client::new`.
    ///
    /// Storage authenticates per request through `add_auth_headers`, so there is
    /// no interceptor in this stack.
    pub fn traced_storage(&self) -> TracedChannel {
        trace_layer(self.storage_channel.clone())
    }

    /// The event-publishing channel with trace propagation and auth, for
    /// `Client::new`. `None` when no aggregation endpoint is configured.
    pub fn traced_event_publishing(&self) -> Option<TracedAuthedChannel> {
        let channel = self.event_publishing_channel.clone()?;
        Some(trace_authed(channel, self.interceptor.clone()))
    }

    /// Return the service identity's current bearer token.
    ///
    /// Event-driven storage operations have no caller-supplied token, while the
    /// existing storage metadata helpers accept a raw token. Invoke the selected
    /// interceptor on an empty request so both supported auth modes keep using
    /// their canonical token-validation behavior.
    ///
    /// Returns a [`GetAuthTokenError`] variant that lets callers distinguish
    /// retryable failures (OAuth fetch) from permanent mis-configuration.
    pub fn get_auth_token(&self) -> Result<String, GetAuthTokenError> {
        use tonic::service::Interceptor;

        let mut interceptor = self.interceptor.clone();
        let req = interceptor
            .call(tonic::Request::new(()))
            .map_err(GetAuthTokenError::InterceptorCallFailed)?;

        let header_value = req
            .metadata()
            .get("authorization")
            .ok_or(GetAuthTokenError::MissingAuthorizationHeader)?;

        let header_str = header_value
            .to_str()
            .map_err(|_| GetAuthTokenError::InvalidHeaderEncoding)?;

        header_str
            .strip_prefix("Bearer ")
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .ok_or(GetAuthTokenError::InvalidTokenFormat)
    }

    /// Build a non-connecting [`OutboundClients`] suitable for unit tests that
    /// never actually issue an outbound RPC. All channels are constructed via
    /// `Endpoint::connect_lazy`, the interceptor is a static-token
    /// `ClientCredentialsInterceptor`, and the refresher is a no-op (so 401-retry
    /// / reconnect paths do not hit the network). Tests that need a live
    /// refresher must inject one explicitly.
    ///
    /// Production code paths construct `OutboundClients` via
    /// [`OutboundClients::build`] from real configuration; this helper is
    /// exposed crate-wide so other workspace crates can build a stand-in
    /// `OutboundClients` for unit/integration tests.
    #[doc(hidden)]
    pub fn for_tests() -> Self {
        use std::sync::{Arc, Mutex};
        use thumbnail_auth::{ClientCredentialsInterceptor, Token, TokenManager};

        let lazy = |url: &str| {
            tonic::transport::Endpoint::from_shared(url.to_string())
                .expect("static endpoint url should parse")
                .connect_lazy()
        };

        let token_manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "test-client".into(),
            "test-secret".into(),
            "test-scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        {
            let handle: Arc<Mutex<_>> = token_manager.get_token_handle();
            *handle.lock().expect("token mutex") = Some(Token::new("test-token".to_string(), 3600));
        }
        let token_gate = crate::auth::TokenGate::client_credentials(token_manager.clone());
        let interceptor = AuthInterceptor::ClientCredentials(ClientCredentialsInterceptor::new(
            token_manager.get_token_handle(),
        ));

        OutboundClients {
            storage_channel: lazy("http://127.0.0.1:1"),
            event_publishing_channel: Some(lazy("http://127.0.0.1:2")),
            event_consumer_channel: Some(lazy("http://127.0.0.1:3")),
            interceptor,
            refresher: Arc::new(NoopRefresher),
            token_gate,
        }
    }

    /// As [`Self::for_tests`], but with no token to hand out.
    ///
    /// The fixture for "outbound auth is broken", which callers have to classify
    /// distinctly from a failure of the work itself — a credential outage fails
    /// every message identically and says nothing about any of them.
    #[doc(hidden)]
    pub fn for_tests_without_credentials() -> Self {
        use std::sync::Arc;
        use thumbnail_auth::{ClientCredentialsInterceptor, TokenManager};

        let token_manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "test-client".into(),
            "test-secret".into(),
            "test-scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        // Empty handle shared by interceptor and gate so tests that probe expiry
        // see the same auth state the outbound call uses. Refresher stays a
        // no-op — inject a live one when the test needs network refresh.
        let token_gate = crate::auth::TokenGate::client_credentials(token_manager.clone());
        OutboundClients {
            interceptor: AuthInterceptor::ClientCredentials(ClientCredentialsInterceptor::new(
                token_manager.get_token_handle(),
            )),
            refresher: Arc::new(NoopRefresher),
            token_gate,
            ..Self::for_tests()
        }
    }

    /// As [`Self::for_tests`], but a refresh actually changes the token.
    ///
    /// For the `Unauthenticated` refresh-and-reissue branch, which is the only path
    /// that drives one client twice. With the default no-op refresher a test can
    /// prove the reissue happened but not that it carried a new credential — see
    /// [`RefreshToNewToken`].
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn for_tests_with_token_refresh() -> Self {
        let base = Self::for_tests();
        let gate = base.token_gate.clone();
        OutboundClients {
            refresher: std::sync::Arc::new(RefreshToNewToken { gate }),
            ..base
        }
    }

    /// The token [`Self::for_tests`] starts with, before any refresh.
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub const TEST_TOKEN: &'static str = "test-token";

    /// The token [`Self::for_tests_with_token_refresh`] plants on refresh.
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub const TEST_TOKEN_REFRESHED: &'static str = "test-token-refreshed";

    /// As [`Self::for_tests`], but publishing events to `publisher_addr`.
    ///
    /// For tests that assert on published events, which are otherwise invisible:
    /// see `test_fixtures::mock_publisher_server`.
    #[doc(hidden)]
    pub fn for_tests_publishing_to(publisher_addr: std::net::SocketAddr) -> Self {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{publisher_addr}"))
            .expect("mock publisher endpoint url should parse")
            .connect_lazy();

        OutboundClients {
            event_publishing_channel: Some(channel),
            ..Self::for_tests()
        }
    }
}

/// Test-only refresher that never touches the network.
struct NoopRefresher;

#[tonic::async_trait]
impl TokenRefresher for NoopRefresher {
    async fn refresh(&self) {}
}

/// Test-only refresher that plants a *visibly different* token.
///
/// [`NoopRefresher`] leaves the rejected token in place, which is enough for tests
/// that only need the 401 path not to hit the network — but it makes the
/// refresh-and-reissue branch untestable in the way that matters. With the token
/// unchanged, a test can see that a second call was made and cannot see whether it
/// carried a refreshed credential, so a reissue that resent the rejected token
/// passes. The token this plants (`test-token-refreshed`) differs from the one
/// [`OutboundClients::for_tests`] starts with, so the two attempts are
/// distinguishable at the server.
#[cfg(any(test, feature = "test-utils"))]
struct RefreshToNewToken {
    gate: TokenGate,
}

#[cfg(any(test, feature = "test-utils"))]
#[tonic::async_trait]
impl TokenRefresher for RefreshToNewToken {
    async fn refresh(&self) {
        self.gate.restore_fresh_token_for_tests();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serial_test::serial;

    /// Minimal wrapper that flattens [`OutboundAuthArgs`] so we can exercise
    /// clap parsing in isolation from the binary. This stays in lock-step
    /// with the unified `server` top-level CLI shape, which embeds
    /// `OutboundAuthArgs` for outbound client authentication wiring.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        outbound: OutboundAuthArgs,
    }

    /// Every env var the [`OutboundAuthArgs`] surface reads via
    /// `#[arg(env = "...")]`. Tests below clear all of these before parsing
    /// so a polluted process environment (CI runners, dotenv-style local
    /// shells, the GitLab `rust:test` job's `variables:` block, etc.) does
    /// not leak into the test outcome.
    /// Snapshot the relevant env vars, clear them, and return a guard that
    /// restores them on drop. Combined with `#[serial]` this gives each
    /// test a fully hermetic env regardless of runner / shell state.
    fn cleared_env() -> EnvGuard {
        let snapshot: Vec<(&'static str, Option<String>)> = crate::outbound_auth_env_keys()
            .iter()
            .map(|k| (*k, crate::config::raw_env_value(k)))
            .collect();
        for k in crate::outbound_auth_env_keys() {
            std::env::remove_var(k);
        }
        EnvGuard { snapshot }
    }

    struct EnvGuard {
        snapshot: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.snapshot {
                match v {
                    Some(value) => std::env::set_var(k, value),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// Spec: "service starts with the client-credentials mode enabled".
    /// We can't actually run a server here (the test would need an OIDC
    /// server + connectable upstreams), but we can assert that clap accepts
    /// the env shape and that `one_mode_enabled` correctly reports `true`,
    /// which is what `OutboundClients::build` gates the wiring decision on.
    #[test]
    #[serial]
    fn only_client_credentials_enabled_parses_and_is_valid() {
        let _guard = cleared_env();
        let cli = TestCli::try_parse_from([
            "test",
            "--enable-client-credentials",
            "--client-credentials-openid-configuration=https://example/.well-known/openid-configuration",
            "--client-id=test-id",
            "--client-secret=test-secret",
            "--client-scope=test-scope",
        ])
        .expect("clap should accept only-client-credentials");
        assert!(cli.outbound.client_credentials.enabled);
        assert!(!cli.outbound.access_token_file.enabled);
        assert!(cli.outbound.one_mode_enabled());
    }

    /// Spec: "service starts with the access-token-file mode enabled".
    #[test]
    #[serial]
    fn only_access_token_file_enabled_parses_and_is_valid() {
        let _guard = cleared_env();
        let cli = TestCli::try_parse_from([
            "test",
            "--enable-access-token-file",
            "--access-token-file-path=/etc/secrets/access-token/token",
        ])
        .expect("clap should accept only-access-token-file");
        assert!(!cli.outbound.client_credentials.enabled);
        assert!(cli.outbound.access_token_file.enabled);
        assert!(cli.outbound.one_mode_enabled());
    }

    /// Spec: "service refuses to start when both modes are enabled".
    /// clap's `conflicts_with` on each `enabled` flag rejects this combo at
    /// argument-parsing time, with a message that names both flags.
    #[test]
    #[serial]
    fn both_modes_enabled_is_rejected_by_clap() {
        let _guard = cleared_env();
        let err = TestCli::try_parse_from([
            "test",
            "--enable-client-credentials",
            "--client-credentials-openid-configuration=https://example/.well-known/openid-configuration",
            "--client-id=test-id",
            "--client-secret=test-secret",
            "--client-scope=test-scope",
            "--enable-access-token-file",
            "--access-token-file-path=/etc/secrets/access-token/token",
        ])
        .expect_err("clap must reject both --enable-client-credentials and --enable-access-token-file");
        let msg = err.to_string();
        assert!(
            msg.contains("--enable-client-credentials")
                || msg.contains("--enable-access-token-file"),
            "expected clap error to name the conflicting flags, got: {msg}"
        );
    }

    /// Spec: "service refuses to start when neither mode is enabled".
    /// clap accepts this combo at parse-time (since `requires_ifs` only
    /// fires when `enabled=true`), but `one_mode_enabled` returns `false`
    /// and `OutboundClients::build` rejects it before any channel is
    /// constructed.
    #[tokio::test]
    #[serial]
    async fn neither_mode_enabled_is_rejected_by_build() {
        let _guard = cleared_env();
        let cli = TestCli::try_parse_from(["test"]).expect("clap should parse with no flags set");
        assert!(!cli.outbound.client_credentials.enabled);
        assert!(!cli.outbound.access_token_file.enabled);
        assert!(!cli.outbound.one_mode_enabled());

        let result = OutboundClients::build(OutboundClientsConfig {
            auth: cli.outbound,
            storage_endpoint: "http://127.0.0.1:1".to_string(),
            event_publishing_endpoint: None,
            event_consumer_endpoint: None,
        })
        .await;
        let err = match result {
            Ok(_) => panic!("OutboundClients::build must reject zero-mode configuration"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("--enable-client-credentials")
                && msg.contains("--enable-access-token-file"),
            "expected build error to name both auth flags, got: {msg}"
        );
    }

    #[tokio::test]
    async fn get_auth_token_extracts_the_service_token() {
        assert_eq!(
            OutboundClients::for_tests()
                .get_auth_token()
                .expect("test interceptor should always yield a valid token"),
            "test-token"
        );
    }

    /// The fixture callers use to exercise their credential-failure path has to
    /// actually fail, or the tests built on it prove nothing.
    #[tokio::test]
    async fn the_credential_less_fixture_yields_no_token() {
        let error = OutboundClients::for_tests_without_credentials()
            .get_auth_token()
            .expect_err("a fixture with no token must not produce one");

        assert!(
            error.is_retryable(),
            "an absent token is a transient OAuth condition, not poison input: {error}"
        );
    }
}
