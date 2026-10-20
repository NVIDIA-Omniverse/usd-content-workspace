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
use futures::{future::BoxFuture, StreamExt};
use storage_core::traits::{async_trait, fileobject::v1beta::*, BoxStream, Request, Result, StreamingOptions};

use crate::backend::created_timestamp::{
    delete_created_timestamp, record_created_timestamp, resource_address_location, CreatedTimestampService,
};

#[async_trait]
impl<S: FileObject> FileObject for CreatedTimestampService<S> {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    fn stat<'a: 'b, 'b>(&'a self, request: Request<StatRequest>) -> BoxFuture<'b, Result<StatResponse>> {
        self.inner.stat(request)
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

    // A direct write completes with a `ResourceInfo`; record the creation time then (set-once).
    // Chunked/redirect/multipart writes create the object at their separate completion call, which
    // does not carry the destination — recording those is left for a follow-up.
    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<'static, Result<Bytes>>,
    ) -> Result<BoxStream<WriteResponse>> {
        let metadata = self.metadata.clone();
        let destination = request.get_ref().destination_resource_address.clone();
        let stream = self.inner.write(request, data_future).await?;
        Ok(Box::pin(stream.then(move |response| {
            // Only the terminal `ResourceInfo` creates the object; clone the recording inputs
            // lazily so a chunked write does not clone them for every intermediate response.
            let recording = match &response {
                Ok(response)
                    if matches!(
                        response.write_response_type,
                        Some(write_response::WriteResponseType::ResourceInfo(_))
                    ) =>
                {
                    Some((metadata.clone(), destination.clone()))
                },
                _ => None,
            };
            async move {
                let response = response?;
                if let Some((metadata, destination)) = recording {
                    record_created_timestamp(&metadata, resource_address_location(destination)).await;
                }
                Ok(response)
            }
        })) as BoxStream<WriteResponse>)
    }

    // Delete the object, then best-effort remove its `created_timestamp`. Cleanup runs only after
    // the underlying delete succeeds, so a failed delete leaves the metadata untouched; a metadata
    // failure never fails the delete.
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let address = request.get_ref().resource_address.clone();
        let response = self.inner.delete(request).await?;
        delete_created_timestamp(&self.metadata, resource_address_location(address)).await;
        Ok(response)
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
}
