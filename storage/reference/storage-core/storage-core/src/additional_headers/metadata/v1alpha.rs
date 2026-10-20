// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::traits::{
    async_trait,
    error::Result,
    metadata::v1alpha::{
        DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, Metadata,
        UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
    },
    Request,
};

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> Metadata for AdditionalHeaderManager<S>
where
    S: Metadata + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "additional_headers::get_metadata", skip_all)]
    async fn get_metadata(&self, mut request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.get_metadata(request).await
    }

    #[instrument(name = "additional_headers::update_metadata", skip_all)]
    async fn update_metadata(&self, mut request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.update_metadata(request).await
    }

    #[instrument(name = "additional_headers::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        mut request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.update_metadata_batch(request).await
    }

    #[instrument(name = "additional_headers::delete_metadata", skip_all)]
    async fn delete_metadata(&self, mut request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.delete_metadata(request).await
    }
}
