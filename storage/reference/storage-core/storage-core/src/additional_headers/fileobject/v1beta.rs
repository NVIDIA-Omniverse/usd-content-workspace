// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
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

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> FileObject for AdditionalHeaderManager<S>
where
    S: FileObject + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, mut request: Request<StatRequest>) -> Result<StatResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.stat(request).await
    }

    async fn read(&self, mut request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.read(request).await
    }

    async fn read_from_address(
        &self,
        mut request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.read_from_address(request).await
    }

    async fn write(
        &self,
        mut request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.write(request, data_future).await
    }

    async fn delete(&self, mut request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.delete(request).await
    }

    async fn abort_multipart_upload(
        &self,
        mut request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.abort_multipart_upload(request).await
    }

    async fn complete_multipart_upload(
        &self,
        mut request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.complete_multipart_upload(request).await
    }

    async fn upload_part(&self, mut request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.upload_part(request).await
    }

    async fn complete_redirect_upload(
        &self,
        mut request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.complete_redirect_upload(request).await
    }

    async fn fetch_write_type_info(
        &self,
        mut request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.fetch_write_type_info(request).await
    }

    async fn enumerate(
        &self,
        mut request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.enumerate(request, streaming).await
    }
}
