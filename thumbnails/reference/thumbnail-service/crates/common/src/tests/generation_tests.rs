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

//! Tests for `create_thumbnails` business logic.
//!
//! Covers in this file:
//! - Validation (empty address → InvalidArgument without touching storage)
//! - Stat error propagation (NOT_FOUND, UNAUTHENTICATED → typed errors)
//!
//! Dispatch success and failure paths are tested at the handler level in
//! `crates/service/src/http/tests.rs` and `crates/service/src/grpc/tests.rs`,
//! each against its own ephemeral generate publisher.
//!
//! Fan-out mechanics — per-specification dispatch, partial failures, and the
//! batch behavior of the CREATE path — are unit-tested in
//! `work_events::dispatch`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_server::{
    FileObjectService as FileObjectServiceTrait, FileObjectServiceServer,
};
use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::{
    AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
    CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse,
    CopyRequest, CopyResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponse,
    FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, GetNativeMetadataSupportRequest,
    GetNativeMetadataSupportResponse, GetOptimisticLockingSupportRequest,
    GetOptimisticLockingSupportResponse, MoveRequest, MoveResponse, ReadFromAddressRequest,
    ReadFromAddressResponse, ReadRequest, ReadResponse, ResourceIdentity, ResourceInfo,
    StatRequest, StatResponse, UploadPartRequest, UploadPartResponse, WriteRequest, WriteResponse,
};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::generation::{create_thumbnails, CreateThumbnailsError};
use crate::OutboundClients;

// ─── Minimal mock Stat server ────────────────────────────────────────────────

#[derive(Clone)]
#[allow(dead_code)] // Identity variant used by task-queue tests that run with --ignored
enum StatBehavior {
    Identity(String),
    Error(tonic::Code),
}

struct MockStatService {
    behavior: Arc<Mutex<StatBehavior>>,
}

type BoxStream<T> = std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

#[tonic::async_trait]
impl FileObjectServiceTrait for MockStatService {
    type EnumerateStream = BoxStream<EnumerateResponse>;
    type ReadStream = BoxStream<ReadResponse>;
    type ReadFromAddressStream = BoxStream<ReadFromAddressResponse>;
    type WriteStream = BoxStream<WriteResponse>;

    async fn stat(&self, _: Request<StatRequest>) -> Result<Response<StatResponse>, Status> {
        match self
            .behavior
            .lock()
            .expect("mock stat mutex poisoned")
            .clone()
        {
            StatBehavior::Identity(id) => Ok(Response::new(StatResponse {
                resource_info: Some(ResourceInfo {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: id,
                    }),
                    metadata: None,
                }),
            })),
            StatBehavior::Error(code) => Err(Status::new(code, "mock: injected")),
        }
    }

    async fn enumerate(
        &self,
        _: Request<EnumerateRequest>,
    ) -> Result<Response<Self::EnumerateStream>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn read(&self, _: Request<ReadRequest>) -> Result<Response<Self::ReadStream>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn read_from_address(
        &self,
        _: Request<ReadFromAddressRequest>,
    ) -> Result<Response<Self::ReadFromAddressStream>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn fetch_write_type_info(
        &self,
        _: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<Response<FetchWriteTypeInfoResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn write(
        &self,
        _: tonic::Request<tonic::Streaming<WriteRequest>>,
    ) -> Result<Response<Self::WriteStream>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn complete_redirect_upload(
        &self,
        _: Request<CompleteRedirectUploadRequest>,
    ) -> Result<Response<CompleteRedirectUploadResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn upload_part(
        &self,
        _: Request<UploadPartRequest>,
    ) -> Result<Response<UploadPartResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn complete_multipart_upload(
        &self,
        _: Request<CompleteMultipartUploadRequest>,
    ) -> Result<Response<CompleteMultipartUploadResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn abort_multipart_upload(
        &self,
        _: Request<AbortMultipartUploadRequest>,
    ) -> Result<Response<AbortMultipartUploadResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn delete(&self, _: Request<DeleteRequest>) -> Result<Response<DeleteResponse>, Status> {
        Err(Status::unimplemented(""))
    }
    async fn copy(&self, _: Request<CopyRequest>) -> Result<Response<CopyResponse>, Status> {
        Err(Status::unimplemented(""))
    }

    async fn r#move(&self, _: Request<MoveRequest>) -> Result<Response<MoveResponse>, Status> {
        Err(Status::unimplemented(""))
    }

    async fn get_native_metadata_support(
        &self,
        _: Request<GetNativeMetadataSupportRequest>,
    ) -> Result<Response<GetNativeMetadataSupportResponse>, Status> {
        Err(Status::unimplemented(""))
    }

    async fn get_optimistic_locking_support(
        &self,
        _: Request<GetOptimisticLockingSupportRequest>,
    ) -> Result<Response<GetOptimisticLockingSupportResponse>, Status> {
        Err(Status::unimplemented(""))
    }
}

