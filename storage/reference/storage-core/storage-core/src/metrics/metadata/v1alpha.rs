// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tracing::instrument;

use crate::traits::{
    error::Result,
    metadata::v1alpha::{
        DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, Metadata,
        UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
    },
    Request,
};

use super::super::{OperationMetrics, SdkMetrics};

#[async_trait]
impl<S> Metadata for OperationMetrics<S>
where
    S: Metadata + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "operation_metrics::get_metadata", skip_all)]
    async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let _timer = self.get_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.get_metadata_metrics_v1alpha,
            self.inner.get_metadata(request).await
        )
    }

    #[instrument(name = "operation_metrics::update_metadata", skip_all)]
    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let _timer = self.update_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.update_metadata_metrics_v1alpha,
            self.inner.update_metadata(request).await
        )
    }

    #[instrument(name = "operation_metrics::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let _timer = self.update_metadata_batch_metrics_v1alpha.latency.start();

        record_operation!(
            self.update_metadata_batch_metrics_v1alpha,
            self.inner.update_metadata_batch(request).await
        )
    }

    #[instrument(name = "operation_metrics::delete_metadata", skip_all)]
    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let _timer = self.delete_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.delete_metadata_metrics_v1alpha,
            self.inner.delete_metadata(request).await
        )
    }
}

#[async_trait]
impl<S> Metadata for SdkMetrics<S>
where
    S: Metadata + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "sdk_metrics::get_metadata", skip_all)]
    async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let _timer = self.get_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.get_metadata_metrics_v1alpha,
            self.inner.get_metadata(request).await
        )
    }

    #[instrument(name = "sdk_metrics::update_metadata", skip_all)]
    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let _timer = self.update_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.update_metadata_metrics_v1alpha,
            self.inner.update_metadata(request).await
        )
    }

    #[instrument(name = "sdk_metrics::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let _timer = self.update_metadata_batch_metrics_v1alpha.latency.start();

        record_operation!(
            self.update_metadata_batch_metrics_v1alpha,
            self.inner.update_metadata_batch(request).await
        )
    }

    #[instrument(name = "sdk_metrics::delete_metadata", skip_all)]
    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let _timer = self.delete_metadata_metrics_v1alpha.latency.start();

        record_operation!(
            self.delete_metadata_metrics_v1alpha,
            self.inner.delete_metadata(request).await
        )
    }
}
