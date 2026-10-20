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

use super::protos::envoy::config::core::v3::header_value_option::HeaderAppendAction;
use super::protos::envoy::config::core::v3::{HeaderValue, HeaderValueOption};
use super::protos::envoy::r#type::v3::{HttpStatus, StatusCode};
use super::protos::envoy::service::auth::v3::authorization_server::{
    Authorization, AuthorizationServer,
};
use super::protos::envoy::service::auth::v3::check_response::HttpResponse;
use super::protos::envoy::service::auth::v3::{
    CheckRequest, CheckResponse, DeniedHttpResponse, OkHttpResponse,
};
use super::protos::google::rpc::Status as RpcStatus;

use crate::settings::AppSettings;
use crate::AppState;
use opentelemetry::KeyValue;
use permission_auth::AuthError;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tonic_health::pb::health_server::Health;
use tonic_health::pb::health_server::HealthServer;

/// Service implementation.
///
/// This receives requests from the tonic server, processes them as needed (e.g.
/// extract metadata), and then invokes the controller to run business logic.
#[derive(Debug)]
pub struct AuthService {
    settings: AppSettings,
    metrics_server_duration: opentelemetry::metrics::Histogram<u64>,
    state: AppState,
}

#[tonic::async_trait]
impl Authorization for AuthService {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn check(
        &self,
        request: Request<CheckRequest>,
    ) -> Result<Response<CheckResponse>, Status> {
        tracing::debug!("authorization check start");
        let metrics_duration_start_time = std::time::Instant::now();
        // This section is overly verbose, but debugging gRPC headers is not fun
        let unauthorized_response_401 = Ok(Response::new(CheckResponse {
            status: Some(RpcStatus {
                code: 16, //gRPC UNAUTHENTICATED
                message: "".to_string(),
                details: vec![],
            }),
            dynamic_metadata: None,
            http_response: Some(HttpResponse::DeniedResponse(DeniedHttpResponse {
                status: Some(HttpStatus {
                    code: i32::from(StatusCode::Unauthorized),
                }),
                headers: Vec::new(),
                body: String::new(),
            })),
        }));

        let forbidden_response_403 = Ok(Response::new(CheckResponse {
            status: Some(RpcStatus {
                code: 7, //gRPC PERMISSION_DENIED
                message: "".to_string(),
                details: vec![],
            }),
            dynamic_metadata: None,
            http_response: Some(HttpResponse::DeniedResponse(DeniedHttpResponse {
                status: Some(HttpStatus {
                    code: i32::from(StatusCode::Forbidden),
                }),
                headers: Vec::new(),
                body: String::new(),
            })),
        }));

        let check_request = request.into_inner();
        let attrs = match check_request.attributes {
            Some(attrs) => attrs,
            None => {
                tracing::warn!("failed to get attributes from request");
                return unauthorized_response_401;
            }
        };
        let attr_request = match attrs.request {
            Some(attr_request) => attr_request,
            None => {
                tracing::warn!("failed to get request field from attributes");
                return unauthorized_response_401;
            }
        };
        let http_field = match attr_request.http {
            Some(http_field) => http_field,
            None => {
                tracing::warn!("failed to get http field from request");
                return unauthorized_response_401;
            }
        };
        let auth_header = match http_field.headers.get("authorization") {
            Some(auth_header) => auth_header,
            None => {
                tracing::warn!("failed to get authorization header field");
                return unauthorized_response_401;
            }
        };
        match auth_header.split_once(' ') {
            Some(("Bearer", token)) => {
                if token.len() > self.settings.auth.max_token_length {
                    tracing::warn!(
                        token_len = token.len(),
                        max_token_length = self.settings.auth.max_token_length,
                        "authorization failed: token exceeds max_token_length"
                    );
                    return unauthorized_response_401;
                }
                match permission_auth::verify_user_token(
                    token,
                    &self.settings.auth,
                    &self.state.client,
                    &self.state.cache,
                )
                .await
                {
                    Ok(principal) => {
                        tracing::debug!("authorization successful for user {}", principal.id);
                        let duration = metrics_duration_start_time.elapsed().as_millis() as u64;
                        let attributes = &[KeyValue::new(
                            "rpc",
                            "envoy.service.auth.v3.authorization_server.Authorization/Check",
                        )];
                        self.metrics_server_duration.record(duration, attributes);

                        let (username_source, username) = self
                            .settings
                            .server
                            .username_claims
                            .iter()
                            .find_map(|claim| {
                                principal
                                    .info
                                    .as_ref()
                                    .and_then(|claims| claims.get(claim))
                                    .and_then(|value| value.as_str())
                                    .map(|value| (claim.as_str(), value))
                            })
                            .unwrap_or_default();

                        let headers = vec![
                            upstream_header(&self.settings.server.user_id_header, &principal.id),
                            upstream_header(&self.settings.server.username_header, username),
                            upstream_header(
                                &self.settings.server.username_source_claim_header,
                                username_source,
                            ),
                        ];

                        Ok(Response::new(CheckResponse {
                            status: Some(RpcStatus {
                                code: 0, //gRPC OK
                                message: "Authorization Success".to_string(),
                                details: vec![],
                            }),
                            dynamic_metadata: None,

                            http_response: Some(HttpResponse::OkResponse(OkHttpResponse {
                                headers,
                                ..Default::default()
                            })),
                        }))
                    }
                    Err(auth_error) => {
                        tracing::warn!("authorization failed: {}", auth_error);
                        match auth_error {
                            AuthError::Forbidden() => forbidden_response_403,
                            _ => unauthorized_response_401,
                        }
                    }
                }
            }
            _ => {
                tracing::warn!("authorization failed: missing header");
                unauthorized_response_401
            }
        }
    }
}

