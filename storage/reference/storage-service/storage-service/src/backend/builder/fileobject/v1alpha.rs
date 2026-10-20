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

use std::sync::Arc;

use bytes::Bytes;
use futures::future::BoxFuture;
use storage_core::traits::{fileobject::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

#[derive(Clone)]
pub struct AnyFileObject {
    inner: Arc<dyn FileObject>,
}

impl AnyFileObject {
    pub fn new(inner: Arc<dyn FileObject>) -> Self {
        Self { inner }
    }
}

impl FileObject for AnyFileObject {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    fn stat<'a: 'b, 'b>(&'a self, request: Request<StatRequest>) -> BoxFuture<'b, Result<StatResponse>> {
        self.inner.stat(request)
    }

    fn is_read_only<'a: 'b, 'b>(
        &'a self,
        request: Request<IsReadOnlyRequest>,
    ) -> BoxFuture<'b, Result<IsReadOnlyResponse>> {
        self.inner.is_read_only(request)
    }

    fn read<'a: 'b, 'b>(&'a self, request: Request<ReadRequest>) -> BoxFuture<'b, Result<BoxStream<ReadResponse>>> {
        self.inner.read(request)
    }

    fn read_from_address<'a: 'b, 'b>(
        &'a self,
        request: Request<ReadFromAddressRequest>,
    ) -> BoxFuture<'b, Result<BoxStream<ReadFromAddressResponse>>> {
        self.inner.read_from_address(request)
    }

    fn write<'a: 'b, 'b>(
        &'a self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<'static, Result<Bytes>>,
    ) -> BoxFuture<'b, Result<BoxStream<WriteResponse>>> {
        self.inner.write(request, data_future)
    }

    fn delete<'a: 'b, 'b>(&'a self, request: Request<DeleteRequest>) -> BoxFuture<'b, Result<DeleteResponse>> {
        self.inner.delete(request)
    }

    fn abort_multipart_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> BoxFuture<'b, Result<AbortMultipartUploadResponse>> {
        self.inner.abort_multipart_upload(request)
    }

    fn complete_multipart_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> BoxFuture<'b, Result<CompleteMultipartUploadResponse>> {
        self.inner.complete_multipart_upload(request)
    }

    fn upload_part<'a: 'b, 'b>(
        &'a self,
        request: Request<UploadPartRequest>,
    ) -> BoxFuture<'b, Result<UploadPartResponse>> {
        self.inner.upload_part(request)
    }

    fn complete_redirect_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> BoxFuture<'b, Result<CompleteRedirectUploadResponse>> {
        self.inner.complete_redirect_upload(request)
    }

    fn fetch_write_type_info<'a: 'b, 'b>(
        &'a self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> BoxFuture<'b, Result<FetchWriteTypeInfoResponse>> {
        self.inner.fetch_write_type_info(request)
    }

    fn enumerate<'a: 'b, 'b>(
        &'a self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<EnumerateResponseWithContinueToken>>> {
        self.inner.enumerate(request, streaming)
    }

    fn copy<'a: 'b, 'b>(&'a self, request: Request<CopyRequest>) -> BoxFuture<'b, Result<CopyResponse>> {
        self.inner.copy(request)
    }

    fn move_object<'a: 'b, 'b>(&'a self, request: Request<MoveRequest>) -> BoxFuture<'b, Result<MoveResponse>> {
        self.inner.move_object(request)
    }
}
