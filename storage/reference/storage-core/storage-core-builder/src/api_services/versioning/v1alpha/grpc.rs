// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use storage_core_util::log_if_slow;

use crate::api_services::error::log_grpc_error;
use crate::api_services::slow_stream::SlowGuardedStream;
use crate::traits::versioning::v1alpha::{
    versioning_service_server::{VersioningService, VersioningServiceServer},
    EnumerateVersionsRequest, EnumerateVersionsResponse, IsVersionedRequest, IsVersionedResponse,
    Versioning as VersioningTrait, FILE_DESCRIPTOR_SET,
};
use crate::traits::StreamingOptions;
use crate::{ingress::grpc::ToGrpcService, traits::async_trait};
use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::instrument;
use tracing_futures::Instrument;

use super::super::super::REQUEST_ID;
use super::Versioning;

impl<L> ToGrpcService for Versioning<L>
where
    L: VersioningTrait,
{
    type Service = VersioningServiceServer<Versioning<L>>;

    fn service(&self) -> Self::Service {
        VersioningServiceServer::new(self.clone())
    }

    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

#[async_trait]
impl<T: VersioningTrait> VersioningService for Versioning<T> {
    type EnumerateVersionsStream =
        std::pin::Pin<Box<dyn Stream<Item = Result<EnumerateVersionsResponse, Status>> + Send + 'static>>;
    #[instrument(name = "grpc::enumerate_versions", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
    ) -> Result<Response<Self::EnumerateVersionsStream>, Status> {
        let context = request.get_ref().resource_address.clone();
        let guard = log_if_slow!(guard: "grpc::enumerate_versions").with_context(context.as_str());
        let stream = self
            .versioning
            .enumerate_versions(request.into(), StreamingOptions::Streamed, None)
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map(move |r| match r {
                Ok(r) => Ok(r.response),
                Err(e) => Err(log_grpc_error(e.as_ref(), &context)),
            })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    #[instrument(name = "grpc::is_versioned", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn is_versioned(
        &self,
        request: Request<IsVersionedRequest>,
    ) -> Result<Response<IsVersionedResponse>, Status> {
        let context = request.get_ref().resource_address.clone();
        let _guard = log_if_slow!(guard: "grpc::is_versioned").with_context(context.as_str());
        Ok(Response::new(
            self.versioning
                .is_versioned(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::traits::error::Result as StorageResult;

    /// Stands in for anything a tower layer or interceptor inserts into the tonic
    /// request extensions (an authenticated principal, connection info, ...).
    #[derive(Clone, Debug, PartialEq)]
    struct Marker(&'static str);

    /// What the trait implementation received.
    #[derive(Default)]
    struct Seen {
        resource_address: Option<String>,
        request_id: Option<String>,
        marker: Option<Marker>,
    }

    #[derive(Clone, Default)]
    struct CapturingVersioning {
        seen: Arc<Mutex<Seen>>,
    }

    #[async_trait]
    impl VersioningTrait for CapturingVersioning {
        fn inner(&self) -> Option<&dyn VersioningTrait> {
            None
        }

        async fn is_versioned(
            &self,
            request: crate::traits::Request<IsVersionedRequest>,
        ) -> StorageResult<IsVersionedResponse> {
            let mut seen = self.seen.lock().unwrap();
            seen.resource_address = Some(request.get_ref().resource_address.clone());
            seen.request_id = request
                .metadata()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            seen.marker = request.extensions().get::<Marker>().cloned();
            Ok(IsVersionedResponse { is_versioned: true })
        }
    }

    /// The handler hands the whole tonic request to the trait: the message, the
    /// metadata and the extensions.
    #[tokio::test]
    async fn grpc_is_versioned_forwards_metadata_and_extensions() {
        let backend = CapturingVersioning::default();
        let seen = backend.seen.clone();
        let service = Versioning::new(backend);

        let mut request = Request::new(IsVersionedRequest {
            resource_address: "https://bucket.example/object".into(),
        });
        request.metadata_mut().insert("x-request-id", "abc".parse().unwrap());
        request.extensions_mut().insert(Marker("interceptor"));

        let response = VersioningService::is_versioned(&service, request).await.unwrap();
        assert!(response.into_inner().is_versioned);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.resource_address.as_deref(), Some("https://bucket.example/object"));
        assert_eq!(seen.request_id.as_deref(), Some("abc"));
        assert_eq!(seen.marker, Some(Marker("interceptor")));
    }
}
