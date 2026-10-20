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

use crate::backend::metadata::*;
use async_trait::async_trait;
use std::sync::Arc;
use storage_core::{
    increment_error,
    layer::Layer,
    metrics::utils::{latency_base, results_counter_base, MethodMetrics, TimedStream},
    record_operation,
    traits::{Request, Result},
};

/// Metadata API metrics layer.
pub struct MetadataMetricsLayer {
    metrics: Arc<Metrics>,
}

impl Default for MetadataMetricsLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataMetricsLayer {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Metrics::new()),
        }
    }
}

impl<S> Layer<S> for MetadataMetricsLayer {
    type Service = MetadataMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetadataMetricsService {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

/// Metadata API metrics service.
pub struct MetadataMetricsService<S> {
    inner: S,
    metrics: Arc<Metrics>,
}

#[async_trait]
impl<S: Metadata> Metadata for MetadataMetricsService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let _timer = self.metrics.get_metadata_batch.latency.start();
        record_operation!(
            self.metrics.get_metadata_batch,
            self.inner.get_metadata_batch(request).await
        )
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let timer = self.metrics.query_metadata.latency.start();
        let stream = self
            .inner
            .query_metadata(request, options)
            .await
            .inspect_err(|error| increment_error!(self.metrics.query_metadata, error))?;

        Ok(Box::pin(TimedStream::new(
            stream,
            timer,
            self.metrics.query_metadata.clone(),
        )))
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let _timer = self.metrics.update_metadata_batch.latency.start();
        record_operation!(
            self.metrics.update_metadata_batch,
            self.inner.update_metadata_batch(request).await
        )
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let _timer = self.metrics.delete_metadata_batch.latency.start();
        record_operation!(
            self.metrics.delete_metadata_batch,
            self.inner.delete_metadata_batch(request).await
        )
    }
}

/// Metadata API metrics.
struct Metrics {
    get_metadata_batch: MethodMetrics,
    query_metadata: Arc<MethodMetrics>,
    update_metadata_batch: MethodMetrics,
    delete_metadata_batch: MethodMetrics,
}

impl Metrics {
    fn new() -> Self {
        Self {
            get_metadata_batch: MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "batch_get_metadata",
                "v1alpha",
                "metadata",
            ),
            query_metadata: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "query_metadata",
                "v1alpha",
                "metadata",
            )),
            update_metadata_batch: MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "update_metadata_batch",
                "v1alpha",
                "metadata",
            ),
            delete_metadata_batch: MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete_metadata_batch",
                "v1alpha",
                "metadata",
            ),
        }
    }
}
