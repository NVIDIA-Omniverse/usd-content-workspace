// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::metrics::utils::TimedStream;
use crate::traits::{
    versioning::v1beta::{EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, Versioning},
    BoxStream, Request, Result, StreamingOptions,
};

use super::super::{OperationMetrics, SdkMetrics};
use crate::metrics::utils::{EntryCount, EntryCountStream};

#[async_trait]
impl<S> Versioning for OperationMetrics<S>
where
    S: Versioning + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let timer = self.enumerate_versions_metrics_v1beta.latency.start();

        match self
            .inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
        {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.enumerate_versions_metrics_v1beta.clone());
                Ok(Box::pin(EntryCountStream::new(
                    timed,
                    self.enumerate_versions_item_count_v1beta.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.enumerate_versions_metrics_v1beta, e);
                Err(e)
            },
        }
    }
}

#[async_trait]
impl<S> Versioning for SdkMetrics<S>
where
    S: Versioning + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let timer = self.enumerate_versions_metrics_v1beta.latency.start();

        match self
            .inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
        {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.enumerate_versions_metrics_v1beta.clone(),
            ))),
            Err(e) => {
                increment_error!(self.enumerate_versions_metrics_v1beta, e);
                Err(e)
            },
        }
    }
}

impl EntryCount for EnumerateVersionsResponseWithContinueToken {
    fn entry_count(&self) -> u64 {
        self.response.items.len() as u64
    }
}
