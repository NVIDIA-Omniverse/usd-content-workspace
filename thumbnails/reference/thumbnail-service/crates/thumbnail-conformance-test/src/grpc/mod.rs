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

//! gRPC suite: exercises `AssociateThumbnail`, `DisassociateThumbnail`, and
//! `CreateThumbnails` on `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService`,
//! mirroring the REST suite's coverage through the gRPC status-code contract
//! documented on each RPC in `proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto`.

pub(crate) mod thumbnail_service;
#[cfg(test)]
pub(crate) use thumbnail_service::{ASSOCIATION_REJECTIONS, CREATE_CASES};

use std::time::Duration;

use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_client::ThumbnailServiceClient;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::service::Interceptor;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::Request;

use crate::style::{ConfigurationFault, TestError};

pub use thumbnail_service::run;

/// Classify a status the case did not expect, the gRPC counterpart of
/// `rest::classify_unexpected_status`.
///
/// `UNAUTHENTICATED` identifies missing or rejected credentials; `PERMISSION_DENIED`
/// identifies denied access, including source-storage permissions. Neither certifies
/// the contract, and they need different operator advice. `UNIMPLEMENTED` is the
/// endpoint not serving this API at all — the gRPC analogue of a `404` with no
/// `ErrorResponse`, and the status a deployment predating an RPC returns.
pub(crate) fn classify_unexpected_status(status: &RpcError, context: &str) -> TestError {
    if matches!(status, RpcError::ClientTimeout) {
        return TestError::configuration(ConfigurationFault::ClientTimeout,
            format!("{context}: incomplete observation after --http-timeout; increase the timeout and rerun"));
    }
    match status.code() {
        tonic::Code::Unauthenticated => TestError::configuration(
            ConfigurationFault::RejectedCredentials,
            format!(
                "{context} returned {}, so the run never reached the contract: {status}",
                status.code()
            ),
        ),
        tonic::Code::PermissionDenied => TestError::configuration(
            ConfigurationFault::AccessDenied,
            format!("{context} returned PERMISSION_DENIED: {status}"),
        ),
        tonic::Code::Unimplemented => TestError::configuration(
            ConfigurationFault::EndpointMismatch,
            format!(
                "{context} returned UNIMPLEMENTED, so this deployment does not serve the RPC \
                 rather than serving it wrongly: check --grpc-url and the deployment's API \
                 version. Status: {status}"
            ),
        ),
        code => TestError::new(format!("{context} returned {code}: {status}")),
    }
}

/// Attaches the resolved bearer token, if any, to every outbound gRPC call.
#[derive(Clone)]
pub(crate) struct BearerInterceptor {
    token: Option<String>,
}

impl Interceptor for BearerInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, tonic::Status> {
        if let Some(token) = &self.token {
            let value: MetadataValue<_> = format!("Bearer {token}")
                .parse()
                .map_err(|_| tonic::Status::invalid_argument("bearer token is not valid ASCII"))?;
            request.metadata_mut().insert("authorization", value);
        }
        Ok(request)
    }
}

#[derive(Debug)]
pub(crate) enum RpcError {
    Server(tonic::Status),
    ClientTimeout,
}

impl RpcError {
    pub fn code(&self) -> tonic::Code {
        match self {
            Self::Server(status) => status.code(),
            Self::ClientTimeout => tonic::Code::DeadlineExceeded,
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Self::Server(status) => status.message(),
            Self::ClientTimeout => "client --http-timeout elapsed",
        }
    }
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server(status) => status.fmt(f),
            Self::ClientTimeout => write!(f, "client --http-timeout elapsed"),
        }
    }
}

async fn bounded_call<T>(
    timeout: Duration,
    call: impl std::future::Future<Output = Result<T, tonic::Status>>,
) -> Result<T, RpcError> {
    tokio::time::timeout(timeout, call)
        .await
        .map_err(|_| RpcError::ClientTimeout)?
        .map_err(RpcError::Server)
}

pub struct Client {
    inner: ThumbnailServiceClient<InterceptedService<Channel, BearerInterceptor>>,
    timeout: Duration,
}

impl Client {
    pub(crate) async fn associate_thumbnail(
        &mut self,
        request: proto::AssociateThumbnailRequest,
    ) -> Result<tonic::Response<proto::AssociateThumbnailResponse>, RpcError> {
        bounded_call(self.timeout, self.inner.associate_thumbnail(request)).await
    }
    pub(crate) async fn disassociate_thumbnail(
        &mut self,
        request: proto::DisassociateThumbnailRequest,
    ) -> Result<tonic::Response<proto::DisassociateThumbnailResponse>, RpcError> {
        bounded_call(self.timeout, self.inner.disassociate_thumbnail(request)).await
    }
    pub(crate) async fn create_thumbnails(
        &mut self,
        request: proto::CreateThumbnailsRequest,
    ) -> Result<tonic::Response<proto::CreateThumbnailsResponse>, RpcError> {
        bounded_call(self.timeout, self.inner.create_thumbnails(request)).await
    }
}

