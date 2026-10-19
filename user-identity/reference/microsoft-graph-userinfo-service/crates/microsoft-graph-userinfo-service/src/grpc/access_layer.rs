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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tonic::{Code, Status};
use tower::{Layer, Service};

use crate::access::AccessLog;

/// Tower layer that records call-level access records for gRPC requests
/// rejected before a handler runs — an unknown method (`UNIMPLEMENTED`) or a
/// request whose payload failed to decode (`INVALID_ARGUMENT`). Handler
/// outcomes (success and every error the service itself returns) are recorded
/// by the per-method emit in [`crate::grpc::service`]; this layer only covers
/// what never reaches a method. Infrastructure and discovery services are
/// excluded.
#[derive(Clone)]
pub struct AccessLogGrpcLayer {
    access: Arc<AccessLog>,
}

impl AccessLogGrpcLayer {
    pub fn new(access: Arc<AccessLog>) -> Self {
        Self { access }
    }
}

impl<S> Layer<S> for AccessLogGrpcLayer {
    type Service = AccessLogGrpc<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AccessLogGrpc {
            inner,
            access: self.access.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AccessLogGrpc<S> {
    inner: S,
    access: Arc<AccessLog>,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for AccessLogGrpc<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        if !self.access.is_enabled() {
            return Box::pin(self.inner.call(request));
        }

        let operation = request.uri().path().to_owned();
        let header_fields = self.access.header_fields_http(request.headers());
        let access = self.access.clone();

        // Clone-and-replace so the instance that was `poll_ready`'d is the one
        // driven to completion, per the tower `Service` contract.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let response = inner.call(request).await?;
            if let Some(status) = response.extensions().get::<Status>() {
                let code = status.code();
                if is_pre_handler_rejection(code) && !is_infra_method(&operation) {
                    access.emit_grpc_call_level(header_fields, operation, code, status.message());
                }
            }
            Ok(response)
        })
    }
}

/// gRPC codes produced only before a handler runs: an unknown method
/// (`UNIMPLEMENTED`) or a payload that failed to decode (`INVALID_ARGUMENT`).
/// The service's handlers never return these, so recording them here does not
/// duplicate the per-method records.
fn is_pre_handler_rejection(code: Code) -> bool {
    matches!(code, Code::Unimplemented | Code::InvalidArgument)
}

/// Whether a gRPC method path belongs to an infrastructure or discovery service
/// that must never be logged (health, reflection, capability discovery).
fn is_infra_method(path: &str) -> bool {
    path.starts_with("/grpc.health.")
        || path.starts_with("/grpc.reflection.")
        || path.starts_with("/nvidia.omniverse.discovery.capabilities.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_handler_rejection_covers_decode_and_unknown_method_only() {
        assert!(is_pre_handler_rejection(Code::Unimplemented));
        assert!(is_pre_handler_rejection(Code::InvalidArgument));
        assert!(!is_pre_handler_rejection(Code::NotFound));
        assert!(!is_pre_handler_rejection(Code::PermissionDenied));
        assert!(!is_pre_handler_rejection(Code::Unauthenticated));
        assert!(!is_pre_handler_rejection(Code::Ok));
    }

    #[test]
    fn infra_methods_are_excluded() {
        assert!(is_infra_method("/grpc.health.v1.Health/Check"));
        assert!(is_infra_method(
            "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo"
        ));
        assert!(is_infra_method(
            "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"
        ));
        assert!(is_infra_method(
            "/nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices"
        ));
        assert!(!is_infra_method(
            "/nvidia.omniverse.userinfo.v1alpha.UserInfoService/GetUser"
        ));
        assert!(!is_infra_method("/unknown.Service/Method"));
    }
}
