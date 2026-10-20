// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use bytes::Bytes;
use futures::StreamExt;
use storage_core::traits::{
    async_trait, fileobject::v1beta::*, BoxFuture, BoxStream, Request, Result, StreamingOptions,
};

use crate::backend::trace_error::{log_storage_error, TraceErrorService};

#[async_trait]
impl<S: FileObject> FileObject for TraceErrorService<S> {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        self.inner
            .stat(request)
            .await
            .inspect_err(|error| log_storage_error(error, "stat"))
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        self.inner
            .read(request)
            .await
            .inspect_err(|error| log_storage_error(error, "read"))
            .map(|stream| {
                Box::pin(stream.then(async |item_or_error| {
                    item_or_error.inspect_err(|error| log_storage_error(error, "read_fetch"))
                })) as BoxStream<ReadResponse>
            })
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        self.inner
            .read_from_address(request)
            .await
            .inspect_err(|error| log_storage_error(error, "read_from_address"))
            .map(|stream| {
                Box::pin(stream.then(async |item_or_error| {
                    item_or_error.inspect_err(|error| log_storage_error(error, "read_from_address_fetch"))
                })) as BoxStream<ReadFromAddressResponse>
            })
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        self.inner
            .write(request, data_future)
            .await
            .inspect_err(|error| log_storage_error(error, "write"))
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        self.inner
            .delete(request)
            .await
            .inspect_err(|error| log_storage_error(error, "delete"))
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        self.inner
            .abort_multipart_upload(request)
            .await
            .inspect_err(|error| log_storage_error(error, "abort_multipart_upload"))
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        self.inner
            .complete_multipart_upload(request)
            .await
            .inspect_err(|error| log_storage_error(error, "complete_multipart_upload"))
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        self.inner
            .upload_part(request)
            .await
            .inspect_err(|error| log_storage_error(error, "upload_part"))
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        self.inner
            .complete_redirect_upload(request)
            .await
            .inspect_err(|error| log_storage_error(error, "complete_redirect_upload"))
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        self.inner
            .fetch_write_type_info(request)
            .await
            .inspect_err(|error| log_storage_error(error, "fetch_write_type_info"))
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        self.inner
            .enumerate(request, streaming)
            .await
            .inspect_err(|error| log_storage_error(error, "enumerate"))
            .map(|stream| {
                Box::pin(stream.then(async |item_or_error| {
                    item_or_error.inspect_err(|error| log_storage_error(error, "enumerate_fetch"))
                })) as BoxStream<EnumerateResponseWithContinueToken>
            })
    }
}
