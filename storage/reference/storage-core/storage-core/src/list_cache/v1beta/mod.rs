// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

mod filefolder;
mod fileobject;

use std::sync::Arc;

use ahash::RandomState;
use either::Either;
use futures::StreamExt;
use storage_core_util::{debug_limited, trace_limited, warn_limited};
use tonic::metadata::MetadataMap;
use tonic::Extensions;
use tracing::instrument;
use url::Url;

use storage_core_builder::traits::{
    filefolder::v1beta::{FileFolder, FolderAddress, ListItem, ListStatRequest},
    fileobject::v1beta::{Metadata, ResourceIdentity, ResourceInfo, StatResponse},
    BoxStream, Result, StorageError, StreamingOptions,
};

use super::{cache, Error, ListCache, ListMap};

impl<S> ListCache<S>
where
    S: FileFolder,
{
    // get the list cache map for a given folder address
    #[instrument(name = "list_cache::v1beta::get_list_map", skip_all)]
    pub(crate) async fn get_list_map_v1beta(
        &self,
        folder_address: &Url,
        metadata: &MetadataMap,
        extensions: &Extensions,
    ) -> Result<Arc<ListMap>> {
        let key = Self::key(folder_address)?;
        trace_limited!(?key);

        self.cache
            .try_get_with_by_ref(key.as_ref(), async move {
                let start = tokio::time::Instant::now();
                let mut map = ListMap::with_hasher(RandomState::new());

                let list_request = tonic::Request::from_parts(
                    metadata.clone(),
                    extensions.clone(),
                    ListStatRequest {
                        folder: Some(FolderAddress {
                            uri: folder_address.to_string(),
                        }),
                    },
                )
                .into();

                let mut list_stream = self.inner.list_stat(list_request, StreamingOptions::Streamed).await?;

                while let Some(result) = list_stream.next().await {
                    let list_response = result?.response;
                    map.reserve(list_response.subfolder_addresses.len() + list_response.entries.len());

                    for subfolder in list_response.subfolder_addresses {
                        let decoded_subfolder_uri = percent_encoding::percent_decode_str(&subfolder.uri)
                            .decode_utf8()
                            .map_err(|e| {
                                Error::internal(format!("failed to decode subfolder URI: {}: {e}", subfolder.uri))
                            })?;
                        trace_limited!(subfolder.uri, subfolder = decoded_subfolder_uri.as_ref());
                        map.insert(
                            decoded_subfolder_uri.into_owned(),
                            cache::ListItem::FolderAddress(subfolder.uri),
                        );
                    }

                    for entry in list_response.entries {
                        let list_item = cache::ListItem::try_from(&entry)?;
                        let decoded_resource_address = percent_encoding::percent_decode_str(&entry.resource_address)
                            .decode_utf8()
                            .map_err(|e| Error::internal(e.to_string()))?;
                        trace_limited!(resource_address = decoded_resource_address.as_ref(), ?list_item);
                        map.insert(decoded_resource_address.into_owned(), list_item);
                    }
                }

                let duration = start.elapsed().as_secs_f64();
                trace_limited!(?folder_address, duration);

                if duration > 60.0 {
                    warn_limited!(%folder_address, duration, "List cache lookup took > 60 seconds!");
                }

                Ok(Arc::new(map))
            })
            .await
            .map_err(|e| Box::new(Error::cache(e)) as StorageError)
    }

    #[instrument(name = "list_cache::v1beta::list", skip_all)]
    pub(crate) async fn list_v1beta<T, U, V, F, G>(
        &self,
        folder_address: &Url,
        metadata: &MetadataMap,
        extensions: &Extensions,
        streaming: StreamingOptions,
        item_function: F,
        response_function: G,
    ) -> Result<BoxStream<T>>
    where
        T: std::marker::Send + 'static,
        U: std::marker::Send + 'static,
        V: std::marker::Send + 'static,
        F: Fn(&cache::ListItem) -> (Either<U, V>, u64) + std::marker::Send + 'static,
        G: Fn(Vec<U>, Vec<V>, Option<String>) -> T + std::marker::Send + 'static,
    {
        debug_limited!(?folder_address, ?streaming);

        let map = self.get_list_map_v1beta(folder_address, metadata, extensions).await?;

        Ok(match streaming {
            StreamingOptions::Streamed => {
                let page_size_limit = self.page_size_limit.as_u64();
                Box::pin(async_stream::stream! {
                    let mut folders = Vec::new();
                    let mut entries = Vec::new();
                    let mut response_size = 0;

                    for (key, value) in map.iter() {
                        trace_limited!(key, ?value);

                        // convert the cached item and get the size of the item in bytes
                        let (either, size) = item_function(value);

                        // see if adding new item will exceed the response size limit
                        if response_size > 0 && response_size + size >= page_size_limit {
                            trace_limited!(
                                response_size,
                                folders = folders.len(),
                                entries = entries.len(),
                                "page size limit reached"
                            );
                            yield Ok(response_function(folders, entries, None));
                            response_size = 0;
                            folders = Vec::new();
                            entries = Vec::new();
                        }

                        // update the response size
                        response_size += size;

                        // add the item to the response
                        match either {
                            Either::Left(folder) => folders.push(folder),
                            Either::Right(entry) => entries.push(entry),
                        }
                    }

                    trace_limited!(
                        response_size,
                        folders = folders.len(),
                        entries = entries.len(),
                        "final page response"
                    );

                    yield Ok(response_function(folders, entries, None));
                })
            },
            StreamingOptions::Paginated(pagination) => {
                let skip = pagination
                    .continuation_handle
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let take = pagination
                    .max_page_size
                    .map(|page_size| page_size as usize)
                    .unwrap_or(2000)
                    .min(2000);
                Box::pin(async_stream::stream! {
                    let iter = map.iter().enumerate().skip(skip).take(take);
                    let (capacity, _) = iter.size_hint();
                    let mut folders = Vec::with_capacity(capacity);
                    let mut entries = Vec::with_capacity(capacity);
                    let mut continue_index = None;

                    for (index, (_, value)) in iter {
                        let (either, _) = item_function(value);

                        // add the item to the response
                        match either {
                            Either::Left(folder) => folders.push(folder),
                            Either::Right(entry) => entries.push(entry),
                        }

                        continue_index = Some(index + 1);
                    }

                    // if we reached the end of the map, no continue token
                    let continue_token = continue_index
                        .filter(|index| *index < map.len())
                        .map(|index| index.to_string());

                    trace_limited!(?continue_token, folders = folders.len(), entries = entries.len(),);

                    yield Ok(response_function(folders, entries, continue_token));
                })
            },
        })
    }
}

