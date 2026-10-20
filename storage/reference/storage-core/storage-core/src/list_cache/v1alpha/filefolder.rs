// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use either::Either;
use get_size2::GetSize;
use storage_core_util::debug_limited;
use tracing::instrument;

use storage_core_builder::traits::{
    filefolder::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse, FileFolder,
        FolderAddress, ListItem, ListRequest, ListResponse, ListResponseWithContinueToken, ListStatRequest,
        ListStatResponse, ListStatResponseWithContinueToken,
    },
    fileobject::v1alpha::{Metadata, ResourceIdentity, ResourceInfo},
    BoxStream, Request, Result, StreamingOptions,
};

use super::super::{cache, ListCache};

#[async_trait]
impl<S> FileFolder for ListCache<S>
where
    S: FileFolder,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    #[instrument(name = "list_cache::list", skip_all)]
    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let folder_address = Self::ensure_trailing_slash(request.get_ref().folder.as_ref().map(|folder| &folder.uri))?;
        debug_limited!(?folder_address);

        self.list_v1alpha(
            &folder_address,
            request.metadata(),
            request.extensions(),
            streaming,
            move |list_item| match list_item {
                cache::ListItem::FolderAddress(uri) => {
                    let folder = FolderAddress { uri: uri.clone() };
                    let size = folder.get_size() as u64;
                    (Either::Left(folder), size)
                },
                cache::ListItem::ListItem { resource_address, .. } => (
                    Either::Right(resource_address.clone()),
                    resource_address.get_size() as u64,
                ),
            },
            move |subfolder_addresses, sub_resource_addresses, continue_token| ListResponseWithContinueToken {
                response: ListResponse {
                    subfolder_addresses,
                    sub_resource_addresses,
                },
                continue_token,
            },
        )
        .await
    }

    #[instrument(name = "list_cache::list_stat", skip_all)]
    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let folder_address = Self::ensure_trailing_slash(request.get_ref().folder.as_ref().map(|folder| &folder.uri))?;
        debug_limited!(?folder_address);

        self.list_v1alpha(
            &folder_address,
            request.metadata(),
            request.extensions(),
            streaming,
            move |list_item| match list_item {
                cache::ListItem::FolderAddress(uri) => {
                    let folder = FolderAddress { uri: uri.clone() };
                    let size = folder.get_size() as u64;
                    (Either::Left(folder), size)
                },
                cache::ListItem::ListItem {
                    resource_address,
                    resource_identity,
                    data_object_size,
                    last_modified_timestamp,
                    native_metadata,
                } => {
                    let list_item = ListItem {
                        resource_address: resource_address.clone(),
                        resource_info: Some(ResourceInfo {
                            resource_identity: Some(ResourceIdentity {
                                encoded_identity: resource_identity.clone(),
                            }),
                            metadata: Some(Metadata {
                                native_metadata: native_metadata.clone(),
                                data_object_size: *data_object_size,
                                last_modified_timestamp: *last_modified_timestamp,
                            }),
                        }),
                    };
                    let size = list_item.get_size() as u64;
                    (Either::Right(list_item), size)
                },
            },
            move |subfolder_addresses, entries, continue_token| ListStatResponseWithContinueToken {
                response: ListStatResponse {
                    subfolder_addresses,
                    entries,
                },
                continue_token,
            },
        )
        .await
    }

    #[instrument(name = "list_cache::create_folder", skip_all)]
    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let folder_address = Self::ensure_trailing_slash(request.get_ref().folder.as_ref().map(|folder| &folder.uri))?;
        let response = self.inner.create_folder(request).await?;
        // invalidate the parent folder address and all its children
        let parent_url = Self::parent_url(folder_address.clone())?;
        let key = Self::key(&parent_url)?;
        debug_limited!(?key);
        self.cache.invalidate_prefix(key.as_ref()).await;
        // also invalidate all ancestors above the parent
        let ancestor_keys = Self::ancestor_keys(folder_address)?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::delete_folder", skip_all)]
    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let folder_address = Self::ensure_trailing_slash(request.get_ref().folder.as_ref().map(|folder| &folder.uri))?;
        let response = self.inner.delete_folder(request).await?;
        // invalidate the parent folder address and all its children
        let parent_url = Self::parent_url(folder_address.clone())?;
        let key = Self::key(&parent_url)?;
        debug_limited!(?key);
        self.cache.invalidate_prefix(key.as_ref()).await;
        // also invalidate all ancestors above the parent
        let ancestor_keys = Self::ancestor_keys(folder_address)?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }
}
