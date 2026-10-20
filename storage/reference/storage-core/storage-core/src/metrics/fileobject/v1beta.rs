// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    error::Result,
    fileobject::v1beta::{
        read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
        AbortMultipartUploadResponse, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse,
        CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, DeleteRequest, DeleteResponse, EnumerateRequest,
        EnumerateResponseWithContinueToken, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject,
        ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest, StatResponse,
        UploadPartRequest, UploadPartResponse, WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
};

use super::{ReadResponseExt, ReadStream, UploadPreferenceExt, WriteStream};
use crate::metrics::{
    utils::{EntryCount, EntryCountStream, TimedStream},
    OperationMetrics, SdkMetrics,
};

#[async_trait]
impl<S> FileObject for OperationMetrics<S>
where
    S: FileObject + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let _timer = self.stat_metrics_v1beta.latency.start();

        record_operation!(self.stat_metrics_v1beta, self.inner.stat(request).await)
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let timer = self.read_metrics_v1beta.latency.start();

        match self.inner.read(request).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.read_metrics_v1beta.clone());
                Ok(Box::pin(ReadStream::new(
                    timed,
                    self.read_redirects_v1beta.clone(),
                    self.read_chunk_size_hist_v1beta.clone(),
                    self.read_data_size_hist_v1beta.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.read_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let timer = self.read_from_address_metrics_v1beta.latency.start();

        match self.inner.read_from_address(request).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.read_from_address_metrics_v1beta.clone());
                Ok(Box::pin(ReadStream::new(
                    timed,
                    self.read_from_address_redirects_v1beta.clone(),
                    self.read_from_address_chunk_size_hist_v1beta.clone(),
                    self.read_from_address_data_size_hist_v1beta.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.read_from_address_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let timer = self.write_metrics_v1beta.latency.start();

        let data_size = request.get_ref().data_object_size;
        match self.inner.write(request, data_future).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.write_metrics_v1beta.clone());
                Ok(Box::pin(WriteStream::new(
                    timed,
                    self.write_upload_method_v1beta_body.clone(),
                    self.write_upload_method_v1beta_redirect.clone(),
                    self.write_upload_method_v1beta_multipart.clone(),
                    self.write_data_size_hist_v1beta.clone(),
                    data_size,
                )))
            },
            Err(e) => {
                increment_error!(self.write_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let _timer = self.delete_metrics_v1beta.latency.start();

        record_operation!(self.delete_metrics_v1beta, self.inner.delete(request).await)
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let _timer = self.abort_multipart_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.abort_multipart_upload_metrics_v1beta,
            self.inner.abort_multipart_upload(request).await
        )
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let _timer = self.complete_multipart_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.complete_multipart_upload_metrics_v1beta,
            self.inner.complete_multipart_upload(request).await
        )
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let _timer = self.upload_part_metrics_v1beta.latency.start();

        record_operation!(self.upload_part_metrics_v1beta, self.inner.upload_part(request).await)
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let _timer = self.complete_redirect_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.complete_redirect_upload_metrics_v1beta,
            self.inner.complete_redirect_upload(request).await
        )
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let _timer = self.fetch_write_type_info_metrics_v1beta.latency.start();

        record_operation!(
            self.fetch_write_type_info_metrics_v1beta,
            self.inner.fetch_write_type_info(request).await
        )
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let timer = self.enumerate_metrics_v1beta.latency.start();

        match self.inner.enumerate(request, streaming).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.enumerate_metrics_v1beta.clone());
                Ok(Box::pin(EntryCountStream::new(
                    timed,
                    self.enumerate_item_count_v1beta.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.enumerate_metrics_v1beta, e);
                Err(e)
            },
        }
    }
}

