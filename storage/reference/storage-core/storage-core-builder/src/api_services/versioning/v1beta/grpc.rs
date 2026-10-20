// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use storage_core_util::log_if_slow;

use crate::api_services::error::log_grpc_error;
use crate::api_services::slow_stream::SlowGuardedStream;
use crate::traits::versioning::v1beta::{
    versioning_service_server::{VersioningService, VersioningServiceServer},
    EnumerateVersionsRequest, EnumerateVersionsResponse, Versioning as VersioningTrait, FILE_DESCRIPTOR_SET,
};
use crate::traits::StreamingOptions;
use crate::{ingress::grpc::ToGrpcService, traits::async_trait};
use futures::{Stream, StreamExt};
use tonic::{Response, Status};
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
        request: tonic::Request<EnumerateVersionsRequest>,
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
}
