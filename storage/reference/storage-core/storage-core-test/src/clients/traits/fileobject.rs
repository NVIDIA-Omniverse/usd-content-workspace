// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use storage_core::traits::fileobject::v1alpha::{
    AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
    CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest,
    CopyResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponse, FetchWriteTypeInfoRequest,
    FetchWriteTypeInfoResponse, GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse,
    GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse, IsReadOnlyRequest, IsReadOnlyResponse,
    MoveRequest, MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest,
    StatResponse, UploadPartRequest, UploadPartResponse, WriteParameters, WriteResponse,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

#[tonic::async_trait]
pub trait FileObject
where
    Self: std::marker::Send + std::marker::Sync + 'static,
{
    async fn stat(&mut self, _request: StatRequest) -> Result<StatResponse, Status> {
        unimplemented!()
    }
    async fn read_from_address(
        &mut self,
        _request: ReadFromAddressRequest,
    ) -> Result<ReceiverStream<Result<ReadFromAddressResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn read(&mut self, _request: ReadRequest) -> Result<ReceiverStream<Result<ReadResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn enumerate(
        &mut self,
        _request: EnumerateRequest,
    ) -> Result<ReceiverStream<Result<EnumerateResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn write(&mut self, _request: WriteParameters, _data: Option<Bytes>) -> Result<WriteResponse, Status> {
        unimplemented!()
    }

    async fn delete(&mut self, _request: DeleteRequest) -> Result<DeleteResponse, Status> {
        unimplemented!()
    }

    async fn abort_multipart_upload(
        &mut self,
        _request: AbortMultipartUploadRequest,
    ) -> Result<AbortMultipartUploadResponse, Status> {
        unimplemented!()
    }

    async fn complete_multipart_upload(
        &mut self,
        _request: CompleteMultipartUploadRequest,
    ) -> Result<CompleteMultipartUploadResponse, Status> {
        unimplemented!()
    }

    async fn upload_part(&mut self, _request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
        unimplemented!()
    }

    async fn complete_redirect_upload(
        &mut self,
        _request: CompleteRedirectUploadRequest,
    ) -> Result<CompleteRedirectUploadResponse, Status> {
        unimplemented!()
    }

    async fn fetch_write_type_info(
        &mut self,
        _request: FetchWriteTypeInfoRequest,
    ) -> Result<FetchWriteTypeInfoResponse, Status> {
        unimplemented!()
    }

    async fn get_optimistic_locking_support(
        &mut self,
        _request: GetOptimisticLockingSupportRequest,
    ) -> Result<GetOptimisticLockingSupportResponse, Status> {
        unimplemented!()
    }

    async fn get_native_metadata_support(
        &mut self,
        _request: GetNativeMetadataSupportRequest,
    ) -> Result<GetNativeMetadataSupportResponse, Status> {
        unimplemented!()
    }

    async fn is_read_only(&mut self, _request: IsReadOnlyRequest) -> Result<IsReadOnlyResponse, Status> {
        unimplemented!()
    }

    async fn copy(&mut self, _request: CopyRequest) -> Result<CopyResponse, Status> {
        unimplemented!()
    }

    async fn move_object(&mut self, _request: MoveRequest) -> Result<MoveResponse, Status> {
        unimplemented!()
    }
}
