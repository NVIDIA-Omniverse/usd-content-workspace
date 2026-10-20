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

use futures::future::BoxFuture;
use snafu::IntoError;
use storage_core::traits::{fileobject::v1alpha::*, BoxStream, Bytes, Request, Result, StreamingOptions};

use crate::backend::az_blob_name_rejecter::{error::error, AzureBlobNameRejecterService};

impl<S> AzureBlobNameRejecterService<S> {
    fn invoke_with_resource_identity_v1alpha<'a, T, R>(
        &self,
        request: Request<T>,
        parameter: impl FnOnce(&T) -> &Option<ResourceIdentity>,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        use error::*;

        self.invoke_with_parameter(
            request,
            |message| {
                parameter(message)
                    .as_ref()
                    .map(|identity| identity.encoded_identity.as_str())
                    .ok_or_else(|| ResourceIdentityMissing.build())
            },
            |source| ResourceIdentityInvalid.into_error(source),
            invoke,
        )
    }
}

impl<S: FileObject> FileObject for AzureBlobNameRejecterService<S> {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    fn stat<'a: 'b, 'b>(&'a self, request: Request<StatRequest>) -> BoxFuture<'b, Result<StatResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.stat(request),
        )
    }

    fn read<'a: 'b, 'b>(&'a self, request: Request<ReadRequest>) -> BoxFuture<'b, Result<BoxStream<ReadResponse>>> {
        self.invoke_with_resource_identity_v1alpha(
            request,
            |message| &message.resource_identity,
            |request| self.inner.read(request),
        )
    }

    fn read_from_address<'a: 'b, 'b>(
        &'a self,
        request: Request<ReadFromAddressRequest>,
    ) -> BoxFuture<'b, Result<BoxStream<ReadFromAddressResponse>>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.read_from_address(request),
        )
    }

    fn write<'a: 'b, 'b>(
        &'a self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<'static, Result<Bytes>>,
    ) -> BoxFuture<'b, Result<BoxStream<WriteResponse>>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.write(request, data_future),
        )
    }

    fn delete<'a: 'b, 'b>(&'a self, request: Request<DeleteRequest>) -> BoxFuture<'b, Result<DeleteResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.delete(request),
        )
    }

    fn abort_multipart_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> BoxFuture<'b, Result<AbortMultipartUploadResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.abort_multipart_upload(request),
        )
    }

    fn complete_multipart_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> BoxFuture<'b, Result<CompleteMultipartUploadResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.complete_multipart_upload(request),
        )
    }

    fn upload_part<'a: 'b, 'b>(
        &'a self,
        request: Request<UploadPartRequest>,
    ) -> BoxFuture<'b, Result<UploadPartResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.upload_part(request),
        )
    }

    fn complete_redirect_upload<'a: 'b, 'b>(
        &'a self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> BoxFuture<'b, Result<CompleteRedirectUploadResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.complete_redirect_upload(request),
        )
    }

    fn fetch_write_type_info<'a: 'b, 'b>(
        &'a self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> BoxFuture<'b, Result<FetchWriteTypeInfoResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.destination_resource_address,
            |request| self.inner.fetch_write_type_info(request),
        )
    }

    fn enumerate<'a: 'b, 'b>(
        &'a self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<EnumerateResponseWithContinueToken>>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.enumerate(request, streaming),
        )
    }

    fn copy<'a: 'b, 'b>(&'a self, request: Request<CopyRequest>) -> BoxFuture<'b, Result<CopyResponse>> {
        self.invoke_with_resource_identity_v1alpha(
            request,
            |message| &message.source_resource_identity,
            |request| {
                self.invoke_with_resource_address(
                    request,
                    |message| &message.destination_resource_address,
                    |request| self.inner.copy(request),
                )
            },
        )
    }

    fn move_object<'a: 'b, 'b>(&'a self, request: Request<MoveRequest>) -> BoxFuture<'b, Result<MoveResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.source_resource_address,
            |request| {
                self.invoke_with_resource_address(
                    request,
                    |message| &message.destination_resource_address,
                    |request| self.inner.move_object(request),
                )
            },
        )
    }

    fn get_optimistic_locking_support<'a: 'b, 'b>(
        &'a self,
        request: Request<GetOptimisticLockingSupportRequest>,
    ) -> BoxFuture<'b, Result<GetOptimisticLockingSupportResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.get_optimistic_locking_support(request),
        )
    }

    fn get_native_metadata_support<'a: 'b, 'b>(
        &'a self,
        request: Request<GetNativeMetadataSupportRequest>,
    ) -> BoxFuture<'b, Result<GetNativeMetadataSupportResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.get_native_metadata_support(request),
        )
    }

    fn is_read_only<'a: 'b, 'b>(
        &'a self,
        request: Request<IsReadOnlyRequest>,
    ) -> BoxFuture<'b, Result<IsReadOnlyResponse>> {
        self.invoke_with_resource_address(
            request,
            |message| &message.resource_address,
            |request| self.inner.is_read_only(request),
        )
    }
}
