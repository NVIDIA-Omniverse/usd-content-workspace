// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::metrics::utils::TimedStream;
use crate::traits::{
    versioning::v1alpha::{
        EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, IsVersionedRequest, IsVersionedResponse,
        Versioning,
    },
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
        let timer = self.enumerate_versions_metrics_v1alpha.latency.start();

        match self
            .inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
        {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.enumerate_versions_metrics_v1alpha.clone());
                Ok(Box::pin(EntryCountStream::new(
                    timed,
                    self.enumerate_versions_item_count_v1alpha.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.enumerate_versions_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        let _timer = self.is_versioned_metrics_v1alpha.latency.start();

        record_operation!(
            self.is_versioned_metrics_v1alpha,
            self.inner.is_versioned(request).await
        )
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
        let timer = self.enumerate_versions_metrics_v1alpha.latency.start();

        match self
            .inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
        {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.enumerate_versions_metrics_v1alpha.clone(),
            ))),
            Err(e) => {
                increment_error!(self.enumerate_versions_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        let _timer = self.is_versioned_metrics_v1alpha.latency.start();

        record_operation!(
            self.is_versioned_metrics_v1alpha,
            self.inner.is_versioned(request).await
        )
    }
}

impl EntryCount for EnumerateVersionsResponseWithContinueToken {
    fn entry_count(&self) -> u64 {
        self.response.items.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Inner versioning mock that records the `x-request-id` metadata it receives.
    #[derive(Clone, Default)]
    struct CapturingInner {
        request_ids: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait]
    impl Versioning for CapturingInner {
        fn inner(&self) -> Option<&dyn Versioning> {
            None
        }

        async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
            let request_id = request
                .metadata()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            self.request_ids.lock().expect("request ids").push(request_id);
            Ok(IsVersionedResponse { is_versioned: true })
        }
    }

    fn request() -> Request<IsVersionedRequest> {
        let mut request = Request::new(IsVersionedRequest {
            resource_address: "https://bucket.example/object".to_string(),
        });
        request
            .metadata_mut()
            .insert("x-request-id", "abc".parse().expect("metadata value"));
        request
    }

    /// Both metrics layers forward the caller's request unchanged and return the
    /// inner response.
    #[tokio::test]
    async fn is_versioned_forwards_request_and_response() {
        let inner = CapturingInner::default();

        let operation = OperationMetrics::new(inner.clone(), "test-backend".to_string());
        assert!(
            operation
                .is_versioned(request())
                .await
                .expect("operation metrics")
                .is_versioned
        );
        let sdk = SdkMetrics::new(inner.clone(), "test-backend".to_string());
        assert!(sdk.is_versioned(request()).await.expect("sdk metrics").is_versioned);

        assert_eq!(
            *inner.request_ids.lock().expect("request ids"),
            vec![Some("abc".to_string()); 2]
        );
    }
}
