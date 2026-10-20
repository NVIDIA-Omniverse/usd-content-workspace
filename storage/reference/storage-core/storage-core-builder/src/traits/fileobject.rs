// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1beta {
    use crate::traits::{BoxFuture, BoxStream, Bytes, Request, Result, StreamingOptions};
    /// Re-export the protos.
    pub use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::*;

    // All methods can now be included in a single trait using the macro
    crate::generate_storage_trait! {
        FileObject {
            async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse>;
            async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>>;
            async fn read_from_address(&self, request: Request<ReadFromAddressRequest>) -> Result<BoxStream<ReadFromAddressResponse>>;
            async fn write(
                &self,
                request: Request<WriteParameters>,
                data_future: BoxFuture<Bytes>,
            ) -> Result<BoxStream<WriteResponse>>;
            async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse>;
            async fn abort_multipart_upload(&self, request: Request<AbortMultipartUploadRequest>) -> Result<AbortMultipartUploadResponse>;
            async fn complete_multipart_upload(&self, request: Request<CompleteMultipartUploadRequest>) -> Result<CompleteMultipartUploadResponse>;
            async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse>;
            async fn complete_redirect_upload(&self, request: Request<CompleteRedirectUploadRequest>) -> Result<CompleteRedirectUploadResponse>;
            async fn fetch_write_type_info(&self, request: Request<FetchWriteTypeInfoRequest>) -> Result<FetchWriteTypeInfoResponse>;
            async fn enumerate(
                &self,
                request: Request<EnumerateRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<EnumerateResponseWithContinueToken>>;
        }
    }
    pub struct EnumerateResponseWithContinueToken {
        pub response: EnumerateResponse,
        pub continue_token: Option<String>,
    }

    impl EnumerateResponseWithContinueToken {
        pub fn new(response: EnumerateResponse, continue_token: String) -> Self {
            Self {
                response,
                continue_token: Some(continue_token),
            }
        }
    }
}

pub mod v1alpha {
    use crate::traits::{BoxFuture, BoxStream, Bytes, Request, Result, StreamingOptions};
    /// Re-export the protos.
    pub use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::*;

    // All methods can now be included in a single trait using the macro
    crate::generate_storage_trait! {
        FileObject {
            async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse>;
            async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>>;
            async fn read_from_address(&self, request: Request<ReadFromAddressRequest>) -> Result<BoxStream<ReadFromAddressResponse>>;
            async fn write(
                &self,
                request: Request<WriteParameters>,
                data_future: BoxFuture<Bytes>,
            ) -> Result<BoxStream<WriteResponse>>;
            async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse>;
            async fn abort_multipart_upload(&self, request: Request<AbortMultipartUploadRequest>) -> Result<AbortMultipartUploadResponse>;
            async fn complete_multipart_upload(&self, request: Request<CompleteMultipartUploadRequest>) -> Result<CompleteMultipartUploadResponse>;
            async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse>;
            async fn complete_redirect_upload(&self, request: Request<CompleteRedirectUploadRequest>) -> Result<CompleteRedirectUploadResponse>;
            async fn fetch_write_type_info(&self, request: Request<FetchWriteTypeInfoRequest>) -> Result<FetchWriteTypeInfoResponse>;
            async fn enumerate(
                &self,
                request: Request<EnumerateRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<EnumerateResponseWithContinueToken>>;
            async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse>;
            async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse>;
            async fn get_optimistic_locking_support(&self, request: Request<GetOptimisticLockingSupportRequest>) -> Result<GetOptimisticLockingSupportResponse>;
            async fn get_native_metadata_support(&self, request: Request<GetNativeMetadataSupportRequest>) -> Result<GetNativeMetadataSupportResponse>;
            async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse>;
        }
    }
    pub struct EnumerateResponseWithContinueToken {
        pub response: EnumerateResponse,
        pub continue_token: Option<String>,
    }

    impl EnumerateResponseWithContinueToken {
        pub fn new(response: EnumerateResponse, continue_token: String) -> Self {
            Self {
                response,
                continue_token: Some(continue_token),
            }
        }
    }
}
