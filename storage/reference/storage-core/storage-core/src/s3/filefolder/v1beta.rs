// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use storage_core_util::additional_headers::AdditionalHeaders;
use storage_core_util::debug_limited;
use tracing::instrument;

use crate::traits::{
    filefolder::v1beta::{
        DeleteFolderRequest, DeleteFolderResponse, FileFolder as FileFolderTrait, FolderAddress, ListItem, ListRequest,
        ListResponse, ListResponseWithContinueToken, ListStatRequest, ListStatResponse,
        ListStatResponseWithContinueToken,
    },
    BoxStream, Request, Result, StreamingOptions,
};

use super::super::{
    error,
    fileobject::{encode_list_versions_page_markers, ListObjectVersionsPageResponse, ListVersionsMarkerMode},
    resource::{Address, Identity, WrappedResourceInfo},
    service::Storage,
};

pub static DELIMITER: &str = "/";

#[async_trait]
impl FileFolderTrait for Storage {
    fn inner(&self) -> Option<&dyn FileFolderTrait> {
        None
    }

    #[instrument(name = "s3::list", skip_all)]
    async fn list(
        &self,
        mut request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?streaming, ?additional_headers);

        let request = request.into_inner();

        let folder_address = request
            .folder
            .ok_or_else(|| error::Error::invalid_argument("Folder address is required"))?;

        let mut folder_address = Address::parse_folder_address(&folder_address.uri)?;
        let client = self.client(&mut folder_address).await?;

        self.list_sdk(
            &client,
            Some(DELIMITER),
            folder_address,
            streaming,
            additional_headers,
            move |contents, common_prefixes, continue_token, folder_address| {
                // We can't determine the bucket address until after we resolve the client
                // to determine the URL addressing style
                let bucket_address = folder_address.bucket_address()?;
                let subfolder_addresses: Vec<_> = common_prefixes
                    .filter_map(|prefix| prefix.prefix.as_deref())
                    .map(|prefix| FolderAddress {
                        uri: bucket_address.get_folder_address(prefix).into(),
                    })
                    .collect();
                let sub_resource_addresses: Vec<_> = contents
                    .filter_map(|object| object.key())
                    .map(|key| -> String { bucket_address.get_object_address(key).into() })
                    .collect();

                Ok(ListResponseWithContinueToken {
                    response: ListResponse {
                        subfolder_addresses,
                        sub_resource_addresses,
                    },
                    continue_token,
                })
            },
        )
        .await
    }

    #[instrument(name = "s3::list_stat", skip_all)]
    async fn list_stat(
        &self,
        mut request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?streaming, ?additional_headers);

        let request = request.into_inner();

        let folder_address = request
            .folder
            .ok_or_else(|| error::Error::invalid_argument("Folder address is required"))?;

        let mut folder_address = Address::parse_folder_address(&folder_address.uri)?;
        let client = self.client(&mut folder_address).await?;

        self.list_versions_sdk(
            &client,
            Some(DELIMITER),
            folder_address,
            streaming,
            ListVersionsMarkerMode::ListStatCombined,
            true,
            additional_headers,
            move |output, folder_address, next_markers| {
                // We can't determine the bucket address until after we resolve the client
                // to determine the URL addressing style
                let bucket_address = folder_address.bucket_address()?;
                let prefix = folder_address.key()?.to_string();

                let subfolder_addresses: Vec<_> = output
                    .common_prefixes()
                    .iter()
                    .filter_map(|cp| cp.prefix.as_deref())
                    .map(|p| FolderAddress {
                        uri: bucket_address.get_folder_address(p).into(),
                    })
                    .collect();

                let entries: Vec<_> = output
                    .versions()
                    .iter()
                    .filter(|v| v.is_latest() == Some(true))
                    .filter_map(|object| {
                        let key = object.key()?;
                        if key == prefix {
                            return None;
                        }
                        let resource_address = bucket_address.get_object_address(key);
                        let version_id = object.version_id().filter(|v| *v != "null").map(String::from);
                        let resource_identity = Identity::from((
                            resource_address.clone(),
                            object.e_tag().map(ToOwned::to_owned),
                            version_id,
                        ));
                        let data_object_size = object.size().map(|s| s as u64);
                        let last_modified_timestamp =
                            object
                                .last_modified()
                                .map(|ts| storage_core_protos::google::protobuf::Timestamp {
                                    seconds: ts.secs(),
                                    nanos: ts.subsec_nanos() as i32,
                                });

                        Some(ListItem {
                            resource_address: resource_address.into(),
                            resource_info: Some(
                                WrappedResourceInfo::from((
                                    &resource_identity,
                                    data_object_size,
                                    last_modified_timestamp,
                                ))
                                .0
                                .into(),
                            ),
                        })
                    })
                    .collect();

                let is_empty = subfolder_addresses.is_empty() && entries.is_empty();

                Ok(ListObjectVersionsPageResponse {
                    response: ListStatResponseWithContinueToken {
                        response: ListStatResponse {
                            subfolder_addresses,
                            entries,
                        },
                        continue_token: encode_list_versions_page_markers(next_markers),
                    },
                    is_empty,
                })
            },
        )
        .await
    }

    #[instrument(name = "s3::delete_folder", skip_all)]
    async fn delete_folder(&self, mut request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| error::Error::invalid_argument("Folder address is required"))?;
        let folder_address = Address::parse_folder_address(&folder_address.uri)?;
        self.delete_folder_sdk(folder_address, additional_headers).await?;
        Ok(DeleteFolderResponse {})
    }
}
