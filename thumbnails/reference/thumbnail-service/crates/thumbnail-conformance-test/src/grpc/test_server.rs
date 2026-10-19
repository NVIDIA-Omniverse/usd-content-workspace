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

//! Local protocol double. These tests exercise the client, not a deployed candidate.
use super::*;
use proto::thumbnail_service_server::{ThumbnailService, ThumbnailServiceServer};
use std::sync::{Arc, Mutex};
use tokio_stream::wrappers::TcpListenerStream;

#[derive(Clone, Default)]
pub(super) struct FakeService {
    pub code: Option<tonic::Code>,
    pub delay: Duration,
    pub reject_png_quality: bool,
    pub(super) thumbnails_seen: Arc<Mutex<Vec<String>>>,
    pub(super) associations: Arc<Mutex<Vec<proto::Resource>>>,
    pub(super) create_requests: Arc<Mutex<Vec<proto::CreateThumbnailsRequest>>>,
}
impl FakeService {
    async fn fault(&self) -> Result<(), tonic::Status> {
        tokio::time::sleep(self.delay).await;
        match self.code {
            Some(code) => Err(tonic::Status::new(code, "injected server response")),
            None => Ok(()),
        }
    }
}
#[tonic::async_trait]
impl ThumbnailService for FakeService {
    async fn associate_thumbnail(
        &self,
        request: Request<proto::AssociateThumbnailRequest>,
    ) -> Result<tonic::Response<proto::AssociateThumbnailResponse>, tonic::Status> {
        self.fault().await?;
        let request = request.into_inner();
        if let Some(thumbnail) = &request.thumbnail {
            self.thumbnails_seen
                .lock()
                .unwrap()
                .push(thumbnail.resource_address.clone());
        }
        if request.specification.is_none()
            || request
                .association_target
                .as_ref()
                .is_none_or(|target| target.resource_address.is_empty())
        {
            return Err(tonic::Status::invalid_argument("invalid association"));
        }
        self.associations
            .lock()
            .unwrap()
            .push(request.thumbnail.unwrap());
        Ok(tonic::Response::new(proto::AssociateThumbnailResponse {
            success: true,
            ..Default::default()
        }))
    }
    async fn disassociate_thumbnail(
        &self,
        _: Request<proto::DisassociateThumbnailRequest>,
    ) -> Result<tonic::Response<proto::DisassociateThumbnailResponse>, tonic::Status> {
        self.fault().await?;
        let succeeded = self
            .associations
            .lock()
            .unwrap()
            .drain(..)
            .map(|thumbnail| proto::DisassociateResult {
                thumbnail: Some(thumbnail),
                error_message: None,
            })
            .collect();
        Ok(tonic::Response::new(proto::DisassociateThumbnailResponse {
            succeeded,
            failed: vec![],
        }))
    }
    async fn create_thumbnails(
        &self,
        request: Request<proto::CreateThumbnailsRequest>,
    ) -> Result<tonic::Response<proto::CreateThumbnailsResponse>, tonic::Status> {
        self.fault().await?;
        let request = request.into_inner();
        self.create_requests.lock().unwrap().push(request.clone());
        if request.resource_address.is_empty() || request.specifications.len() > 1000 {
            return Err(tonic::Status::invalid_argument("invalid create request"));
        }
        for spec in request.specifications {
            let Some(proto::thumbnail_specification::Specification::ImageToImage(image)) =
                spec.specification
            else {
                return Err(tonic::Status::invalid_argument("missing spec"));
            };
            if image.width == 0 {
                return Err(tonic::Status::invalid_argument("specification 0 width"));
            }
            if image.quality == Some(200) {
                if matches!(
                    image.encoding_format(),
                    proto::ImageEncodingFormat::Jpeg | proto::ImageEncodingFormat::Webp
                ) {
                    return Err(tonic::Status::invalid_argument("specification 0 quality"));
                }
                if self.reject_png_quality {
                    return Err(tonic::Status::internal("old unconditional validator"));
                }
            }
        }
        Ok(tonic::Response::new(proto::CreateThumbnailsResponse {
            failed: vec![],
        }))
    }
    async fn get_thumbnails(
        &self,
        _: Request<proto::GetThumbnailsRequest>,
    ) -> Result<tonic::Response<proto::GetThumbnailsResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not used by this client"))
    }
}

pub(super) struct TestServer {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(super) async fn start(service: FakeService, ready_after: Option<Duration>) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reporter, health) = tonic_health::server::health_reporter();
    let name = "nvidia.omniverse.thumbnail.v1alpha.ThumbnailService";
    reporter
        .set_service_status(name, tonic_health::ServingStatus::NotServing)
        .await;
    if let Some(delay) = ready_after {
        if delay.is_zero() {
            reporter
                .set_service_status(name, tonic_health::ServingStatus::Serving)
                .await;
        } else {
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                reporter
                    .set_service_status(name, tonic_health::ServingStatus::Serving)
                    .await;
            });
        }
    }
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(health)
            .add_service(ThumbnailServiceServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    TestServer { url, task }
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;
    use clap::Parser;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

    /// An independent gRPC endpoint remains observable after REST readiness fails.
    #[tokio::test]
    async fn failed_rest_readiness_preserves_grpc_coverage_but_never_certifies() {
        for (suite, openapi_status, expect_grpc) in [
            ("all", 200, true),
            ("rest", 200, false),
            ("all", 503, false),
        ] {
            let rest = MockServer::start().await;
            Mock::given(path("/health"))
                .respond_with(ResponseTemplate::new(503))
                .expect(1)
                .mount(&rest)
                .await;
            Mock::given(path(crate::contract::OPENAPI_PATH))
                .respond_with(
                    ResponseTemplate::new(openapi_status)
                        .set_body_json(crate::contract::document_with_bound(1000.into())),
                )
                .expect(if suite == "all" { 1 } else { 0 })
                .mount(&rest)
                .await;
            let service = FakeService::default();
            let creates = service.create_requests.clone();
            let associations = service.thumbnails_seen.clone();
            let grpc = start(service, Some(Duration::ZERO)).await;
            let args = crate::config::Args::parse_from([
                "conformance",
                "--test-suite",
                suite,
                "--rest-url",
                &rest.uri(),
                "--grpc-url",
                &grpc.url,
                "--source-file",
                "source",
                "--wait-retries",
                "1",
                "--http-timeout",
                "1s",
            ]);
            assert!(!crate::run(&args).await.unwrap());
            let requests = rest.received_requests().await.unwrap();
            assert!(requests
                .iter()
                .all(|r| !r.url.path().starts_with("/v1alpha")));
            let creates = creates.lock().unwrap();
            if expect_grpc {
                assert_eq!(creates.len(), super::super::CREATE_CASES.len());
                assert_eq!(associations.lock().unwrap().len(), 3);
                let Some(proto::thumbnail_specification::Specification::ImageToImage(image)) =
                    &creates[0].specifications[0].specification
                else {
                    panic!("missing valid image specification")
                };
                assert_eq!(
                    image.quality, None,
                    "the received valid RPC must omit quality"
                );
            } else {
                assert!(creates.is_empty());
                assert!(associations.lock().unwrap().is_empty());
            }
        }
    }
}
