// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use storage_core_util::additional_headers::AdditionalHeaders;
use storage_core_util::debug_limited;
use tracing::instrument;

use crate::traits::{
    filefolder::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse,
        FileFolder as FileFolderTrait, FolderAddress, FolderMode, GetFolderModeRequest, GetFolderModeResponse,
        ListItem, ListRequest, ListResponse, ListResponseWithContinueToken, ListStatRequest, ListStatResponse,
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

/// Max concurrent HeadObject calls when the ListStat native-metadata fan-out is enabled. Kept in the
/// conservative 8–16 band: a burst of HEADs against a single prefix can draw S3 `503 SlowDown`.
const NATIVE_METADATA_LIST_FANOUT_CONCURRENCY: usize = 16;

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
        let want_native_metadata = request.include_native_metadata;

        let folder_address = request
            .folder
            .ok_or_else(|| error::Error::invalid_argument("Folder address is required"))?;

        let mut folder_address = Address::parse_folder_address(&folder_address.uri)?;
        let client = self.client(&mut folder_address).await?;

        // S3 LIST returns no user metadata. When the caller asked for native metadata, the feature is
        // enabled, and the (off-by-default) fan-out toggle is on, we HeadObject each listed entry below.
        // Unlike Azure — which returns metadata inline on list and ignores `include_native_metadata` —
        // S3 honors the flag: the fan-out only runs when the caller sets it.
        let want_fanout = want_native_metadata
            && self.config.native_metadata_enabled.unwrap_or(false)
            && self.config.native_metadata_list_fanout_enabled.unwrap_or(false);
        let fanout_headers = additional_headers.clone();

        let stream = self
            .list_versions_sdk(
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
            .await?;

        if !want_fanout {
            return Ok(stream);
        }

        // HeadObject each listed entry (bounded concurrency) to populate native_metadata. Fail-soft per
        // entry so an object deleted between LIST and HEAD does not fail the whole listing. Pages are
        // collected before returning: the enrichment borrows `&self` (via `client`/`stat_sdk`) but the
        // returned `BoxStream` is `'static`, so it cannot hold that borrow across a per-page `and_then`.
        // This is acceptable because the fan-out is an opt-in for small folders — the startup guard
        // (`Config::validate_native_metadata_cache_consistency`) is what keeps it from being required on
        // the large folders `list_cache` exists for. Each page's continue token is preserved.
        use futures::StreamExt;
        let pages: Vec<ListStatResponseWithContinueToken> = futures::TryStreamExt::try_collect(stream).await?;
        let mut enriched_pages = Vec::with_capacity(pages.len());
        for mut page in pages {
            let entries = std::mem::take(&mut page.response.entries);
            let enriched: Vec<ListItem> = futures::stream::iter(entries)
                .map(|mut entry| {
                    let headers = fanout_headers.clone();
                    async move {
                        // Resolve each listed entry the same way stat() does: `client()` normalizes the
                        // address (e.g. strips the bucket from a path-style URL) so `stat_sdk` HeadObjects
                        // the right key. Reusing the folder's client with an unresolved address made every
                        // path-style (MinIO / S3-compatible) HEAD hit `bucket/bucket/key` and 404.
                        match Address::parse(&entry.resource_address) {
                            Ok(mut addr) => match self.client(&mut addr).await {
                                Ok(entry_client) => match self.stat_sdk(&entry_client, addr, headers).await {
                                    Ok(ri) => {
                                        if let Some(info) = entry.resource_info.as_mut() {
                                            if let Some(meta) = info.metadata.as_mut() {
                                                meta.native_metadata = ri.native_metadata;
                                            }
                                        }
                                    },
                                    Err(e) => storage_core_util::warn_limited!(
                                        resource_address = %entry.resource_address,
                                        error = %e,
                                        "list_stat native metadata HEAD failed; leaving native_metadata empty for this entry"
                                    ),
                                },
                                Err(e) => storage_core_util::warn_limited!(
                                    resource_address = %entry.resource_address,
                                    error = %e,
                                    "list_stat native metadata: client resolution failed; leaving native_metadata empty for this entry"
                                ),
                            },
                            Err(e) => storage_core_util::warn_limited!(
                                resource_address = %entry.resource_address,
                                error = %e,
                                "list_stat native metadata: unparseable resource address; skipping"
                            ),
                        }
                        entry
                    }
                })
                .buffer_unordered(NATIVE_METADATA_LIST_FANOUT_CONCURRENCY)
                .collect()
                .await;
            page.response.entries = enriched;
            enriched_pages.push(page);
        }
        Ok(Box::pin(futures::stream::iter(enriched_pages.into_iter().map(Ok))))
    }

    #[instrument(name = "s3::get_folder_mode", skip_all)]
    async fn get_folder_mode(&self, _request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        Ok(GetFolderModeResponse {
            folder_mode: FolderMode::Hybrid as i32,
        })
    }

    #[instrument(name = "s3::create_folder", skip_all)]
    async fn create_folder(&self, mut request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let folder_address = request
            .into_inner()
            .folder
            .ok_or_else(|| error::Error::invalid_argument("Folder address is required"))?;
        let folder_address = Address::parse_folder_address(&folder_address.uri)?;
        self.create_folder_sdk(folder_address, additional_headers).await?;
        Ok(CreateFolderResponse {})
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
