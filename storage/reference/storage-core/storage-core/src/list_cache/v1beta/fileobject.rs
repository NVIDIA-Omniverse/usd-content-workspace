// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use storage_core_util::debug_limited;
use tracing::instrument;

use storage_core_builder::traits::{
    filefolder::v1beta::FileFolder,
    fileobject::v1beta::{
        write_response, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse, CompleteRedirectUploadRequest,
        CompleteRedirectUploadResponse, DeleteRequest, DeleteResponse, FileObject, ReadFromAddressRequest,
        ReadFromAddressResponse, ResourceInfo, StatRequest, StatResponse, WriteParameters, WriteResponse,
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

    #[instrument(name = "list_cache::v1beta::stat", skip_all)]
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
        let mut resource_url = url::Url::parse(resource_address).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?resource_url);

        // list_stat() does not support versioning, so if the resource address has a version-id query parameter,
        // we need to pass the request to the inner to handle it.
        for query_param in resource_url.query_pairs() {
            if query_param.0 == "version-id" {
                return self.inner.stat(request).await;
            }
        }

        // Listed entries are keyed without query/fragment (see get_list_map_v1beta
        // in v1beta/mod.rs), so normalize the per-entry lookup key to match.
        resource_url.set_query(None);
        resource_url.set_fragment(None);

        let parent_url = Self::parent_url(resource_url.clone()).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?parent_url);

        let map = self
            .get_list_map_v1beta(&parent_url, request.metadata(), request.extensions())
            .await?;

        let key = Self::key(&resource_url).map_err(|e| Error::invalid_argument(e.to_string()))?;
        debug_limited!(?key);

        match map.get(key.as_ref()) {
            Some(list_item) => Ok(list_item.try_into()?),
            None => Err(Error::not_found("resource address not found in parent folder cache").into()),
        }
    }

    #[instrument(name = "list_cache::v1beta::read_from_address", skip_all)]
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

    #[instrument(name = "list_cache::v1beta::write", skip_all)]
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

    #[instrument(name = "list_cache::v1beta::delete", skip_all)]
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let ancestor_keys = Self::ancestor_keys(request.get_ref().resource_address.as_str())
            .map_err(|e| Error::invalid_argument(e.to_string()))?;
        let response = self.inner.delete(request).await?;
        self.invalidate_ancestors(&ancestor_keys).await;
        Ok(response)
    }

    #[instrument(name = "list_cache::v1beta::complete_multipart_upload", skip_all)]
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

    #[instrument(name = "list_cache::v1beta::complete_redirect_upload", skip_all)]
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
}

#[cfg(test)]
mod nocache_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use storage_core_builder::layer::Layer;
    use storage_core_builder::traits::{
        async_trait,
        filefolder::v1beta::{
            FileFolder, ListItem, ListStatRequest, ListStatResponse, ListStatResponseWithContinueToken,
        },
        fileobject::v1beta::{FileObject, Metadata, ResourceIdentity, ResourceInfo, StatRequest, StatResponse},
        BoxStream, Request, Result, StreamingOptions,
    };
    use storage_core_util::cache::CacheControl;

    use crate::list_cache::{Config, ListCacheLayer};

    const OBJECT: &str = "https://bucket.example.com/folder/.manifest";
    /// Identity the cached parent-folder listing holds — potentially stale after a rewrite.
    const LISTED_ID: &str = "stale-listed-id";
    /// Identity a fresh backend stat returns.
    const FRESH_ID: &str = "fresh-stat-id";

    #[derive(Clone)]
    struct CountingInner {
        stat_count: Arc<AtomicUsize>,
        list_stat_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl FileObject for CountingInner {
        fn inner(&self) -> Option<&dyn FileObject> {
            None
        }

        async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
            self.stat_count.fetch_add(1, Ordering::SeqCst);
            Ok(StatResponse {
                resource_info: Some(ResourceInfo {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: FRESH_ID.to_string(),
                    }),
                    metadata: Some(Metadata {
                        data_object_size: Some(1),
                        last_modified_timestamp: None,
                    }),
                }),
            })
        }
    }

    #[async_trait]
    impl FileFolder for CountingInner {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        async fn list_stat(
            &self,
            _request: Request<ListStatRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
            self.list_stat_count.fetch_add(1, Ordering::SeqCst);
            let response = ListStatResponseWithContinueToken {
                response: ListStatResponse {
                    subfolder_addresses: Vec::new(),
                    entries: vec![ListItem {
                        resource_address: OBJECT.to_string(),
                        resource_info: Some(ResourceInfo {
                            resource_identity: Some(ResourceIdentity {
                                encoded_identity: LISTED_ID.to_string(),
                            }),
                            metadata: Some(Metadata {
                                data_object_size: Some(1),
                                last_modified_timestamp: None,
                            }),
                        }),
                    }],
                },
                continue_token: None,
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
        }
    }

    fn stat_request(no_cache: bool) -> Request<StatRequest> {
        let mut request = Request::new(StatRequest {
            resource_address: OBJECT.to_string(),
        });
        if no_cache {
            request.extensions_mut().insert(CacheControl::NoCache);
        }
        request
    }

    fn identity(response: &StatResponse) -> &str {
        response
            .resource_info
            .as_ref()
            .and_then(|info| info.resource_identity.as_ref())
            .map(|id| id.encoded_identity.as_str())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn nocache_extension_bypasses_list_cache_stat() {
        let stat_count = Arc::new(AtomicUsize::new(0));
        let list_stat_count = Arc::new(AtomicUsize::new(0));
        let service = ListCacheLayer::new(Config::default(), None, None)
            .await
            .expect("build")
            .expect("enabled")
            .layer(CountingInner {
                stat_count: stat_count.clone(),
                list_stat_count: list_stat_count.clone(),
            });

        // Without NoCache, the stat is answered from the cached parent-folder listing: the inner is
        // listed once, never statted, and the (potentially stale) listed identity is returned.
        let response = service.stat(stat_request(false)).await.expect("stat");
        assert_eq!(identity(&response), LISTED_ID);
        assert_eq!(list_stat_count.load(Ordering::SeqCst), 1);
        assert_eq!(stat_count.load(Ordering::SeqCst), 0);

        // With NoCache, the list cache is bypassed: the request delegates to the inner stat, which
        // returns the fresh identity, and no additional listing happens.
        let response = service.stat(stat_request(true)).await.expect("stat");
        assert_eq!(identity(&response), FRESH_ID);
        assert_eq!(stat_count.load(Ordering::SeqCst), 1);
        assert_eq!(list_stat_count.load(Ordering::SeqCst), 1);
    }
}
