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

use storage_core::traits::{
    capabilities::v1alpha::Capabilities, filefolder::v1alpha::FileFolder, fileobject::v1alpha::FileObject,
    versioning::v1alpha::Versioning,
};

use crate::{
    __impl_service_aggregate,
    backend::builder::{
        capabilities::v1alpha::AnyCapabilities, filefolder::v1alpha::AnyFileFolder, fileobject::v1alpha::AnyFileObject,
        versioning::v1alpha::AnyVersioning,
    },
};

__impl_service_aggregate!(
    AnyServiceSet,
    { fileobject, AnyFileObject, FileObject },
    { filefolder, AnyFileFolder, FileFolder },
    { versioning, AnyVersioning, Versioning },
    { capabilities, AnyCapabilities, Capabilities },
);

mod fileobject_impl {
    use bytes::Bytes;
    use futures::future::BoxFuture;
    use storage_core::traits::{fileobject::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

    impl FileObject for super::AnyServiceSet {
        fn inner(&self) -> Option<&dyn FileObject> {
            None
        }

        fn stat<'a: 'b, 'b>(&'a self, request: Request<StatRequest>) -> BoxFuture<'b, Result<StatResponse>> {
            self.fileobject.stat(request)
        }

        fn read<'a: 'b, 'b>(&'a self, request: Request<ReadRequest>) -> BoxFuture<'b, Result<BoxStream<ReadResponse>>> {
            self.fileobject.read(request)
        }

        fn read_from_address<'a: 'b, 'b>(
            &'a self,
            request: Request<ReadFromAddressRequest>,
        ) -> BoxFuture<'b, Result<BoxStream<ReadFromAddressResponse>>> {
            self.fileobject.read_from_address(request)
        }

        fn write<'a: 'b, 'b>(
            &'a self,
            request: Request<WriteParameters>,
            data_future: BoxFuture<'static, Result<Bytes>>,
        ) -> BoxFuture<'b, Result<BoxStream<WriteResponse>>> {
            self.fileobject.write(request, data_future)
        }

        fn delete<'a: 'b, 'b>(&'a self, request: Request<DeleteRequest>) -> BoxFuture<'b, Result<DeleteResponse>> {
            self.fileobject.delete(request)
        }

        fn abort_multipart_upload<'a: 'b, 'b>(
            &'a self,
            request: Request<AbortMultipartUploadRequest>,
        ) -> BoxFuture<'b, Result<AbortMultipartUploadResponse>> {
            self.fileobject.abort_multipart_upload(request)
        }

        fn complete_multipart_upload<'a: 'b, 'b>(
            &'a self,
            request: Request<CompleteMultipartUploadRequest>,
        ) -> BoxFuture<'b, Result<CompleteMultipartUploadResponse>> {
            self.fileobject.complete_multipart_upload(request)
        }

        fn upload_part<'a: 'b, 'b>(
            &'a self,
            request: Request<UploadPartRequest>,
        ) -> BoxFuture<'b, Result<UploadPartResponse>> {
            self.fileobject.upload_part(request)
        }

        fn complete_redirect_upload<'a: 'b, 'b>(
            &'a self,
            request: Request<CompleteRedirectUploadRequest>,
        ) -> BoxFuture<'b, Result<CompleteRedirectUploadResponse>> {
            self.fileobject.complete_redirect_upload(request)
        }

        fn fetch_write_type_info<'a: 'b, 'b>(
            &'a self,
            request: Request<FetchWriteTypeInfoRequest>,
        ) -> BoxFuture<'b, Result<FetchWriteTypeInfoResponse>> {
            self.fileobject.fetch_write_type_info(request)
        }

        fn enumerate<'a: 'b, 'b>(
            &'a self,
            request: Request<EnumerateRequest>,
            streaming: StreamingOptions,
        ) -> BoxFuture<'b, Result<BoxStream<EnumerateResponseWithContinueToken>>> {
            self.fileobject.enumerate(request, streaming)
        }

        fn copy<'a: 'b, 'b>(&'a self, request: Request<CopyRequest>) -> BoxFuture<'b, Result<CopyResponse>> {
            self.fileobject.copy(request)
        }

        fn move_object<'a: 'b, 'b>(&'a self, request: Request<MoveRequest>) -> BoxFuture<'b, Result<MoveResponse>> {
            self.fileobject.move_object(request)
        }

        fn is_read_only<'a: 'b, 'b>(
            &'a self,
            request: Request<IsReadOnlyRequest>,
        ) -> BoxFuture<'b, Result<IsReadOnlyResponse>> {
            self.fileobject.is_read_only(request)
        }
    }
}

