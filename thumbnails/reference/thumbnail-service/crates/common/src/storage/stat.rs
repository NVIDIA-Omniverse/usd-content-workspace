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

//! Storage file-object Stat operation — resolves resource_identity from resource_address.

use std::sync::Arc;

use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_client::FileObjectServiceClient;
use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::StatRequest;
use thiserror::Error;
use tonic::Code;
use tracing::debug;

use crate::OutboundClients;

/// Why a storage Stat call failed.
#[derive(Debug, Error)]
pub enum StatError {
    /// The resource_address does not exist in storage.
    #[error("resource not found in storage")]
    NotFound,
    /// The caller's token was missing or rejected by the storage service.
    #[error("{0}")]
    Unauthenticated(String),
    /// The token was accepted, but the caller is not allowed to read the resource.
    ///
    /// Distinct from [`StatError::Unauthenticated`] because the remedies differ:
    /// a rejected credential can be refreshed, an authorization decision cannot.
    /// Folding the two together tells a caller to retry with a new token when a
    /// new token cannot help.
    #[error("{0}")]
    PermissionDenied(String),
    /// Any other failure (connection error, internal storage error, etc.).
    #[error("{0}")]
    Internal(String),
}

/// Resolve `resource_identity` (`encoded_identity`) for a file at `resource_address`
/// by calling `FileObjectService.Stat` on the storage service.
///
/// Uses the caller's Bearer token; the storage service is reached via the
/// `outbound.storage_channel` already established at startup.
pub async fn stat_file_object(
    resource_address: &str,
    auth_token: &str,
    outbound: Arc<OutboundClients>,
) -> Result<String, StatError> {
    let mut client = FileObjectServiceClient::new(outbound.traced_storage());

    let mut request = tonic::Request::new(StatRequest {
        resource_address: resource_address.to_string(),
    });
    // A stalled storage service holds the caller's CreateThumbnails request open
    // for as long as it stalls: connect timeout bounds establishing the
    // connection and keepalive notices a dead peer, but neither bounds a peer
    // that accepted the call and went quiet. Per-call rather than on the channel,
    // because the same builder serves the durable consume stream.
    request.set_timeout(
        crate::current_runtime_config()
            .storage_channel
            .storage_grpc_request_timeout,
    );
    super::add_auth_headers(&mut request, auth_token)
        .map_err(|e| StatError::Internal(format!("failed to set auth headers: {e}")))?;

    debug!(resource_address, "stat_file_object: calling storage Stat");

    let response = client
        .stat(request)
        .await
        .map_err(|status| match status.code() {
            Code::NotFound => StatError::NotFound,
            Code::Unauthenticated => {
                StatError::Unauthenticated(format!("storage Stat rejected token: {status}"))
            }
            Code::PermissionDenied => StatError::PermissionDenied(format!(
                "storage Stat denied access to the resource: {status}"
            )),
            _ => StatError::Internal(format!("storage Stat failed: {status}")),
        })?;

    let resource_info = response.into_inner().resource_info.ok_or_else(|| {
        StatError::Internal("storage Stat returned empty resource_info".to_string())
    })?;

    let encoded_identity = resource_info
        .resource_identity
        .ok_or_else(|| {
            StatError::Internal(
                "storage Stat returned resource_info with no resource_identity".to_string(),
            )
        })?
        .encoded_identity;

    if encoded_identity.trim().is_empty() {
        return Err(StatError::Internal(
            "storage Stat returned an empty encoded_identity".to_string(),
        ));
    }

    debug!(
        resource_address,
        encoded_identity = %encoded_identity,
        "stat_file_object: resolved identity"
    );
    Ok(encoded_identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_server::{
        FileObjectService as FileObjectServiceTrait, FileObjectServiceServer,
    };
    use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::{
        AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest,
        CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest, DeleteResponse,
        EnumerateRequest, EnumerateResponse, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse,
        GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse,
        GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse, MoveRequest,
        MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse,
        ResourceIdentity, ResourceInfo, StatRequest, StatResponse, UploadPartRequest,
        UploadPartResponse, WriteRequest, WriteResponse,
    };
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;
    use tonic::{Request, Response, Status};

    /// Controls what the mock Stat server returns.
    #[derive(Clone)]
    enum StatBehavior {
        /// Return the given encoded_identity.
        Identity(String),
        /// Fail with the given status code.
        Error(tonic::Code),
    }

    struct MockStatServer {
        behavior: Arc<Mutex<StatBehavior>>,
    }

    type BoxStream<T> =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

    #[tonic::async_trait]
    impl FileObjectServiceTrait for MockStatServer {
        type EnumerateStream = BoxStream<EnumerateResponse>;
        type ReadStream = BoxStream<ReadResponse>;
        type ReadFromAddressStream = BoxStream<ReadFromAddressResponse>;
        type WriteStream = BoxStream<WriteResponse>;

        async fn stat(
            &self,
            _request: Request<StatRequest>,
        ) -> Result<Response<StatResponse>, Status> {
            let behavior = self
                .behavior
                .lock()
                .expect("mock stat server mutex must not be poisoned")
                .clone();

            match behavior {
                StatBehavior::Identity(id) => Ok(Response::new(StatResponse {
                    resource_info: Some(ResourceInfo {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: id,
                        }),
                        metadata: None,
                    }),
                })),
                StatBehavior::Error(code) => {
                    Err(Status::new(code, "mock stat server: injected error"))
                }
            }
        }

        async fn enumerate(
            &self,
            _: Request<EnumerateRequest>,
        ) -> Result<Response<Self::EnumerateStream>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn read(
            &self,
            _: Request<ReadRequest>,
        ) -> Result<Response<Self::ReadStream>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn read_from_address(
            &self,
            _: Request<ReadFromAddressRequest>,
        ) -> Result<Response<Self::ReadFromAddressStream>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn fetch_write_type_info(
            &self,
            _: Request<FetchWriteTypeInfoRequest>,
        ) -> Result<Response<FetchWriteTypeInfoResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn write(
            &self,
            _: tonic::Request<tonic::Streaming<WriteRequest>>,
        ) -> Result<Response<Self::WriteStream>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn complete_redirect_upload(
            &self,
            _: Request<CompleteRedirectUploadRequest>,
        ) -> Result<Response<CompleteRedirectUploadResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn upload_part(
            &self,
            _: Request<UploadPartRequest>,
        ) -> Result<Response<UploadPartResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn complete_multipart_upload(
            &self,
            _: Request<CompleteMultipartUploadRequest>,
        ) -> Result<Response<CompleteMultipartUploadResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn abort_multipart_upload(
            &self,
            _: Request<AbortMultipartUploadRequest>,
        ) -> Result<Response<AbortMultipartUploadResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn delete(
            &self,
            _: Request<DeleteRequest>,
        ) -> Result<Response<DeleteResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn copy(&self, _: Request<CopyRequest>) -> Result<Response<CopyResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn r#move(&self, _: Request<MoveRequest>) -> Result<Response<MoveResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn get_native_metadata_support(
            &self,
            _: Request<GetNativeMetadataSupportRequest>,
        ) -> Result<Response<GetNativeMetadataSupportResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }

        async fn get_optimistic_locking_support(
            &self,
            _: Request<GetOptimisticLockingSupportRequest>,
        ) -> Result<Response<GetOptimisticLockingSupportResponse>, Status> {
            Err(Status::unimplemented("not used in stat tests"))
        }
    }

    /// Spin up an in-process mock FileObjectService and return its address along
    /// with a handle to control the Stat behavior.
    async fn start_mock_stat_server(
        behavior: StatBehavior,
    ) -> (SocketAddr, Arc<Mutex<StatBehavior>>) {
        let behavior = Arc::new(Mutex::new(behavior));
        let behavior_for_server = behavior.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("must bind ephemeral port");
        let addr = listener
            .local_addr()
            .expect("bound listener must expose local addr");

        tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(FileObjectServiceServer::new(MockStatServer {
                    behavior: behavior_for_server,
                }))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await;
        });

        (addr, behavior)
    }

    /// Build an `OutboundClients` whose storage_channel points at the given address.
    fn outbound_for_addr(addr: SocketAddr) -> Arc<OutboundClients> {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .expect("test address must be valid")
            .connect_lazy();
        Arc::new(OutboundClients {
            storage_channel: channel,
            ..OutboundClients::for_tests()
        })
    }

    #[tokio::test]
    async fn stat_file_object_returns_encoded_identity_on_success() {
        let (addr, _) = start_mock_stat_server(StatBehavior::Identity(
            "encoded-identity-abc123".to_string(),
        ))
        .await;
        let outbound = outbound_for_addr(addr);

        let result = stat_file_object(
            "https://storage.example/container/image.png",
            "test-token",
            outbound,
        )
        .await;

        assert_eq!(result.unwrap(), "encoded-identity-abc123");
    }

    #[tokio::test]
    async fn stat_file_object_maps_not_found_to_stat_error_not_found() {
        let (addr, _) = start_mock_stat_server(StatBehavior::Error(tonic::Code::NotFound)).await;
        let outbound = outbound_for_addr(addr);

        let result = stat_file_object(
            "https://storage.example/container/missing.png",
            "test-token",
            outbound,
        )
        .await;

        assert!(
            matches!(result, Err(StatError::NotFound)),
            "expected StatError::NotFound, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn stat_file_object_maps_unauthenticated_to_stat_error_unauthenticated() {
        let (addr, _) =
            start_mock_stat_server(StatBehavior::Error(tonic::Code::Unauthenticated)).await;
        let outbound = outbound_for_addr(addr);

        let result = stat_file_object(
            "https://storage.example/container/image.png",
            "expired-token",
            outbound,
        )
        .await;

        assert!(
            matches!(result, Err(StatError::Unauthenticated(_))),
            "expected StatError::Unauthenticated, got: {result:?}"
        );
    }

    /// `PermissionDenied` is reported as itself, not folded into
    /// `Unauthenticated`. The two have different remedies: a rejected credential
    /// can be refreshed, an authorization decision cannot, so conflating them
    /// tells the caller to retry with a new token when no token will help.
    #[tokio::test]
    async fn stat_file_object_keeps_permission_denied_distinct_from_unauthenticated() {
        let (addr, _) =
            start_mock_stat_server(StatBehavior::Error(tonic::Code::PermissionDenied)).await;
        let outbound = outbound_for_addr(addr);

        let result = stat_file_object(
            "https://storage.example/container/image.png",
            "no-perm-token",
            outbound,
        )
        .await;

        assert!(
            matches!(result, Err(StatError::PermissionDenied(_))),
            "expected StatError::PermissionDenied for PermissionDenied, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn stat_file_object_maps_internal_error_to_stat_error_internal() {
        let (addr, _) = start_mock_stat_server(StatBehavior::Error(tonic::Code::Internal)).await;
        let outbound = outbound_for_addr(addr);

        let result = stat_file_object(
            "https://storage.example/container/image.png",
            "test-token",
            outbound,
        )
        .await;

        assert!(
            matches!(result, Err(StatError::Internal(_))),
            "expected StatError::Internal, got: {result:?}"
        );
    }
}