/// Establish readiness on the selected gRPC endpoint, including delayed startup.
/// Explicit local deadlines preserve the distinction from server-returned statuses.
pub async fn wait_for_ready(
    grpc_url: &str,
    tls_enabled: bool,
    token: Option<String>,
    request_timeout: Duration,
    retries: u32,
    interval: Duration,
) -> Result<Client, TestError> {
    if retries == 0 {
        return Err(TestError::new(
            "gRPC readiness requires --wait-retries >= 1; run incomplete",
        ));
    }
    let mut endpoint = Endpoint::from_shared(grpc_url.to_string())
        .map_err(|error| TestError::new(format!("invalid --grpc-url: {error}")))?;
    if tls_enabled {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new())
            .map_err(|error| TestError::new(format!("failed to configure gRPC TLS: {error}")))?;
    }
    let mut last_error = TestError::new("gRPC readiness was not observed");
    for attempt in 1..=retries {
        let connected = tokio::time::timeout(request_timeout, endpoint.connect()).await;
        match connected {
            Ok(Ok(channel)) => {
                let mut health = tonic_health::pb::health_client::HealthClient::with_interceptor(
                    channel.clone(),
                    BearerInterceptor {
                        token: token.clone(),
                    },
                );
                let request = tonic_health::pb::HealthCheckRequest {
                    service: "nvidia.omniverse.thumbnail.v1alpha.ThumbnailService".to_string(),
                };
                match bounded_call(request_timeout, health.check(request)).await {
                    Ok(response)
                        if response.get_ref().status
                            == tonic_health::pb::health_check_response::ServingStatus::Serving
                                as i32 =>
                    {
                        return Ok(Client {
                            inner: ThumbnailServiceClient::with_interceptor(
                                channel,
                                BearerInterceptor { token },
                            ),
                            timeout: request_timeout,
                        });
                    }
                    Ok(_) => last_error = TestError::new("gRPC health is not SERVING"),
                    Err(error) => {
                        last_error = classify_unexpected_status(&error, "gRPC health readiness")
                    }
                }
            }
            Ok(Err(error)) => {
                last_error = TestError::new(format!(
                    "gRPC readiness could not connect to --grpc-url: {error}"
                ))
            }
            Err(_) => {
                last_error =
                    classify_unexpected_status(&RpcError::ClientTimeout, "gRPC readiness connect")
            }
        }
        if attempt < retries {
            tokio::time::sleep(interval).await;
        }
    }
    last_error.message = format!(
        "gRPC readiness exhausted {retries} attempts; run incomplete: {}",
        last_error.message
    );
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn client_deadlines_are_distinct_from_server_statuses() {
        let error = bounded_call::<()>(Duration::from_millis(1), std::future::pending())
            .await
            .unwrap_err();
        assert_eq!(
            classify_unexpected_status(&error, "call").configuration_fault,
            Some(ConfigurationFault::ClientTimeout)
        );
        for code in [
            tonic::Code::Cancelled,
            tonic::Code::Internal,
            tonic::Code::DeadlineExceeded,
            tonic::Code::Unavailable,
        ] {
            let error = RpcError::Server(tonic::Status::new(code, "server response"));
            assert_eq!(
                classify_unexpected_status(&error, "call").configuration_fault,
                None
            );
        }
        for (code, fault) in [
            (
                tonic::Code::Unauthenticated,
                ConfigurationFault::RejectedCredentials,
            ),
            (
                tonic::Code::PermissionDenied,
                ConfigurationFault::AccessDenied,
            ),
        ] {
            let error = RpcError::Server(tonic::Status::new(code, "credentials"));
            assert_eq!(
                classify_unexpected_status(&error, "call").configuration_fault,
                Some(fault)
            );
        }
        let error = RpcError::Server(tonic::Status::unimplemented("wrong endpoint"));
        assert_eq!(
            classify_unexpected_status(&error, "call").configuration_fault,
            Some(ConfigurationFault::EndpointMismatch)
        );
    }
}

#[cfg(test)]
mod test_server;

#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[tokio::test]
    async fn delayed_readiness_succeeds_and_unready_target_is_incomplete() {
        let server = test_server::start(
            test_server::FakeService::default(),
            Some(Duration::from_millis(30)),
        )
        .await;
        wait_for_ready(
            &server.url,
            false,
            None,
            Duration::from_secs(1),
            20,
            Duration::from_millis(5),
        )
        .await
        .unwrap();
        let server = test_server::start(test_server::FakeService::default(), None).await;
        let error = wait_for_ready(
            &server.url,
            false,
            None,
            Duration::from_secs(1),
            2,
            Duration::from_millis(1),
        )
        .await
        .err()
        .unwrap();
        assert!(error.message.contains("readiness") && error.message.contains("run incomplete"));
    }
}