async fn start_mock_stat(behavior: StatBehavior) -> (SocketAddr, Arc<Mutex<StatBehavior>>) {
    let behavior = Arc::new(Mutex::new(behavior));
    let b2 = behavior.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(FileObjectServiceServer::new(MockStatService {
                behavior: b2,
            }))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await;
    });
    (addr, behavior)
}

fn outbound_with_storage(storage_addr: SocketAddr) -> Arc<OutboundClients> {
    let ch = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
        .unwrap()
        .connect_lazy();
    Arc::new(OutboundClients {
        storage_channel: ch,
        ..OutboundClients::for_tests()
    })
}

// ─── Validation tests (no network) ───────────────────────────────────────────

#[tokio::test]
async fn empty_resource_address_returns_invalid_argument_without_calling_storage() {
    crate::test_fixtures::init_test_runtime_config("generation-tests");
    // Use a storage addr that would refuse connections — we must never reach it.
    let outbound = Arc::new(OutboundClients::for_tests());
    let runtime = crate::current_runtime_config();

    let result = create_thumbnails("   ", &[], "token", outbound, runtime).await;

    assert!(
        matches!(result, Err(CreateThumbnailsError::InvalidArgument(_))),
        "expected InvalidArgument for blank address, got: {result:?}"
    );
}

#[tokio::test]
async fn blank_address_string_returns_invalid_argument() {
    crate::test_fixtures::init_test_runtime_config("generation-tests");
    let outbound = Arc::new(OutboundClients::for_tests());
    let runtime = crate::current_runtime_config();

    let result = create_thumbnails("", &[], "token", outbound, runtime).await;

    assert!(matches!(
        result,
        Err(CreateThumbnailsError::InvalidArgument(_))
    ));
}

// ─── Stat error propagation tests ────────────────────────────────────────────

#[tokio::test]
async fn stat_not_found_returns_create_thumbnails_not_found() {
    crate::test_fixtures::init_test_runtime_config("generation-tests");
    let (addr, _) = start_mock_stat(StatBehavior::Error(tonic::Code::NotFound)).await;
    let outbound = outbound_with_storage(addr);
    let runtime = crate::current_runtime_config();

    let result = create_thumbnails(
        "https://storage.example/container/missing.png",
        &[],
        "token",
        outbound,
        runtime,
    )
    .await;

    assert!(
        matches!(result, Err(CreateThumbnailsError::NotFound(_))),
        "expected NotFound, got: {result:?}"
    );
}

#[tokio::test]
async fn stat_unauthenticated_returns_create_thumbnails_unauthenticated() {
    crate::test_fixtures::init_test_runtime_config("generation-tests");
    let (addr, _) = start_mock_stat(StatBehavior::Error(tonic::Code::Unauthenticated)).await;
    let outbound = outbound_with_storage(addr);
    let runtime = crate::current_runtime_config();

    let result = create_thumbnails(
        "https://storage.example/container/image.png",
        &[],
        "expired-token",
        outbound,
        runtime,
    )
    .await;

    assert!(
        matches!(result, Err(CreateThumbnailsError::Unauthenticated(_))),
        "expected Unauthenticated, got: {result:?}"
    );
}