impl TryFrom<&ListItem> for cache::ListItem {
    type Error = StorageError;

    fn try_from(item: &ListItem) -> Result<Self> {
        let resource_info = item
            .resource_info
            .as_ref()
            .ok_or(Error::internal("resource info not found"))?;
        let resource_identity = resource_info
            .resource_identity
            .as_ref()
            .ok_or(Error::internal("resource identity not found"))?;
        let metadata = resource_info.metadata.ok_or(Error::internal("metadata not found"))?;
        let list_item = Self::ListItem {
            resource_address: item.resource_address.clone(),
            resource_identity: resource_identity.encoded_identity.clone(),
            data_object_size: metadata.data_object_size,
            last_modified_timestamp: metadata.last_modified_timestamp,
            // v1beta cannot carry native metadata.
            native_metadata: Default::default(),
        };

        Ok(list_item)
    }
}

impl TryFrom<&cache::ListItem> for StatResponse {
    type Error = StorageError;

    fn try_from(list_item: &cache::ListItem) -> Result<Self> {
        match list_item {
            cache::ListItem::FolderAddress(uri) => {
                Err(Error::not_found(format!("folder address {uri} found in cache for stat response")).into())
            },
            cache::ListItem::ListItem {
                resource_address: _,
                resource_identity,
                data_object_size,
                last_modified_timestamp,
                // v1beta cannot carry native metadata.
                ..
            } => Ok(Self {
                resource_info: Some(ResourceInfo {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: resource_identity.clone(),
                    }),
                    metadata: Some(Metadata {
                        data_object_size: *data_object_size,
                        last_modified_timestamp: *last_modified_timestamp,
                    }),
                }),
            }),
        }
    }
}
