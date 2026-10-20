// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use tracing::instrument;

use storage_core_builder::traits::{
    error::{Result, StorageError},
    filefolder::v1alpha::FileFolder,
    fileobject::v1alpha::{
        write_response, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse, CompleteRedirectUploadRequest,
        CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest, DeleteResponse, FileObject, Metadata,
        MoveRequest, MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ResourceIdentity, ResourceInfo,
        StatRequest, StatResponse, WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Request,
};

use storage_core_util::cache::CacheControl;

use crate::stat_cache::cache::{CachedStatResponse, StatCache};
use crate::stat_cache::error::Error;
use crate::stat_cache::{all_version_keys, canonical_cache_value, versioned_cache_key, ApiVersion};

#[async_trait]
impl<S> FileObject for StatCache<S>
where
    S: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    #[instrument(name = "stat_cache::stat", skip_all)]
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.stat(request).await;
        }
        let key = versioned_cache_key(&request.get_ref().resource_address, ApiVersion::V1Alpha)?;
        self.cache
            .try_get_with_by_ref(&key, async move {
                let response = self.inner.stat(request).await?;
                Ok(canonical_cache_value(response.into()))
            })
            .await
            .map(|r| r.into())
            .map_err(|e| Box::new(Error::cached_error(e)) as StorageError)
    }

    #[instrument(name = "stat_cache::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        mut request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        // NoCache: skip the stat-and-inject (which would both cache the stat and hand a possibly-stale
        // address→identity mapping to the layer below) and delegate straight through.
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.read_from_address(request).await;
        }
        // only stat if the resource info is not already in the extensions (another layer may have already cached it)
        if request.extensions().get::<ResourceInfo>().is_none() {
            // We need to stat the resource address to get the resource info.
            // One could argue that we should only `get` here, and not `try_get_with_by_ref`,
            // because it's strange for the stat cache to have logic like that. However, to
            // properly coalesce all stat requests, we need to do the stat here rather than
            // in the backend. This will also properly capture SDK metrics for the stat call.
            let key = versioned_cache_key(&request.get_ref().resource_address, ApiVersion::V1Alpha)?;
            let cached_entry = self
                .cache
                .try_get_with_by_ref(&key, async {
                    let stat_request = tonic::Request::from_parts(
                        request.metadata().clone(),
                        request.extensions().clone(),
                        StatRequest {
                            resource_address: request.get_ref().resource_address.clone(),
                        },
                    )
                    .into();
                    let response = self.inner.stat(stat_request).await?;
                    Ok(canonical_cache_value(response.into()))
                })
                .await
                .map(|r: CachedStatResponse| StatResponse::from(r).resource_info)
                .map_err(|e| Box::new(Error::cached_error(e)) as StorageError)?
                .ok_or_else(|| Error::internal("Cached StatResponse does not contain a ResourceInfo"))?;

            request.extensions_mut().insert(cached_entry);
        }

        let response = self.inner.read_from_address(request).await?;
        Ok(response)
    }

    #[instrument(name = "stat_cache::write", skip_all)]
    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let keys = all_version_keys(&request.get_ref().destination_resource_address)?;

        let stream = self.inner.write(request, data_future).await?;

        let cache = self.cache.clone();
        let stream = stream.then(move |response| {
            let cache = cache.clone();
            let keys = keys.clone();
            async move {
                match response {
                    Ok(WriteResponse {
                        write_response_type: Some(write_response::WriteResponseType::ResourceInfo(_)),
                    }) => {
                        // Invalidate every version partition when we get a ResourceInfo response.
                        for key in &keys {
                            cache.invalidate(key).await;
                        }
                        response
                    },
                    _ => response,
                }
            }
        });

        Ok(Box::pin(stream))
    }

    #[instrument(name = "stat_cache::delete", skip_all)]
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let keys = all_version_keys(&request.get_ref().resource_address)?;
        let response = self.inner.delete(request).await?;
        for key in &keys {
            self.cache.invalidate(key).await;
        }
        Ok(response)
    }

    #[instrument(name = "stat_cache::complete_multipart_upload", skip_all)]
    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let keys = all_version_keys(&request.get_ref().destination_resource_address)?;
        let response = self.inner.complete_multipart_upload(request).await?;
        for key in &keys {
            self.cache.invalidate(key).await;
        }
        Ok(response)
    }

    #[instrument(name = "stat_cache::complete_redirect_upload", skip_all)]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let keys = all_version_keys(&request.get_ref().destination_resource_address)?;
        let response = self.inner.complete_redirect_upload(request).await?;
        for key in &keys {
            self.cache.invalidate(key).await;
        }
        Ok(response)
    }

    #[instrument(name = "stat_cache::copy", skip_all)]
    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        let keys = all_version_keys(&request.get_ref().destination_resource_address)?;
        let response = self.inner.copy(request).await?;
        for key in &keys {
            self.cache.invalidate(key).await;
        }
        Ok(response)
    }

    #[instrument(name = "stat_cache::move_object", skip_all)]
    async fn move_object(&self, mut request: Request<MoveRequest>) -> Result<MoveResponse> {
        // Move invalidates the source and destination keys.
        let source_read_key = versioned_cache_key(&request.get_ref().source_resource_address, ApiVersion::V1Alpha)?;

        // Check if the source resource info is cached if the source previous version is not provided.
        // Consult both L1 and L2 — cross-pod cached stat avoids a backend round trip.
        if request.get_ref().source_previous_version.is_none() {
            match self.cache.get_two_tier(&source_read_key).await {
                Some(Ok(cached_entry)) => {
                    request.extensions_mut().insert(cached_entry);
                },
                Some(Err(e)) => {
                    return Err(Box::new(Error::cached_error(e)) as StorageError);
                },
                None => {
                    // TODO(saxonp): maybe perform stat here like read_from_address?
                },
            }
        }

        let source_keys = all_version_keys(&request.get_ref().source_resource_address)?;
        let destination_keys = all_version_keys(&request.get_ref().destination_resource_address)?;
        let response = self.inner.move_object(request).await?;
        for key in source_keys.iter().chain(&destination_keys) {
            self.cache.invalidate(key).await;
        }
        Ok(response)
    }
}

impl From<CachedStatResponse> for StatResponse {
    fn from(value: CachedStatResponse) -> Self {
        Self {
            resource_info: Some(ResourceInfo {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: value.encoded_identity,
                }),
                metadata: Some(Metadata {
                    native_metadata: value.native_metadata,
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp,
                }),
            }),
        }
    }
}

impl From<StatResponse> for CachedStatResponse {
    fn from(value: StatResponse) -> Self {
        Self {
            encoded_identity: value
                .resource_info
                .as_ref()
                .and_then(|r| r.resource_identity.as_ref())
                .map(|r| r.encoded_identity.clone())
                .unwrap_or_default(),
            data_object_size: value
                .resource_info
                .as_ref()
                .and_then(|r| r.metadata.as_ref())
                .map(|m| m.data_object_size)
                .unwrap_or_default(),
            last_modified_timestamp: value
                .resource_info
                .as_ref()
                .and_then(|r| r.metadata.as_ref())
                .map(|m| m.last_modified_timestamp)
                .unwrap_or_default(),
            native_metadata: value
                .resource_info
                .as_ref()
                .and_then(|r| r.metadata.as_ref())
                .map(|m| m.native_metadata.clone())
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl<S> FileFolder for StatCache<S>
where
    S: FileFolder,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }
}
