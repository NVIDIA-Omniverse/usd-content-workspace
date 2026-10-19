// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tracing::instrument;

use storage_core_builder::traits::{
    error::Result,
    metadata::v1alpha::{
        DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, Metadata,
        UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
    },
    Request,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> Metadata for EgressThrottle<S>
where
    S: Metadata,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "egress_throttle::get_metadata", skip_all)]
    async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.get_metadata(request).await
    }

    #[instrument(name = "egress_throttle::update_metadata", skip_all)]
    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.update_metadata(request).await
    }

    #[instrument(name = "egress_throttle::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        // One permit per batch call, not per entry. This gates layer-level
        // concurrency (number of in-flight `update_metadata_batch` calls) rather
        // than backend-level concurrency (number of HTTP/SQL requests the inner
        // backend ultimately issues). The trait change to surface the whole batch
        // is what lets a batched backend (e.g., Postgres array-based SQL) treat
        // it as one operation; if the inner backend instead fans out via
        // `update_metadata_batch_fanout`, those per-entry calls run under the
        // backend's own concurrency cap (`MAX_BATCH_CONCURRENCY`), not this
        // semaphore. Treating a batch as N permits would force `acquire_many(N)`
        // and deadlock when N exceeds the configured semaphore capacity.
        let _permit = self.semaphore.acquire().await;
        self.inner.update_metadata_batch(request).await
    }

    #[instrument(name = "egress_throttle::delete_metadata", skip_all)]
    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.delete_metadata(request).await
    }
}
