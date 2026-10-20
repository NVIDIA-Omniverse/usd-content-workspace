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
use storage_core::traits::{filefolder::v1alpha::*, BoxStream, Request, Result, StreamingOptions};
use storage_service_permissions::types::ResourceType;
use tonic::async_trait;

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
impl<S> FileFolder for Permissions<S>
where
    S: FileFolder,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let folder_uri = &request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(Error::request_folder_address_missing)?
            .uri;

        // The listed children share the folder's endpoint, so the response
        // doesn't need filtering either.
        if self.is_permission_check_excluded(folder_uri, ResourceType::Folder)? {
            return self.next.list(request, streaming).await;
        }

        self.validate_folder_access(&permission_metadata, folder_uri, Action::List)
            .await?;

        Ok(Box::pin(self.next.list(request, streaming).await?.and_then({
            let inner = self.inner.clone();
            move |response| {
                let inner = inner.clone();
                let permission_metadata = permission_metadata.clone();
                async move { inner.filter_list_response_v1alpha(&permission_metadata, response).await }
            }
        })))
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let folder_uri = &request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(Error::request_folder_address_missing)?
            .uri;

        // The listed children share the folder's endpoint, so the response
        // doesn't need filtering either.
        if self.is_permission_check_excluded(folder_uri, ResourceType::Folder)? {
            return self.next.list_stat(request, streaming).await;
        }

        self.validate_folder_access(&permission_metadata, folder_uri, Action::ListStat)
            .await?;

        Ok(Box::pin(self.next.list_stat(request, streaming).await?.and_then({
            let inner = self.inner.clone();
            move |response| {
                let inner = inner.clone();
                let permission_metadata = permission_metadata.clone();
                async move {
                    inner
                        .filter_list_stat_response_v1alpha(&permission_metadata, response)
                        .await
                }
            }
        })))
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        self.validate_folder_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request
                .get_ref()
                .folder
                .as_ref()
                .ok_or_else(Error::request_folder_address_missing)?
                .uri,
            Action::DeleteFolder,
        )
        .await?;

        self.next.delete_folder(request).await
    }

    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        self.validate_folder_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request
                .get_ref()
                .folder
                .as_ref()
                .ok_or_else(Error::request_folder_address_missing)?
                .uri,
            Action::CreateFolder,
        )
        .await?;

        self.next.create_folder(request).await
    }

    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        self.validate_folder_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request
                .get_ref()
                .folder
                .as_ref()
                .ok_or_else(Error::request_folder_address_missing)?
                .uri,
            Action::GetFolderMode,
        )
        .await?;

        self.next.get_folder_mode(request).await
    }
}

impl PermissionLayerInner {
    async fn filter_list_response_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        mut response: ListResponseWithContinueToken,
    ) -> Result<ListResponseWithContinueToken> {
        let mut batch = self.batch();
        for object_address in response.response.sub_resource_addresses.iter() {
            batch
                .add_resource(object_address, Some(ResourceType::Object), Action::List)
                .map_err(Error::response_resource_url_invalid)?;
        }

        for folder_address in response.response.subfolder_addresses.iter() {
            batch
                .add_resource(&folder_address.uri, Some(ResourceType::Folder), Action::List)
                .map_err(Error::response_resource_url_invalid)?;
        }

        // We've already validated access for the parent folder. If an entity access rule evaluates to Missing,
        // we'll allow it.
        let mut retain_iter = batch
            .execute(permission_metadata)
            .await?
            .into_retain_iter_with_allow_on_missing();
        response.response.sub_resource_addresses.retain_each(&mut retain_iter);
        response.response.subfolder_addresses.retain_each(&mut retain_iter);

        Ok(response)
    }

    async fn filter_list_stat_response_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        mut response: ListStatResponseWithContinueToken,
    ) -> Result<ListStatResponseWithContinueToken> {
        let mut batch = self.batch();
        for object_entry in response.response.entries.iter() {
            batch
                .add_resource(
                    &object_entry.resource_address,
                    Some(ResourceType::Object),
                    Action::ListStat,
                )
                .map_err(Error::response_resource_url_invalid)?;
        }

        for folder_address in response.response.subfolder_addresses.iter() {
            batch
                .add_resource(&folder_address.uri, Some(ResourceType::Folder), Action::ListStat)
                .map_err(Error::response_resource_url_invalid)?;
        }

        // We've already validated access for the parent folder. If an entity access rule evaluates to Missing,
        // we'll allow it.
        let mut retain_iter = batch
            .execute(permission_metadata)
            .await?
            .into_retain_iter_with_allow_on_missing();
        response.response.entries.retain_each(&mut retain_iter);
        response.response.subfolder_addresses.retain_each(&mut retain_iter);

        Ok(response)
    }
}
