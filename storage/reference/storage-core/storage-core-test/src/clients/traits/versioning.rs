// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::versioning::v1alpha::{
    EnumerateVersionsRequest, EnumerateVersionsResponse, IsVersionedRequest, IsVersionedResponse,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

#[tonic::async_trait]
pub trait Versioning
where
    Self: std::marker::Send + std::marker::Sync + 'static,
{
    async fn enumerate_versions(
        &mut self,
        _request: EnumerateVersionsRequest,
    ) -> Result<ReceiverStream<Result<EnumerateVersionsResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn is_versioned(&mut self, _request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
        unimplemented!()
    }
}
