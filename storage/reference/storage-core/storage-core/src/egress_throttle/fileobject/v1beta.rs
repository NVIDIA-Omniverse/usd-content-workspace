// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    error::Result,
    fileobject::v1beta::{
        AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, DeleteRequest,
        DeleteResponse, EnumerateRequest, EnumerateResponseWithContinueToken, FetchWriteTypeInfoRequest,
        FetchWriteTypeInfoResponse, FileObject, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest,
        ReadResponse, StatRequest, StatResponse, UploadPartRequest, UploadPartResponse, WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> FileObject for EgressThrottle<S>
where
    S: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.stat(request).await
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.read(request).await
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.read_from_address(request).await
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.write(request, data_future).await
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.delete(request).await
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.abort_multipart_upload(request).await
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.complete_multipart_upload(request).await
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.upload_part(request).await
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.complete_redirect_upload(request).await
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.fetch_write_type_info(request).await
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.enumerate(request, streaming).await
    }
}
