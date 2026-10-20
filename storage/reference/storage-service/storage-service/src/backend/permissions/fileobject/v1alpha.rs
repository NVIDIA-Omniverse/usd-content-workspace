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

use futures::TryStreamExt;
use storage_core::traits::{fileobject::v1alpha::*, BoxFuture, BoxStream, Bytes, Request, Result, StreamingOptions};
use storage_service_permissions::types::ResourceType;
use tonic::async_trait;
use tracing::instrument;

use crate::{
    backend::permissions::{
        action::Action,
        error::Error,
        request::{PermissionRequestMetadata, RequestExt},
        PermissionLayerInner, Permissions,
    },
    common::iterator::RetainEachExt,
};

#[async_trait]
impl<S> FileObject for Permissions<S>
where
    S: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    #[instrument(skip_all)]
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().resource_address,
            Action::Stat,
        )
        .await?;

        self.next.stat(request).await
    }

    /// `IsReadOnly` is a capability query: it carries no principal and exposes no object content,
    /// so there is nothing to authorize.
    async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        self.next.is_read_only(request).await
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request
                .get_ref()
                .resource_identity
                .as_ref()
                .ok_or_else(Error::request_resource_identity_missing)?
                .encoded_identity,
            Action::Read,
        )
        .await?;

        self.next.read(request).await
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().resource_address,
            Action::ReadFromAddress,
        )
        .await?;

        self.next.read_from_address(request).await
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::Write,
        )
        .await?;

        self.next.write(request, data_future).await
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().resource_address,
            Action::Delete,
        )
        .await?;

        self.next.delete(request).await
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::AbortMultipartUpload,
        )
        .await?;

        self.next.abort_multipart_upload(request).await
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::CompleteMultipartUpload,
        )
        .await?;

        self.next.complete_multipart_upload(request).await
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::UploadPart,
        )
        .await?;

        self.next.upload_part(request).await
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::CompleteRedirectUpload,
        )
        .await?;

        self.next.complete_redirect_upload(request).await
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().destination_resource_address,
            Action::FetchWriteTypeInfo,
        )
        .await?;

        self.next.fetch_write_type_info(request).await
    }

    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        self.validate_binop_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request
                .get_ref()
                .source_resource_identity
                .as_ref()
                .ok_or_else(Error::request_resource_identity_missing)?
                .encoded_identity,
            Action::CopyFrom,
            &request.get_ref().destination_resource_address,
            Action::CopyTo,
        )
        .await?;

        self.next.copy(request).await
    }

    async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse> {
        self.validate_binop_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().source_resource_address,
            Action::MoveFrom,
            &request.get_ref().destination_resource_address,
            Action::MoveTo,
        )
        .await?;

        self.next.move_object(request).await
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;

        // The enumerated items share the folder's endpoint, so the response
        // doesn't need filtering either.
        if self.is_permission_check_excluded(&request.get_ref().resource_address, ResourceType::Folder)? {
            return self.next.enumerate(request, streaming).await;
        }

        self.validate_folder_access(
            &permission_metadata,
            &request.get_ref().resource_address,
            Action::Enumerate,
        )
        .await?;

        Ok(Box::pin(self.next.enumerate(request, streaming).await?.and_then({
            let inner = self.inner.clone();
            move |response| {
                let inner = inner.clone();
                let permission_metadata = permission_metadata.clone();
                async move {
                    inner
                        .filter_enumerate_response_v1alpha(&permission_metadata, response)
                        .await
                }
            }
        })))
    }
}

impl PermissionLayerInner {
    async fn filter_enumerate_response_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        mut response: EnumerateResponseWithContinueToken,
    ) -> Result<EnumerateResponseWithContinueToken> {
        let mut batch = self.batch();
        for item in response.response.items.iter() {
            batch
                .add_resource_hierarchically(&item.resource_address, Some(ResourceType::Object), Action::Enumerate)
                .map_err(Error::response_resource_url_invalid)?;
        }

        let mut retain_iter = batch
            .execute(permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        response.response.items.retain_each(&mut retain_iter);

        Ok(response)
    }
}
