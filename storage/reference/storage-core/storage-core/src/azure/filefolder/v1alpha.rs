// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::{
    azure::{
        error::Error,
        resource::{parse_container_and_folder_address, WrappedResourceInfo, DELIMITER},
        service::Storage,
    },
    traits::{
        filefolder::v1alpha::{
            CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse, FileFolder,
            FolderAddress, FolderMode, GetFolderModeRequest, GetFolderModeResponse, ListItem, ListRequest,
            ListResponse, ListResponseWithContinueToken, ListStatRequest, ListStatResponse,
            ListStatResponseWithContinueToken,
        },
        BoxStream, Request, Result, StreamingOptions,
    },
};

use storage_core_util::additional_headers::AdditionalHeaders;

#[tonic::async_trait]
impl FileFolder for Storage {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    #[instrument(name = "azure::list", skip_all)]
    async fn list(
        &self,
        mut request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| Error::invalid_argument("Folder address is missing"))?;

        let (container_url, folder_address) = parse_container_and_folder_address(&folder_address.uri)?;

        self.list_internal(
            Some(DELIMITER),
            folder_address,
            false,
            streaming,
            additional_headers,
            move |blobs, prefixes, next_marker, _| {
                Ok(ListResponseWithContinueToken {
                    response: ListResponse {
                        subfolder_addresses: prefixes
                            .map(|p| FolderAddress {
                                uri: container_url.get_folder_address(&p.name),
                            })
                            .collect(),
                        sub_resource_addresses: blobs.map(|b| container_url.get_blob_url(&b.name).into()).collect(),
                    },
                    continue_token: next_marker.map(|m| m.as_str().to_string()),
                })
            },
        )
        .await
    }

    #[instrument(name = "azure::list_stat", skip_all)]
    async fn list_stat(
        &self,
        mut request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| Error::invalid_argument("Folder address is required"))?;

        let (container_url, folder_address) = parse_container_and_folder_address(&folder_address.uri)?;

        self.list_internal(
            Some(DELIMITER),
            folder_address,
            false,
            streaming,
            additional_headers,
            move |blobs, prefixes, next_marker, _| {
                Ok(ListStatResponseWithContinueToken {
                    response: ListStatResponse {
                        subfolder_addresses: prefixes
                            .map(|p| FolderAddress {
                                uri: container_url.get_folder_address(&p.name),
                            })
                            .collect(),
                        entries: blobs
                            .map(|b| {
                                let blob_url = container_url.get_blob_url(&b.name);
                                ListItem {
                                    resource_address: blob_url.to_string(),
                                    resource_info: Some(
                                        WrappedResourceInfo::from((blob_url, b, &b.version_id)).0.into(),
                                    ),
                                }
                            })
                            .collect(),
                    },
                    continue_token: next_marker.map(|m| m.as_str().to_string()),
                })
            },
        )
        .await
    }

    #[instrument(name = "azure::create_folder", skip_all)]
    async fn create_folder(&self, mut request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| Error::invalid_argument("Folder address is missing"))?;

        self.create_folder_internal(folder_address.uri, additional_headers)
            .await?;

        Ok(CreateFolderResponse {})
    }

    #[instrument(name = "azure::delete_folder", skip_all)]
    async fn delete_folder(&self, mut request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| Error::invalid_argument("Folder address is missing"))?;

        self.delete_folder_internal(folder_address.uri, additional_headers)
            .await?;

        Ok(DeleteFolderResponse {})
    }

    #[instrument(name = "azure::get_folder_mode", skip_all)]
    async fn get_folder_mode(&self, _request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        Ok(GetFolderModeResponse {
            folder_mode: FolderMode::Hybrid as i32,
        })
    }
}