/// Builds a header that overwrites any client-supplied value so downstream
/// services cannot receive a spoofed identity.
fn upstream_header(key: &str, value: &str) -> HeaderValueOption {
    HeaderValueOption {
        header: Some(HeaderValue {
            key: key.to_string(),
            value: value.to_string(),
            ..Default::default()
        }),
        append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
        ..Default::default()
    }
}

#[tracing::instrument(level = "info")]
pub(crate) fn init_grpc_auth_service(
    settings: &AppSettings,
    meter: &opentelemetry::metrics::Meter,
    state: AppState,
) -> AuthorizationServer<impl Authorization> {
    let metrics_server_duration = meter
        .u64_histogram("rpc.server.duration")
        .with_description("Duration of gRPC requests")
        .with_unit("ms")
        .build();

    let service = AuthService {
        settings: settings.clone(),
        metrics_server_duration,
        state,
    };
    AuthorizationServer::new(service).max_decoding_message_size(
        settings
            .server
            .tonic
            .max_decoding_message_size
            .try_into()
            .expect("Invalid max_decoding_message_size"),
    )
}

#[tracing::instrument(level = "info")]
pub(crate) async fn init_auth_service() -> HealthServer<impl Health> {
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<AuthorizationServer<AuthService>>()
        .await;
    health_service
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::protos::envoy::service::auth::v3::attribute_context::{
        HttpRequest, Request as AttributeRequest,
    };
    use crate::handlers::protos::envoy::service::auth::v3::AttributeContext;
    use base64::prelude::*;
    use permission_auth::AuthCache;
    use std::future::Future;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tracing_subscriber::fmt::MakeWriter;

    #[tokio::test]
    async fn check_does_not_log_bearer_token_or_cookie() {
        let service = auth_service(AppSettings::default());
        let token = unsigned_jwt(r#"{"sub":"user-1","jti":"TOKEN-MARKER"}"#);
        let request = check_request(&[
            ("authorization", format!("Bearer {token}")),
            ("cookie", "session=COOKIE-MARKER".to_string()),
        ]);

        let (response, logs) = capture_logs(service.check(Request::new(request))).await;

        assert_eq!(denied_status(response), StatusCode::Unauthorized as i32);
        assert!(logs.contains("authorization failed"), "logs: {logs}");
        assert!(!logs.contains(&token), "logs: {logs}");
        assert!(!logs.contains("COOKIE-MARKER"), "logs: {logs}");
    }

    #[tokio::test]
    async fn failed_check_does_not_log_unverified_claims() {
        let service = auth_service(AppSettings::default());
        let token = unsigned_jwt(r#"{"sub":"victim\nFORGED-ENTRY level=INFO"}"#);

        let (response, logs) =
            capture_logs(service.check(Request::new(bearer_request(&token)))).await;

        assert_eq!(denied_status(response), StatusCode::Unauthorized as i32);
        assert!(logs.contains("authorization failed"), "logs: {logs}");
        assert!(!logs.contains("victim"), "logs: {logs}");
        assert!(!logs.contains("FORGED-ENTRY"), "logs: {logs}");
    }

    #[tokio::test]
    async fn oversized_token_is_rejected_without_decoding_or_verbose_logging() {
        let service = auth_service(AppSettings::default());
        let padding = "x".repeat(16 * 1024);
        let token = unsigned_jwt(&format!(
            r#"{{"sub":"OVERSIZED-MARKER","pad":"{padding}"}}"#
        ));

        let (response, logs) =
            capture_logs(service.check(Request::new(bearer_request(&token)))).await;

        assert_eq!(denied_status(response), StatusCode::Unauthorized as i32);
        assert!(logs.contains("max_token_length"), "logs: {logs}");
        assert!(!logs.contains("OVERSIZED-MARKER"), "logs: {logs}");
        assert!(logs.len() < 1024, "logged {} bytes", logs.len());
    }

    fn auth_service(settings: AppSettings) -> AuthService {
        AuthService {
            settings,
            metrics_server_duration: opentelemetry::global::meter("test")
                .u64_histogram("rpc.server.duration")
                .build(),
            state: AppState {
                cache: Arc::new(AuthCache::new(
                    16,
                    Duration::from_secs(60),
                    16,
                    Duration::from_secs(60),
                )),
                client: Arc::new(reqwest::Client::new()),
            },
        }
    }

    fn unsigned_jwt(payload: &str) -> String {
        let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT","kid":"k1"}"#);
        let payload = BASE64_URL_SAFE_NO_PAD.encode(payload);
        let signature = BASE64_URL_SAFE_NO_PAD.encode("invalid-signature");
        format!("{header}.{payload}.{signature}")
    }

    fn bearer_request(token: &str) -> CheckRequest {
        check_request(&[("authorization", format!("Bearer {token}"))])
    }

    fn check_request(headers: &[(&str, String)]) -> CheckRequest {
        CheckRequest {
            attributes: Some(AttributeContext {
                request: Some(AttributeRequest {
                    http: Some(HttpRequest {
                        headers: headers
                            .iter()
                            .map(|(name, value)| (name.to_string(), value.clone()))
                            .collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }
    }

    async fn capture_logs<F: Future>(future: F) -> (F::Output, String) {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .finish();
        let output = {
            let _guard = tracing::subscriber::set_default(subscriber);
            future.await
        };
        (output, buffer.contents())
    }

    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl LogBuffer {
        fn contents(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl Write for LogBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for LogBuffer {
        type Writer = LogBuffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn denied_status(response: Result<Response<CheckResponse>, Status>) -> i32 {
        match response.unwrap().into_inner().http_response {
            Some(HttpResponse::DeniedResponse(denied)) => denied.status.unwrap().code,
            other => panic!("expected a denied response, got {other:?}"),
        }
    }
}