#[async_trait]
impl<S> FileObject for SdkMetrics<S>
where
    S: FileObject + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let _timer = self.stat_metrics_v1beta.latency.start();

        record_operation!(self.stat_metrics_v1beta, self.inner.stat(request).await)
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let timer = self.read_metrics_v1beta.latency.start();

        match self.inner.read(request).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.read_metrics_v1beta.clone(),
            ))),
            Err(e) => {
                increment_error!(self.read_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let timer = self.read_from_address_metrics_v1beta.latency.start();

        match self.inner.read_from_address(request).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.read_from_address_metrics_v1beta.clone(),
            ))),
            Err(e) => {
                increment_error!(self.read_from_address_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let timer = self.write_metrics_v1beta.latency.start();

        match self.inner.write(request, data_future).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.write_metrics_v1beta.clone(),
            ))),
            Err(e) => {
                increment_error!(self.write_metrics_v1beta, e);
                Err(e)
            },
        }
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let _timer = self.delete_metrics_v1beta.latency.start();

        record_operation!(self.delete_metrics_v1beta, self.inner.delete(request).await)
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let _timer = self.abort_multipart_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.abort_multipart_upload_metrics_v1beta,
            self.inner.abort_multipart_upload(request).await
        )
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let _timer = self.complete_multipart_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.complete_multipart_upload_metrics_v1beta,
            self.inner.complete_multipart_upload(request).await
        )
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let _timer = self.upload_part_metrics_v1beta.latency.start();

        record_operation!(self.upload_part_metrics_v1beta, self.inner.upload_part(request).await)
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let _timer = self.complete_redirect_upload_metrics_v1beta.latency.start();

        record_operation!(
            self.complete_redirect_upload_metrics_v1beta,
            self.inner.complete_redirect_upload(request).await
        )
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let _timer = self.fetch_write_type_info_metrics_v1beta.latency.start();

        record_operation!(
            self.fetch_write_type_info_metrics_v1beta,
            self.inner.fetch_write_type_info(request).await
        )
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let timer = self.enumerate_metrics_v1beta.latency.start();

        match self.inner.enumerate(request, streaming).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.enumerate_metrics_v1beta.clone(),
            ))),
            Err(e) => {
                increment_error!(self.enumerate_metrics_v1beta, e);
                Err(e)
            },
        }
    }
}

impl ReadResponseExt for ReadResponse {
    fn extract_data_size(&self) -> Option<u64> {
        if let Some(read_response::ReplyType::Metadata(metadata)) = &self.reply_type {
            if let Some(data_object_size) = metadata.data_object_size {
                return Some(data_object_size);
            }
        }
        None
    }

    fn extract_chunk_size(&self) -> Option<u64> {
        if let Some(read_response::ReplyType::Chunk(chunk)) = &self.reply_type {
            return Some(chunk.chunk.len() as u64);
        }
        None
    }

    fn is_redirect(&self) -> bool {
        if let Some(read_response::ReplyType::Redirect(_)) = &self.reply_type {
            return true;
        }
        false
    }
}

impl ReadResponseExt for ReadFromAddressResponse {
    fn extract_data_size(&self) -> Option<u64> {
        if let Some(read_from_address_response::ReplyType::ResourceInfo(resource_info)) = &self.reply_type {
            if let Some(metadata) = resource_info.metadata {
                if let Some(data_object_size) = metadata.data_object_size {
                    return Some(data_object_size);
                }
            }
        }
        None
    }

    fn extract_chunk_size(&self) -> Option<u64> {
        if let Some(read_from_address_response::ReplyType::Chunk(chunk)) = &self.reply_type {
            return Some(chunk.chunk.len() as u64);
        }
        None
    }

    fn is_redirect(&self) -> bool {
        if let Some(read_from_address_response::ReplyType::Redirect(_)) = &self.reply_type {
            return true;
        }
        false
    }
}

impl EntryCount for EnumerateResponseWithContinueToken {
    fn entry_count(&self) -> u64 {
        self.response.items.len() as u64
    }
}

impl UploadPreferenceExt for WriteResponse {
    fn upload_preference(&self) -> &'static str {
        match &self.write_response_type {
            Some(write_response::WriteResponseType::WriteChunksAccepted(_)) => "body",
            Some(write_response::WriteResponseType::ResourceInfo(_)) => "body",
            Some(write_response::WriteResponseType::WriteRedirect(_)) => "redirect",
            Some(write_response::WriteResponseType::MultipartUpload(_)) => "multipart",
            None => "unspecified",
        }
    }
}
