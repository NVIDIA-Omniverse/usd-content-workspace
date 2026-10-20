// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use storage_core_util::{debug_limited, trace_limited};
use tracing::instrument;

use storage_core_builder::traits::{
    filefolder::v1alpha::FileFolder,
    fileobject::v1alpha::{
        write_response, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse, CompleteRedirectUploadRequest,
        CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest, DeleteResponse, FileObject,
        MoveRequest, MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ResourceInfo, StatRequest,
        StatResponse, WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Request, Result,
};

use storage_core_util::cache::CacheControl;

use super::super::{Error, ListCache};

#[async_trait]
impl<S> FileObject for ListCache<S>
where
    S: FileObject + FileFolder,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    #[instrument(name = "list_cache::v1alpha::stat", skip_all)]
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        // NoCache: don't answer from the cached parent-folder listing (which can hold a stale
        // resource identity); delegate straight through. The manifest layer relies on this for its
        // reserved `.manifest` accesses.
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.stat(request).await;
        }
        let resource_address = &request.get_ref().resource_address;
        trace_limited!(?resource_address);
        let mut resource_url = url::Url::parse(resource_address).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?resource_url);

        // list_stat() does not support versioning, so if the resource address has a version-id query parameter,
        // we need to pass the request to the inner to handle it.
        for query_param in resource_url.query_pairs() {
            if query_param.0 == "version-id" {
                return self.inner.stat(request).await;
            }
        }

        // Listed entries are keyed without query/fragment (see get_list_map_v1alpha
        // in v1alpha/mod.rs), so normalize the per-entry lookup key to match.
        resource_url.set_query(None);
        resource_url.set_fragment(None);

        let parent_url = Self::parent_url(resource_url.clone()).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?parent_url);

        let map = self
            .get_list_map_v1alpha(&parent_url, request.metadata(), request.extensions())
            .await?;

        let key = Self::key(&resource_url).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?key);

        match map.get(key.as_ref()) {
            Some(list_item) => Ok(list_item.try_into()?),
            None => Err(Error::not_found(format!("'{key}' resource address not found in parent folder cache")).into()),
        }
    }

    #[instrument(name = "list_cache::v1alpha::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        mut request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        // NoCache: skip the stat-and-inject (which would hand a possibly-stale cached identity to the
        // layer below) and delegate straight through.
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.read_from_address(request).await;
        }
        // only stat if the resource info is not already in the extensions (another layer may have already cached it)
        if request.extensions().get::<ResourceInfo>().is_none() {
            // look up the resource address in the list cache and stash the result in the extensions
            // so that the backends can use it in their read_from_address() implementations
            let stat_request = tonic::Request::from_parts(
                request.metadata().clone(),
                request.extensions().clone(),
                StatRequest {
                    resource_address: request.get_ref().resource_address.clone(),
                },
            )
            .into();
            let stat_response = self.stat(stat_request).await?;
            let cached_entry = stat_response
                .resource_info
                .ok_or(Error::internal("resource info not found in stat response"))?;
            request.extensions_mut().insert(cached_entry);
        }

        self.inner.read_from_address(request).await
    }

    #[instrument(name = "list_cache::v1alpha::write", skip_all)]
    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().destination_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let stream = self.inner.write(request, data_future).await?;
        let cache = self.cache.clone();
        let stream = stream.then(move |response| {
            let cache = cache.clone();
            let ancestor_keys = ancestor_keys.clone();
            async move {
                match response {
                    Ok(WriteResponse {
                        write_response_type: Some(write_response::WriteResponseType::ResourceInfo(_)),
                    }) => {
                        // Invalidate all ancestor cache entries when we get a ResourceInfo response
                        for key in &ancestor_keys {
                            cache.invalidate(key.as_str()).await;
                        }
                        response
                    },
                    _ => response,
                }
            }
        });

        Ok(Box::pin(stream))
    }

    #[instrument(name = "list_cache::v1alpha::delete", skip_all)]
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.delete(request).await?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::v1alpha::complete_multipart_upload", skip_all)]
    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().destination_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.complete_multipart_upload(request).await?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::v1alpha::complete_redirect_upload", skip_all)]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().destination_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.complete_redirect_upload(request).await?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::v1alpha::copy", skip_all)]
    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().destination_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.copy(request).await?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::v1alpha::move_object", skip_all)]
    async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse> {
        let dst_ancestor_keys = Self::ancestor_keys(request.get_ref().destination_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let src_ancestor_keys = Self::ancestor_keys(request.get_ref().source_resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.move_object(request).await?;
        self.invalidate_ancestors(&dst_ancestor_keys).await;
        self.invalidate_ancestors(&src_ancestor_keys).await;
        Ok(response)
    }
}
