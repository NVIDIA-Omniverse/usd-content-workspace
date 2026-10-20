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

//! gRPC access-log Tower layer. Installed server-wide (around the tonic router,
//! outside the per-service auth interceptor) so it records every gRPC access,
//! including unauthenticated calls (`UNAUTHENTICATED`), unknown methods and
//! services (`UNIMPLEMENTED`), and decode errors, in addition to handled RPCs. It
//! installs the request-scoped collector (see [`crate::access_log`]) around the
//! call and emits exactly one set of access lines after it completes. A
//! per-request [`AccessLog`] handle is placed on the request extensions (visible
//! to the handler through tonic's http-to-gRPC extension propagation) so handlers
//! record `params` and the RPC outcome; for failures that never reach a handler
//! the terminal status and message are read from the response `grpc-status` and
//! `grpc-message` headers. Infrastructure and discovery services (health,
//! reflection, capability discovery) are never logged.

use crate::access_log::{
    emit_grpc, resolve_header_extras, scoped, AccessLog, AccessLogConfig, AccessLogState,
};
use axum::http;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

/// Wraps the whole gRPC server so business and invalid RPCs emit access logs,
/// skipping the infrastructure services named in `skip_services`.
#[derive(Clone)]
pub struct GrpcAccessLogLayer {
    config: AccessLogConfig,
    skip_services: Arc<Vec<String>>,
}

impl GrpcAccessLogLayer {
    pub fn new(config: AccessLogConfig, skip_services: Vec<String>) -> Self {
        Self {
            config,
            skip_services: Arc::new(skip_services),
        }
    }
}

impl<S> Layer<S> for GrpcAccessLogLayer {
    type Service = GrpcAccessLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcAccessLogService {
            inner,
            config: self.config.clone(),
            skip_services: self.skip_services.clone(),
        }
    }
}

#[derive(Clone)]
pub struct GrpcAccessLogService<S> {
    inner: S,
    config: AccessLogConfig,
    skip_services: Arc<Vec<String>>,
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for GrpcAccessLogService<S> {
    const NAME: &'static str = S::NAME;
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for GrpcAccessLogService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: http::Request<ReqBody>) -> Self::Future {
        let config = self.config.clone();
        let skip_services = self.skip_services.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        request.extensions_mut().insert(AccessLog);

        let operation = request.uri().path().to_string();
        let service = grpc_service_name(&operation);
        if !config.enabled || skip_services.iter().any(|name| name == service) {
            return Box::pin(inner.call(request));
        }

        let header_extras = resolve_header_extras(&config, |header| {
            request
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        });
        let state = AccessLogState::new(operation, header_extras, config);

        Box::pin(scoped(state, async move {
            let result = inner.call(request).await;
            if let Ok(response) = &result {
                // A terminal error carries `grpc-status`/`grpc-message` in the
                // header map; a successful unary keeps them in the trailers, so
                // this is `None` and the recorded outcome defaults to `OK`. The
                // message is percent-encoded on the wire, so decode it exactly as
                // a gRPC client would.
                if let Some(status) = tonic::Status::from_header_map(response.headers()) {
                    let message =
                        Some(status.message().to_string()).filter(|value| !value.is_empty());
                    AccessLog.record_grpc_outcome(i32::from(status.code()), message);
                }
            }
            emit_grpc();
            result
        }))
    }
}

/// Extracts the gRPC service full name from a request path (`/<service>/<method>`).
fn grpc_service_name(path: &str) -> &str {
    path.trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
}
