// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use futures::future::join_all;
use tracing::instrument;

use storage_core_builder::traits::{
    error::{Result, StorageError},
    metadata::v1alpha::{
        update_metadata_batch_entry_result, DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest,
        GetMetadataResponse, Metadata, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest,
        UpdateMetadataResponse, UserMetadataValue,
    },
    Request,
};

use storage_core_util::metadata::normalize_metadata_key;

use crate::metadata_cache::cache::{CachedMetadata, CachedMetadataEntry, MetadataCache};
use crate::metadata_cache::canonical_cache_key;
use crate::metadata_cache::error::Error;

#[async_trait]
impl<S> Metadata for MetadataCache<S>
where
    S: Metadata,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "metadata_cache::get_metadata", skip_all)]
    async fn get_metadata(&self, mut request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let uri = canonical_cache_key(&request.get_ref().uri)?;
        // Take the caller's requested keys out of the request, leaving it with an empty vec.
        // This serves double duty: we save the keys for filtering later, and the emptied request
        // tells the backend to return ALL metadata so we can cache the complete snapshot.
        let requested_keys = std::mem::take(&mut request.get_mut().user_metadata_keys);

        // try_get_with_by_ref provides operation coalescing: concurrent requests for the same URI
        // share a single backend fetch rather than each issuing their own.
        let cached = self
            .cache
            .try_get_with_by_ref(&uri, async {
                let inner_response = self.inner.get_metadata(request).await?;

                let full_map: HashMap<String, _> = inner_response
                    .user_metadata
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            CachedMetadataEntry {
                                value: v.value,
                                etag: v.etag,
                            },
                        )
                    })
                    .collect();

                Ok(CachedMetadata(full_map))
            })
            .await
            .map_err(|e| Box::new(Error::cached_error(e)) as StorageError)?;

        Ok(self.build_response_v1alpha(&cached.0, &requested_keys))
    }

    #[instrument(name = "metadata_cache::update_metadata", skip_all)]
    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let uri = canonical_cache_key(&request.get_ref().uri)?;

        let response = self.inner.update_metadata(request).await?;

        self.cache.invalidate(&uri).await;

        Ok(response)
    }

    #[instrument(name = "metadata_cache::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        // Snapshot each request's URI by index so we can invalidate only the
        // URIs that the backend actually wrote to. The request is consumed by
        // the inner call, so we collect first.
        let uris_by_index: Vec<Option<String>> = request
            .get_ref()
            .requests
            .iter()
            // Batch entries are independent. An invalid URI must reach the
            // backend and become that entry's error instead of failing the
            // transport-level RPC before any entries are processed.
            .map(|entry| canonical_cache_key(&entry.uri).ok())
            .collect();

        let response = self.inner.update_metadata_batch(request).await?;

        // Invalidate only URIs with at least one successful entry — failed
        // entries didn't write, so their cached state is still valid. Cache
        // misses are safe, so it's better to under-invalidate than to evict
        // entries for URIs whose writes were rejected.
        let mut to_invalidate: HashSet<&str> = HashSet::new();
        for result in &response.results {
            if matches!(
                result.outcome,
                Some(update_metadata_batch_entry_result::Outcome::Success(_))
            ) {
                if let Some(Some(uri)) = uris_by_index.get(result.request_index as usize) {
                    to_invalidate.insert(uri.as_str());
                }
            }
        }

        join_all(to_invalidate.into_iter().map(|uri| self.cache.invalidate(uri))).await;

        Ok(response)
    }

    #[instrument(name = "metadata_cache::delete_metadata", skip_all)]
    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let uri = canonical_cache_key(&request.get_ref().uri)?;

        let response = self.inner.delete_metadata(request).await?;

        self.cache.invalidate(&uri).await;

        Ok(response)
    }
}

impl<S> MetadataCache<S> {
    fn build_response_v1alpha(
        &self,
        full_map: &HashMap<String, CachedMetadataEntry>,
        requested_keys: &[String],
    ) -> GetMetadataResponse {
        let to_value = |(k, entry): (&String, &CachedMetadataEntry)| {
            (
                k.clone(),
                UserMetadataValue {
                    value: entry.value.clone(),
                    etag: entry.etag.clone(),
                },
            )
        };

        let user_metadata = if requested_keys.is_empty() {
            full_map.iter().map(|(k, v)| to_value((k, v))).collect()
        } else if self.normalize_keys {
            requested_keys
                .iter()
                .filter_map(|k| {
                    let normalized = normalize_metadata_key(k.clone());
                    full_map.get(&normalized).map(|v| to_value((&normalized, v)))
                })
                .collect()
        } else {
            requested_keys
                .iter()
                .filter_map(|k| full_map.get(k).map(|v| to_value((k, v))))
                .collect()
        };

        GetMetadataResponse { user_metadata }
    }
}
