// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::metadata::v1alpha::{
    DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, UpdateMetadataBatchRequest,
    UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

#[tonic::async_trait]
pub trait Metadata
where
    Self: std::marker::Send + std::marker::Sync + 'static,
{
    async fn get_metadata(&mut self, _request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
        unimplemented!()
    }

    async fn update_metadata(&mut self, _request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
        unimplemented!()
    }

    async fn update_metadata_batch(
        &mut self,
        _request: UpdateMetadataBatchRequest,
    ) -> Result<ReceiverStream<Result<UpdateMetadataBatchResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn delete_metadata(&mut self, _request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
        unimplemented!()
    }
}
