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

use futures::{future::BoxFuture, FutureExt};
use storage_core::traits::{fileobject::v1alpha::*, BoxStream, Bytes, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::backend::router::{error::Error, Dummy, Match, Service, ServiceRoute};

impl<A, SA: Match, B> Service<ServiceRoute<A, SA>, B> {
    pub(super) fn match_resource_identity_v1alpha(&self, identity: Option<&ResourceIdentity>) -> bool {
        if let Some(identity) = identity {
            self.head.selector.match_resource_identity(&identity.encoded_identity)
        } else {
            false
        }
    }
}

impl<A: FileObject, SA: Match, B: FileObject> FileObject for Service<ServiceRoute<A, SA>, B> {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    fn stat<'a: 'r, 'r>(&'a self, request: Request<StatRequest>) -> BoxFuture<'a, Result<StatResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().resource_address)
        {
            self.head.service.stat(request)
        } else {
            self.tail.stat(request)
        }
    }

    fn is_read_only<'a: 'r, 'r>(
        &'a self,
        request: Request<IsReadOnlyRequest>,
    ) -> BoxFuture<'a, Result<IsReadOnlyResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().resource_address)
        {
            self.head.service.is_read_only(request)
        } else {
            self.tail.is_read_only(request)
        }
    }

    fn read<'a: 'r, 'r>(&'a self, request: Request<ReadRequest>) -> BoxFuture<'a, Result<BoxStream<ReadResponse>>> {
        if self.match_resource_identity_v1alpha(request.get_ref().resource_identity.as_ref()) {
            self.head.service.read(request)
        } else {
            self.tail.read(request)
        }
    }

    fn read_from_address<'a: 'r, 'r>(
        &'a self,
        request: Request<ReadFromAddressRequest>,
    ) -> BoxFuture<'r, Result<BoxStream<ReadFromAddressResponse>>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().resource_address)
        {
            self.head.service.read_from_address(request)
        } else {
            self.tail.read_from_address(request)
        }
    }

    fn write<'a: 'r, 'r>(
        &'a self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<'static, Result<Bytes>>,
    ) -> BoxFuture<'r, Result<BoxStream<WriteResponse>>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.write(request, data_future)
        } else {
            self.tail.write(request, data_future)
        }
    }

    fn delete<'a: 'r, 'r>(&'a self, request: Request<DeleteRequest>) -> BoxFuture<'r, Result<DeleteResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().resource_address)
        {
            self.head.service.delete(request)
        } else {
            self.tail.delete(request)
        }
    }

    fn abort_multipart_upload<'a: 'r, 'r>(
        &'a self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> BoxFuture<'r, Result<AbortMultipartUploadResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.abort_multipart_upload(request)
        } else {
            self.tail.abort_multipart_upload(request)
        }
    }

    fn complete_multipart_upload<'a: 'r, 'r>(
        &'a self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> BoxFuture<'r, Result<CompleteMultipartUploadResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.complete_multipart_upload(request)
        } else {
            self.tail.complete_multipart_upload(request)
        }
    }

    fn upload_part<'a: 'r, 'r>(
        &'a self,
        request: Request<UploadPartRequest>,
    ) -> BoxFuture<'r, Result<UploadPartResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.upload_part(request)
        } else {
            self.tail.upload_part(request)
        }
    }

    fn complete_redirect_upload<'a: 'r, 'r>(
        &'a self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> BoxFuture<'r, Result<CompleteRedirectUploadResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.complete_redirect_upload(request)
        } else {
            self.tail.complete_redirect_upload(request)
        }
    }

    fn fetch_write_type_info<'a: 'r, 'r>(
        &'a self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> BoxFuture<'r, Result<FetchWriteTypeInfoResponse>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            self.head.service.fetch_write_type_info(request)
        } else {
            self.tail.fetch_write_type_info(request)
        }
    }

    fn copy<'a: 'r, 'r>(&'a self, request: Request<CopyRequest>) -> BoxFuture<'r, Result<CopyResponse>> {
        // The copy is served by the backend owning the destination -- walk the chain to it. When the
        // destination is served here the source identity must be too; otherwise the source is on a
        // different backend (or unserved) and the copy would span backends.
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            if self.match_resource_identity_v1alpha(request.get_ref().source_resource_identity.as_ref()) {
                return self.head.service.copy(request);
            }
            return rejected_future(Error::CrossBackendCopy);
        }
        self.tail.copy(request)
    }

    fn move_object<'a: 'r, 'r>(&'a self, request: Request<MoveRequest>) -> BoxFuture<'r, Result<MoveResponse>> {
        // The move is served by the backend owning the destination -- walk the chain to it. When the
        // destination is served here the source address must be too; otherwise it spans backends.
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().destination_resource_address)
        {
            if self
                .head
                .selector
                .match_resource_address(&request.get_ref().source_resource_address)
            {
                return self.head.service.move_object(request);
            }
            return rejected_future(Error::CrossBackendMove);
        }
        self.tail.move_object(request)
    }

    fn enumerate<'a: 'r, 'r>(
        &'a self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'r, Result<BoxStream<EnumerateResponseWithContinueToken>>> {
        if self
            .head
            .selector
            .match_folder_address(&request.get_ref().resource_address)
        {
            self.head.service.enumerate(request, streaming)
        } else {
            self.tail.enumerate(request, streaming)
        }
    }
}

#[async_trait]
impl FileObject for Dummy {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn is_read_only(&self, _request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn read(&self, _request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        Err(Error::UnsupportedIdentity.into())
    }

    async fn read_from_address(
        &self,
        _request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn write(
        &self,
        _request: Request<WriteParameters>,
        _data_future: BoxFuture<'static, Result<Bytes>>,
    ) -> Result<BoxStream<WriteResponse>> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn delete(&self, _request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn abort_multipart_upload(
        &self,
        _request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn complete_multipart_upload(
        &self,
        _request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn upload_part(&self, _request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn complete_redirect_upload(
        &self,
        _request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn fetch_write_type_info(
        &self,
        _request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn copy(&self, _request: Request<CopyRequest>) -> Result<CopyResponse> {
        // Reached only when no backend serves the copy's destination address.
        Err(Error::UnsupportedAddress.into())
    }

    async fn move_object(&self, _request: Request<MoveRequest>) -> Result<MoveResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn enumerate(
        &self,
        _request: Request<EnumerateRequest>,
        _streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        Err(Error::UnsupportedAddress.into())
    }
}

fn rejected_future<'a, T: Send + 'static>(error: Error) -> BoxFuture<'a, Result<T>> {
    futures::future::ready(Err(error.into())).boxed()
}
