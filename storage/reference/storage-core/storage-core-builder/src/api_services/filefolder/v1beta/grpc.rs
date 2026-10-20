// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::instrument;
use tracing_futures::Instrument;

use storage_core_util::log_if_slow;

use crate::{
    api_services::error::log_grpc_error,
    api_services::slow_stream::SlowGuardedStream,
    ingress::grpc::ToGrpcService,
    traits::{
        async_trait,
        filefolder::v1beta::{
            file_folder_service_server::{FileFolderService, FileFolderServiceServer},
            DeleteFolderRequest, DeleteFolderResponse, FileFolder as FileFolderTrait, ListRequest, ListResponse,
            ListStatRequest, ListStatResponse, FILE_DESCRIPTOR_SET,
        },
        StreamingOptions,
    },
};

use super::super::super::REQUEST_ID;
use super::FileFolder;

impl<L> ToGrpcService for FileFolder<L>
where
    L: FileFolderTrait,
{
    type Service = FileFolderServiceServer<FileFolder<L>>;

    fn service(&self) -> Self::Service {
        FileFolderServiceServer::new(self.clone())
    }

    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

#[async_trait]
impl<T: FileFolderTrait> FileFolderService for FileFolder<T> {
    type ListStream = std::pin::Pin<Box<dyn Stream<Item = Result<ListResponse, Status>> + Send + 'static>>;

    #[instrument(name = "grpc::list", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn list(&self, request: Request<ListRequest>) -> Result<Response<Self::ListStream>, Status> {
        let context = request.get_ref().folder.as_ref().unwrap().uri.clone();
        let guard = log_if_slow!(guard: "grpc::list").with_context(context.as_str());
        let stream = self
            .file_folder
            .list(request.into(), StreamingOptions::Streamed)
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map(move |r| match r {
                Ok(r) => Ok(r.response),
                Err(e) => Err(log_grpc_error(e.as_ref(), &context)),
            })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    type ListStatStream = std::pin::Pin<Box<dyn Stream<Item = Result<ListStatResponse, Status>> + Send + 'static>>;
    #[instrument(name = "grpc::list_stat", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn list_stat(&self, request: Request<ListStatRequest>) -> Result<Response<Self::ListStatStream>, Status> {
        let context = request.get_ref().folder.as_ref().unwrap().uri.clone();
        let guard = log_if_slow!(guard: "grpc::list_stat").with_context(context.as_str());
        let stream = self
            .file_folder
            .list_stat(request.into(), StreamingOptions::Streamed)
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map(move |r| match r {
                Ok(r) => Ok(r.response),
                Err(e) => Err(log_grpc_error(e.as_ref(), &context)),
            })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    #[instrument(name = "grpc::delete_folder", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn delete_folder(
        &self,
        request: Request<DeleteFolderRequest>,
    ) -> Result<Response<DeleteFolderResponse>, Status> {
        let context = request.get_ref().folder.as_ref().unwrap().uri.clone();
        let _slow = log_if_slow!(guard: "grpc::delete_folder").with_context(context.as_str());
        Ok(Response::new(
            self.file_folder
                .delete_folder(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }
}
