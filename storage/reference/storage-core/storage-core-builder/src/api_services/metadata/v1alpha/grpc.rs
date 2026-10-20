// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use futures::{stream, Stream};
use tonic::{Request, Response, Status};
use tracing::instrument;
use tracing_futures::Instrument;

use crate::{
    api_services::error::log_grpc_error,
    api_services::slow_stream::SlowGuardedStream,
    ingress::grpc::ToGrpcService,
    traits::{
        async_trait,
        metadata::v1alpha::{
            metadata_service_server::{MetadataService, MetadataServiceServer},
            DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse,
            Metadata as MetadataTrait, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest,
            UpdateMetadataResponse, FILE_DESCRIPTOR_SET,
        },
    },
};
use storage_core_util::log_if_slow;

use super::super::super::REQUEST_ID;
use super::Metadata;

/// Chunk size used when re-paginating a unary `UpdateMetadataBatchResponse`
/// into the gRPC service's streaming wire type. Matches `MAX_BATCH_CONCURRENCY`
/// so each page roughly aligns with one URI-group's worth of fanout work,
/// keeping page sizes small enough that slow consumers see incremental progress.
const BATCH_PAGE_SIZE: usize = 32;

impl<L> ToGrpcService for Metadata<L>
where
    L: MetadataTrait,
{
    type Service = MetadataServiceServer<Metadata<L>>;

    fn service(&self) -> Self::Service {
        MetadataServiceServer::new(self.clone())
    }

    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

#[async_trait]
impl<T: MetadataTrait> MetadataService for Metadata<T> {
    type UpdateMetadataBatchStream =
        std::pin::Pin<Box<dyn Stream<Item = Result<UpdateMetadataBatchResponse, Status>> + Send + 'static>>;

    #[instrument(name = "grpc::get_metadata", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn get_metadata(
        &self,
        request: Request<GetMetadataRequest>,
    ) -> Result<Response<GetMetadataResponse>, Status> {
        let context = request.get_ref().uri.clone();
        Ok(Response::new(
            self.metadata
                .get_metadata(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::update_metadata", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn update_metadata(
        &self,
        request: Request<UpdateMetadataRequest>,
    ) -> Result<Response<UpdateMetadataResponse>, Status> {
        let context = request.get_ref().uri.clone();
        Ok(Response::new(
            self.metadata
                .update_metadata(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::update_metadata_batch", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<Response<Self::UpdateMetadataBatchStream>, Status> {
        let context = "update_metadata_batch";
        let guard = log_if_slow!(guard: "grpc::update_metadata_batch").with_context(context);

        // Batch-size validation lives on the trait (`validate_metadata_batch_size`)
        // so downstream layers and backends see a consistent oversized-batch
        // error shape. The layer chain enforces it before any per-entry work.
        let response = self
            .metadata
            .update_metadata_batch(request.into())
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), context))?;

        let pages = response
            .results
            .chunks(BATCH_PAGE_SIZE)
            .map(|results| {
                Ok(UpdateMetadataBatchResponse {
                    results: results.to_vec(),
                })
            })
            .collect::<Vec<Result<UpdateMetadataBatchResponse, Status>>>();
        let stream = stream::iter(pages).in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    #[instrument(name = "grpc::delete_metadata", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn delete_metadata(
        &self,
        request: Request<DeleteMetadataRequest>,
    ) -> Result<Response<DeleteMetadataResponse>, Status> {
        let context = request.get_ref().uri.clone();
        Ok(Response::new(
            self.metadata
                .delete_metadata(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use futures::StreamExt;

    use super::*;
    use crate::traits::error::Result as StorageResult;

    #[derive(Clone, Default)]
    struct BatchAwareMetadata {
        update_calls: Arc<AtomicUsize>,
        batch_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl MetadataTrait for BatchAwareMetadata {
        fn inner(&self) -> Option<&dyn MetadataTrait> {
            None
        }

        async fn get_metadata(
            &self,
            _request: crate::traits::Request<GetMetadataRequest>,
        ) -> StorageResult<GetMetadataResponse> {
            Ok(GetMetadataResponse::default())
        }

        async fn update_metadata(
            &self,
            _request: crate::traits::Request<UpdateMetadataRequest>,
        ) -> StorageResult<UpdateMetadataResponse> {
            self.update_calls.fetch_add(1, Ordering::SeqCst);
            Ok(UpdateMetadataResponse { etag: "single".into() })
        }

        async fn update_metadata_batch(
            &self,
            request: crate::traits::Request<UpdateMetadataBatchRequest>,
        ) -> StorageResult<UpdateMetadataBatchResponse> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(UpdateMetadataBatchResponse {
                results: request
                    .get_ref()
                    .requests
                    .iter()
                    .enumerate()
                    .map(|(request_index, _)| {
                        storage_core_protos::nvidia::omniverse::storage::metadata::v1alpha::UpdateMetadataBatchEntryResult {
                            request_index: request_index as u32,
                            outcome: Some(
                                storage_core_protos::nvidia::omniverse::storage::metadata::v1alpha::update_metadata_batch_entry_result::Outcome::Success(
                                    UpdateMetadataResponse { etag: "batch".into() },
                                ),
                            ),
                        }
                    })
                    .collect(),
            })
        }

        async fn delete_metadata(
            &self,
            _request: crate::traits::Request<DeleteMetadataRequest>,
        ) -> StorageResult<DeleteMetadataResponse> {
            Ok(DeleteMetadataResponse {})
        }
    }

    #[tokio::test]
    async fn grpc_update_metadata_batch_forwards_to_batch_trait_once() {
        let backend = BatchAwareMetadata::default();
        let update_calls = backend.update_calls.clone();
        let batch_calls = backend.batch_calls.clone();
        let service = Metadata::new(backend);

        let response = MetadataService::update_metadata_batch(
            &service,
            Request::new(UpdateMetadataBatchRequest {
                requests: vec![UpdateMetadataRequest {
                    uri: "s3://bucket/object".into(),
                    user_metadata_key: "key".into(),
                    user_metadata: None,
                    expected_etag: None,
                }],
            }),
        )
        .await
        .unwrap();

        let pages = response.into_inner().collect::<Vec<_>>().await;
        assert_eq!(pages.len(), 1);
        assert_eq!(batch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(update_calls.load(Ordering::SeqCst), 0);
    }
}
