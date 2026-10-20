// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    error::Result,
    versioning::v1alpha::{
        EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, IsVersionedRequest, IsVersionedResponse,
        Versioning,
    },
    BoxStream, Request, StreamingOptions,
};

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> Versioning for AdditionalHeaderManager<S>
where
    S: Versioning + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        mut request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
    }

    async fn is_versioned(&self, mut request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        // The manifest layer below may read a cold-cache `.manifest` from the backend
        // to answer this, so the headers a gated backend requires have to be injected
        // here as they are for every other backend-touching method.
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.is_versioned(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};
    use tonic::metadata::{MetadataMap, MetadataValue};

    use super::*;

    /// What the inner layer received from `is_versioned`.
    #[derive(Default)]
    struct Seen {
        metadata: Option<MetadataMap>,
        additional_headers: Option<Vec<(String, String)>>,
    }

    /// Inner versioning mock that captures the request it receives.
    #[derive(Clone, Default)]
    struct CapturingInner {
        seen: Arc<Mutex<Seen>>,
    }

    #[async_trait]
    impl Versioning for CapturingInner {
        fn inner(&self) -> Option<&dyn Versioning> {
            None
        }

        async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
            let mut seen = self.seen.lock().expect("seen");
            seen.metadata = Some(request.metadata().clone());
            seen.additional_headers = request
                .extensions()
                .get::<AdditionalHeaders>()
                .map(|headers| headers.clone().request_headers(RequestType::Api).collect());
            Ok(IsVersionedResponse { is_versioned: true })
        }
    }

    fn manager(inner: CapturingInner) -> AdditionalHeaderManager<CapturingInner> {
        AdditionalHeaderManager {
            inner,
            cache_control: None,
            cache_control_api: Some("no-cache, no-store".to_string()),
            tracing_header_names: vec!["x-request-id".to_string(), "x-trace-bin".to_string()],
            env_headers: vec![("x-deployment".to_string(), "prod".to_string())],
        }
    }

    fn request() -> Request<IsVersionedRequest> {
        Request::new(IsVersionedRequest {
            resource_address: "https://bucket.example/object".to_string(),
        })
    }

    /// The caller's tracing headers are parsed from its request metadata into the
    /// `AdditionalHeaders` extension, alongside the env and cache-control headers;
    /// the metadata itself reaches the inner layer unchanged.
    #[tokio::test]
    async fn is_versioned_injects_caller_tracing_headers_into_extensions() {
        let inner = CapturingInner::default();
        let manager = manager(inner.clone());
        let mut request = request();
        request
            .metadata_mut()
            .insert("x-request-id", MetadataValue::from_static("abc"));
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Bearer t"));

        let response = manager.is_versioned(request).await.expect("is_versioned");
        assert!(response.is_versioned);

        let seen = inner.seen.lock().expect("seen");
        assert_eq!(
            seen.additional_headers,
            Some(vec![
                ("x-request-id".to_string(), "abc".to_string()),
                ("x-deployment".to_string(), "prod".to_string()),
                ("cache-control".to_string(), "no-cache, no-store".to_string()),
            ])
        );
        let metadata = seen.metadata.as_ref().expect("inner should be called");
        assert_eq!(
            metadata.get("x-request-id").and_then(|value| value.to_str().ok()),
            Some("abc")
        );
        assert_eq!(
            metadata.get("authorization").and_then(|value| value.to_str().ok()),
            Some("Bearer t")
        );
    }

    /// A tracing header that is not valid UTF-8 is rejected before the inner layer
    /// is called, as on every other method this layer decorates.
    #[tokio::test]
    async fn is_versioned_rejects_non_utf8_tracing_header() {
        let inner = CapturingInner::default();
        let manager = manager(inner.clone());
        let mut request = request();
        request
            .metadata_mut()
            .insert_bin("x-trace-bin", MetadataValue::from_bytes(&[0xff, 0xfe]));

        let err = manager
            .is_versioned(request)
            .await
            .expect_err("a non-UTF-8 tracing header must be rejected");
        assert_eq!(err.tonic_status().code(), tonic::Code::InvalidArgument);
        assert!(
            inner.seen.lock().expect("seen").metadata.is_none(),
            "inner must not be called"
        );
    }
}