mod filefolder_impl {
    use futures::future::BoxFuture;
    use storage_core::traits::{filefolder::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

    impl FileFolder for super::AnyServiceSet {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        fn list<'a: 'b, 'b>(
            &'a self,
            request: Request<ListRequest>,
            streaming: StreamingOptions,
        ) -> BoxFuture<'b, Result<BoxStream<ListResponseWithContinueToken>>> {
            self.filefolder.list(request, streaming)
        }

        fn list_stat<'a: 'b, 'b>(
            &'a self,
            request: Request<ListStatRequest>,
            streaming: StreamingOptions,
        ) -> BoxFuture<'b, Result<BoxStream<ListStatResponseWithContinueToken>>> {
            self.filefolder.list_stat(request, streaming)
        }

        fn delete_folder<'a: 'b, 'b>(
            &'a self,
            request: Request<DeleteFolderRequest>,
        ) -> BoxFuture<'b, Result<DeleteFolderResponse>> {
            self.filefolder.delete_folder(request)
        }

        fn create_folder<'a: 'b, 'b>(
            &'a self,
            request: Request<CreateFolderRequest>,
        ) -> BoxFuture<'b, Result<CreateFolderResponse>> {
            self.filefolder.create_folder(request)
        }

        fn get_folder_mode<'a: 'b, 'b>(
            &'a self,
            request: Request<GetFolderModeRequest>,
        ) -> BoxFuture<'b, Result<GetFolderModeResponse>> {
            self.filefolder.get_folder_mode(request)
        }
    }
}

mod capabilities_impl {
    use futures::future::BoxFuture;
    use storage_core::traits::{capabilities::v1alpha::*, Request, Result};

    impl Capabilities for super::AnyServiceSet {
        fn inner(&self) -> Option<&dyn Capabilities> {
            None
        }

        fn list_top_level_addresses<'a: 'b, 'b>(
            &'a self,
            request: Request<ListTopLevelAddressesRequest>,
        ) -> BoxFuture<'b, Result<ListTopLevelAddressesResponse>> {
            self.capabilities.list_top_level_addresses(request)
        }

        fn list_routes<'a: 'b, 'b>(
            &'a self,
            request: Request<ListRoutesRequest>,
        ) -> BoxFuture<'b, Result<ListRoutesResponse>> {
            self.capabilities.list_routes(request)
        }
    }
}

mod versioning_impl {
    use futures::future::BoxFuture;
    use storage_core::traits::{async_trait, versioning::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

    #[async_trait]
    impl Versioning for super::AnyServiceSet {
        fn inner(&self) -> Option<&dyn Versioning> {
            None
        }

        fn enumerate_versions<'a: 'b, 'b>(
            &'a self,
            request: Request<EnumerateVersionsRequest>,
            streaming: StreamingOptions,
            version_id_token: Option<String>,
        ) -> BoxFuture<'b, Result<BoxStream<EnumerateVersionsResponseWithContinueToken>>> {
            self.versioning.enumerate_versions(request, streaming, version_id_token)
        }

        fn is_versioned<'a: 'b, 'b>(
            &'a self,
            request: Request<IsVersionedRequest>,
        ) -> BoxFuture<'b, Result<IsVersionedResponse>> {
            self.versioning.is_versioned(request)
        }
    }
}
