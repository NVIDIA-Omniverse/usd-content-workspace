// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use futures::{Stream, StreamExt, TryStreamExt};
use tonic::{Request, Response, Status};
use tracing::instrument;
use tracing_futures::Instrument;

use super::super::utils::collect_body_streamed_future;
use super::FileObject;
use crate::api_services::error::log_grpc_error;
use crate::api_services::slow_stream::SlowGuardedStream;
use crate::api_services::REQUEST_ID;
use crate::traits::error::StorageError;
use crate::traits::fileobject::v1beta::FileObject as FileObjectTrait;
use crate::traits::fileobject::v1beta::{
    file_object_service_server::{FileObjectService, FileObjectServiceServer},
    write_request::WriteRequestType,
    AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
    CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, DeleteRequest,
    DeleteResponse, EnumerateRequest, EnumerateResponse, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse,
    ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest, StatResponse,
    UploadPartRequest, UploadPartResponse, WriteRequest, WriteResponse, FILE_DESCRIPTOR_SET,
};
use crate::traits::StreamingOptions;
use crate::{ingress::grpc::ToGrpcService, traits::async_trait};
use storage_core_util::log_if_slow;

impl<L> ToGrpcService for FileObject<L>
where
    L: FileObjectTrait,
{
    type Service = FileObjectServiceServer<FileObject<L>>;

    fn service(&self) -> Self::Service {
        FileObjectServiceServer::new(self.clone())
    }

    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

#[async_trait]
impl<T: FileObjectTrait> FileObjectService for FileObject<T> {
    type EnumerateStream = std::pin::Pin<Box<dyn Stream<Item = Result<EnumerateResponse, Status>> + Send + 'static>>;
    #[instrument(name = "grpc::enumerate", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn enumerate(&self, request: Request<EnumerateRequest>) -> Result<Response<Self::EnumerateStream>, Status> {
        let context = request.get_ref().resource_address.clone();
        let guard = log_if_slow!(guard: "grpc::enumerate").with_context(context.as_str());
        let stream = self
            .file_object
            .enumerate(request.into(), StreamingOptions::Streamed)
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map(move |r| match r {
                Ok(r) => Ok(r.response),
                Err(e) => Err(log_grpc_error(e.as_ref(), &context)),
            })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    #[instrument(name = "grpc::stat", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn stat(&self, request: tonic::Request<StatRequest>) -> Result<tonic::Response<StatResponse>, tonic::Status> {
        let context = request.get_ref().resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::stat").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .stat(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    type ReadFromAddressStream =
        std::pin::Pin<Box<dyn Stream<Item = Result<ReadFromAddressResponse, Status>> + Send + 'static>>;
    #[instrument(name = "grpc::read_from_address", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<Response<Self::ReadFromAddressStream>, Status> {
        let context = request.get_ref().resource_address.clone();
        let guard = log_if_slow!(guard: "grpc::read_from_address").with_context(context.as_str());
        let stream = self
            .file_object
            .read_from_address(request.into())
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map_err(move |e: StorageError| -> Status { log_grpc_error(e.as_ref(), &context) })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    type ReadStream = std::pin::Pin<Box<dyn Stream<Item = Result<ReadResponse, Status>> + Send>>;
    #[instrument(name = "grpc::read", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn read(&self, request: Request<ReadRequest>) -> Result<Response<Self::ReadStream>, Status> {
        let context = request
            .get_ref()
            .resource_identity
            .as_ref()
            .map_or(String::new(), |ri| ri.encoded_identity.clone());
        let guard = log_if_slow!(guard: "grpc::read").with_context(context.as_str());
        let stream = self
            .file_object
            .read(request.into())
            .await
            .map_err(|e| log_grpc_error(e.as_ref(), &context))?
            .map_err(move |e: StorageError| -> Status { log_grpc_error(e.as_ref(), &context) })
            .in_current_span();
        Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
    }

    #[instrument(name = "grpc::fetch_write_type_info", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<Response<FetchWriteTypeInfoResponse>, Status> {
        let context = request.get_ref().destination_resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::fetch_write_type_info").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .fetch_write_type_info(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    type WriteStream = std::pin::Pin<Box<dyn Stream<Item = Result<WriteResponse, Status>> + Send + 'static>>;
    #[instrument(name = "grpc::write", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn write(
        &self,
        request: Request<tonic::Streaming<WriteRequest>>,
    ) -> Result<Response<Self::WriteStream>, Status> {
        let _slow = log_if_slow!(guard: "grpc::write");
        let (metadata, extensions, mut write_request) = request.into_parts();

        let params = match write_request.next().await {
            Some(Ok(WriteRequest { write_request_type: r })) if r.is_some() => match r.unwrap() {
                WriteRequestType::Params(p) => p,
                _ => {
                    return Err(Status::invalid_argument(
                        "First message must be WriteRequestType::Params",
                    ));
                },
            },
            _ => {
                return Err(Status::invalid_argument("Failed to receive write request"));
            },
        };

        let context = params.destination_resource_address.clone();
        let data_object_size = params.data_object_size;

        Ok(Response::new(Box::pin(
            self.file_object
                .write(
                    tonic::Request::from_parts(metadata, extensions, params).into(),
                    collect_body_streamed_future(
                        write_request,
                        |message| match message.write_request_type {
                            Some(WriteRequestType::Chunk(c)) => Some(c.chunk),
                            _ => None,
                        },
                        data_object_size as usize,
                    ),
                )
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?
                .map_err(move |e: StorageError| -> Status { log_grpc_error(e.as_ref(), &context) })
                .in_current_span(),
        )))
    }

    #[instrument(name = "grpc::upload_part", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<Response<UploadPartResponse>, Status> {
        let context = request.get_ref().destination_resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::upload_part").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .upload_part(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::complete_multipart_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<Response<CompleteMultipartUploadResponse>, Status> {
        let context = request.get_ref().destination_resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::complete_multipart_upload").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .complete_multipart_upload(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::abort_multipart_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<Response<AbortMultipartUploadResponse>, Status> {
        let context = request.get_ref().destination_resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::abort_multipart_upload").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .abort_multipart_upload(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::complete_redirect_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<Response<CompleteRedirectUploadResponse>, Status> {
        let context = request.get_ref().destination_resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::complete_redirect_upload").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .complete_redirect_upload(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }

    #[instrument(name = "grpc::delete", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<Response<DeleteResponse>, Status> {
        let context = request.get_ref().resource_address.clone();
        let _slow = log_if_slow!(guard: "grpc::delete").with_context(context.as_str());
        Ok(Response::new(
            self.file_object
                .delete(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), &context))?,
        ))
    }
}
