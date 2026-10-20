// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use futures::StreamExt;
use tracing::instrument;

use crate::small_object_cache::cache::{
    canonical_backend_identity, canonical_identity, insert_cache_entry, meta_key, SmallObjectCache,
    SmallObjectCacheEntry,
};
use crate::small_object_cache::error::Error;
use storage_core_builder::traits::{
    error::{Result, StorageError},
    filefolder::v1alpha::{FileFolder, ListItem as ListFolderItem, ListStatRequest, ListStatResponseWithContinueToken},
    fileobject::v1alpha::{
        read_from_address_response, read_response, Chunk, DownloadPreference, FileObject, ReadFromAddressRequest,
        ReadFromAddressResponse, ReadRequest, ReadResponse, ResourceInfo as ResourceInfoV1Alpha, StatRequest,
        StatResponse,
    },
    BoxStream, Request, StreamingOptions,
};
use storage_core_util::api_types::{Header, ReadPart, Redirect, ResourceInfo};
use storage_core_util::cache::ApiVersion;
use storage_core_util::cache::CacheControl;

#[async_trait]
impl<S> FileObject for SmallObjectCache<S>
where
    S: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    #[instrument(name = "small_object_cache::stat", skip_all)]
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.stat(request).await;
        }
        let generation = self.cache.generation_snapshot();
        let response = self.inner.stat(request).await?;
        if let Some(entry) = stat_response_to_metadata_entry(&response) {
            let key = meta_key(&entry.resource_info.encoded_identity, ApiVersion::V1Alpha);
            insert_cache_entry(&self.cache, &self.l2_write_semaphore, key, entry, generation).await;
        }
        Ok(response)
    }

    #[instrument(name = "small_object_cache::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        // NoCache: never serve from or populate the body cache for this request.
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            return self.inner.read_from_address(request).await;
        }
        // If we have a cached resource info from the stat call, then we can use it to read from the cache
        // Perhaps we could also perform the stat here.
        if let Some(cached_resource_info) = request.extensions().get::<ResourceInfoV1Alpha>() {
            let resource_identity = canonical_backend_identity(
                &cached_resource_info
                    .resource_identity
                    .as_ref()
                    .ok_or_else(|| Error::invalid_argument("Resource info has no resource identity"))?
                    .encoded_identity,
            )?;
            let download_preference = *request
                .get_ref()
                .download_preference
                .as_ref()
                .unwrap_or(&DownloadPreference::Unspecified.into());
            let inner = self.inner.clone();
            let internal_stream = self
                .read_internal(
                    resource_identity.clone(),
                    ApiVersion::V1Alpha,
                    download_preference,
                    |_hint| async move {
                        // The caller already injected ResourceInfo into extensions (checked above),
                        // so any cache hint from small_object_cache's own identity-keyed cache is
                        // redundant here. Ignore it.
                        let s = inner.read_from_address(request).await?;
                        let read_part_stream = s.map(|r| r.and_then(read_from_address_response_to_read_part));
                        Ok(Box::pin(read_part_stream) as BoxStream<ReadPart>)
                    },
                )
                .await?;

            let stream = internal_stream.map(move |result| {
                result.map(|read_part| match read_part {
                    ReadPart::ResourceInfo(resource_info) => ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(
                            resource_info.into(),
                        )),
                    },
                    ReadPart::Chunk(chunk) => ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Chunk(Chunk { chunk })),
                    },
                    ReadPart::Redirect(redirect) => ReadFromAddressResponse {
                        reply_type: Some(read_from_address_response::ReplyType::Redirect(redirect.into())),
                    },
                })
            });

            Ok(Box::pin(stream))
        } else {
            self.inner.read_from_address(request).await
        }
    }

    #[instrument(name = "small_object_cache::read", skip_all)]
    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let resource_identity = canonical_identity(
            &request
                .get_ref()
                .resource_identity
                .as_ref()
                .ok_or_else(|| Error::invalid_argument("Resource identity is required"))?
                .encoded_identity,
        )?;

        let download_preference = *request
            .get_ref()
            .download_preference
            .as_ref()
            .unwrap_or(&DownloadPreference::Unspecified.into());

        let inner = self.inner.clone();
        let internal_stream = self
            .read_internal(
                resource_identity.clone(),
                ApiVersion::V1Alpha,
                download_preference,
                |hint| async move {
                    let id = resource_identity.clone();
                    let mut request = request;
                    if let Some(hint) = hint {
                        request.extensions_mut().insert(ResourceInfoV1Alpha::from(hint));
                    }
                    let s = inner.read(request).await?;
                    let read_part_stream =
                        s.map(move |r| r.and_then(|response| read_response_to_read_part(id.clone(), response)));
                    Ok(Box::pin(read_part_stream) as BoxStream<ReadPart>)
                },
            )
            .await?;

        let stream = internal_stream.map(|result| {
            result.map(|read_part| match read_part {
                ReadPart::ResourceInfo(resource_info) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Metadata(resource_info.into())),
                },
                ReadPart::Chunk(chunk) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Chunk(Chunk { chunk })),
                },
                ReadPart::Redirect(redirect) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Redirect(redirect.into())),
                },
            })
        });

        Ok(Box::pin(stream))
    }
}

#[allow(deprecated)]
fn read_response_to_read_part(
    resource_identity: String,
    val: ReadResponse,
) -> std::result::Result<ReadPart, StorageError> {
    match val.reply_type {
        Some(read_response::ReplyType::Metadata(metadata)) => Ok(ReadPart::ResourceInfo(ResourceInfo {
            native_metadata: metadata.native_metadata,
            encoded_identity: resource_identity,
            data_object_size: metadata.data_object_size,
            last_modified_timestamp: metadata.last_modified_timestamp,
        })),
        Some(read_response::ReplyType::Chunk(chunk)) => Ok(ReadPart::Chunk(chunk.chunk)),
        Some(read_response::ReplyType::Redirect(redirect)) => Ok(ReadPart::Redirect(Redirect {
            redirect_target_url: redirect.redirect_target_url,
            method: redirect.method,
            additional_headers: redirect
                .additional_headers
                .into_iter()
                .map(|h| Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
        })),
        None => Err(Error::internal("No reply type in ReadResponse").into()),
    }
}

#[allow(deprecated)]
fn read_from_address_response_to_read_part(
    val: ReadFromAddressResponse,
) -> std::result::Result<ReadPart, StorageError> {
    match val.reply_type {
        Some(read_from_address_response::ReplyType::ResourceInfo(resource_info)) => {
            let resource_identity = resource_info
                .resource_identity
                .ok_or_else(|| Error::internal("Resource info has no resource identity"))?;
            let metadata = resource_info
                .metadata
                .ok_or_else(|| Error::internal("Resource info has no metadata"))?;
            Ok(ReadPart::ResourceInfo(ResourceInfo {
                native_metadata: metadata.native_metadata,
                encoded_identity: canonical_backend_identity(&resource_identity.encoded_identity)?,
                data_object_size: metadata.data_object_size,
                last_modified_timestamp: metadata.last_modified_timestamp,
            }))
        },
        Some(read_from_address_response::ReplyType::Chunk(chunk)) => Ok(ReadPart::Chunk(chunk.chunk)),
        Some(read_from_address_response::ReplyType::Redirect(redirect)) => Ok(ReadPart::Redirect(Redirect {
            redirect_target_url: redirect.redirect_target_url,
            method: redirect.method,
            additional_headers: redirect
                .additional_headers
                .into_iter()
                .map(|h| Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
        })),
        None => Err(Error::internal("No reply type in ReadFromAddressResponse").into()),
    }
}

fn stat_response_to_metadata_entry(response: &StatResponse) -> Option<SmallObjectCacheEntry> {
    let resource_info = response.resource_info.as_ref()?;
    let identity = resource_info.resource_identity.as_ref()?;
    if identity.encoded_identity.is_empty() {
        return None;
    }
    // A malformed backend identity must not turn a successful stat into an
    // error. Skip this optional cache population and return the backend reply.
    let encoded = canonical_backend_identity(&identity.encoded_identity).ok()?;
    Some(SmallObjectCacheEntry {
        bytes: None,
        resource_info: ResourceInfo {
            // Carry native metadata so cache hits match the backend stat (already gated at the source).
            native_metadata: resource_info
                .metadata
                .as_ref()
                .map(|m| m.native_metadata.clone())
                .unwrap_or_default(),
            encoded_identity: encoded,
            data_object_size: resource_info.metadata.as_ref().and_then(|m| m.data_object_size),
            last_modified_timestamp: resource_info.metadata.as_ref().and_then(|m| m.last_modified_timestamp),
        },
    })
}

#[async_trait]
impl<S> FileFolder for SmallObjectCache<S>
where
    S: FileFolder,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    #[instrument(name = "small_object_cache::list_stat", skip_all)]
    async fn list_stat(
        &self,
        mut request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        // Ask the backend for the complete record, the way `list_cache` fills its own listing
        // (`list_cache/v1alpha/mod.rs`). S3 gates its per-object fan-out on the *caller's*
        // `include_native_metadata`, so forwarding the flag untouched lets a caller that leaves it unset
        // (the proto default) publish empty-map hints that `read` would then trust as a completed stat.
        // Free when native metadata is disabled: the backend still gates the actual fetch.
        request.get_mut().include_native_metadata = true;
        let cache = self.cache.clone();
        let l2_write_semaphore = self.l2_write_semaphore.clone();
        let generation = self.cache.generation_snapshot();
        let stream = self.inner.list_stat(request, streaming).await?;
        let stream = stream.inspect(move |item| {
            let Ok(token) = item else {
                return;
            };
            for entry in &token.response.entries {
                if let Some(cache_entry) = list_item_to_metadata_entry(entry) {
                    let key = meta_key(&cache_entry.resource_info.encoded_identity, ApiVersion::V1Alpha);
                    let cache = cache.clone();
                    let l2_write_semaphore = l2_write_semaphore.clone();
                    tokio::spawn(async move {
                        insert_cache_entry(&cache, &l2_write_semaphore, key, cache_entry, generation).await;
                    });
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

// Build a small-object metadata hint from a listing entry. `list_stat` above always asks the backend for
// native metadata, so the entry carries whatever the operator's configuration makes available (Azure inline;
// S3 via the fan-out, which `Config::validate_native_metadata_cache_consistency` requires to be on whenever
// this cache is enabled alongside `native_metadata_enabled`). Carry that map into the hint: a read served
// from the hint then surfaces exactly what a direct stat/read would. The map is empty only when native
// metadata is disabled for the backend, in which case a direct stat/read returns an empty map too.
fn list_item_to_metadata_entry(entry: &ListFolderItem) -> Option<SmallObjectCacheEntry> {
    let resource_info = entry.resource_info.as_ref()?;
    let identity = resource_info.resource_identity.as_ref()?;
    if identity.encoded_identity.is_empty() {
        return None;
    }
    let encoded = canonical_backend_identity(&identity.encoded_identity).ok()?;
    Some(SmallObjectCacheEntry {
        bytes: None,
        resource_info: ResourceInfo {
            native_metadata: resource_info
                .metadata
                .as_ref()
                .map(|m| m.native_metadata.clone())
                .unwrap_or_default(),
            encoded_identity: encoded,
            data_object_size: resource_info.metadata.as_ref().and_then(|m| m.data_object_size),
            last_modified_timestamp: resource_info.metadata.as_ref().and_then(|m| m.last_modified_timestamp),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use storage_core_builder::{
        layer::Layer,
        traits::{
            filefolder::v1alpha::{FolderAddress, ListStatResponse},
            fileobject::v1alpha::{Metadata, ResourceIdentity},
        },
    };
    use storage_core_protos::google::protobuf::Timestamp;

    use super::*;
    use crate::small_object_cache::{Config, SmallObjectCacheLayer};

    const BACKEND_IDENTITY: &str = "https://example.com/v1alpha-metadata()";
    const IDENTITY: &str = "https://example.com/v1alpha-metadata%28%29";

    #[derive(Clone)]
    struct MetadataBackend;

    #[async_trait]
    impl FileObject for MetadataBackend {
        fn inner(&self) -> Option<&dyn FileObject> {
            None
        }

        async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
            Ok(StatResponse {
                resource_info: Some(resource_info()),
            })
        }
    }

    #[async_trait]
    impl FileFolder for MetadataBackend {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        async fn list_stat(
            &self,
            request: Request<ListStatRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
            // Mirror the S3 backend: the per-object native-metadata fan-out only runs when the request
            // that reaches the backend asks for it (`s3/filefolder/v1alpha.rs`, `want_fanout`).
            let mut info = resource_info();
            if request.get_ref().include_native_metadata {
                info.metadata.as_mut().unwrap().native_metadata =
                    [(NATIVE_KEY.to_string(), native_value())].into_iter().collect();
            }
            Ok(Box::pin(futures::stream::iter([Ok(
                ListStatResponseWithContinueToken {
                    response: ListStatResponse {
                        subfolder_addresses: Vec::new(),
                        entries: vec![ListFolderItem {
                            resource_address: "s3://bucket/object".to_string(),
                            resource_info: Some(info),
                        }],
                    },
                    continue_token: None,
                },
            )])))
        }
    }

    const NATIVE_KEY: &str = "assetid";
    const NATIVE_VALUE: &str = "abc-123";

    fn native_value() -> storage_core_protos::google::protobuf::Value {
        storage_core_protos::google::protobuf::Value {
            kind: Some(storage_core_protos::google::protobuf::value::Kind::StringValue(
                NATIVE_VALUE.to_string(),
            )),
        }
    }

    fn resource_info() -> ResourceInfoV1Alpha {
        ResourceInfoV1Alpha {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: BACKEND_IDENTITY.to_string(),
            }),
            metadata: Some(Metadata {
                native_metadata: Default::default(),
                data_object_size: Some(17),
                last_modified_timestamp: Some(Timestamp { seconds: 23, nanos: 0 }),
            }),
        }
    }

    async fn service(
        namespace: &'static str,
    ) -> (
        SmallObjectCache<MetadataBackend>,
        Arc<storage_core_util::cache::LayerCache<SmallObjectCacheEntry>>,
    ) {
        let layer = SmallObjectCacheLayer::new(Config::default(), None, Some(namespace))
            .await
            .expect("small-object cache layer should build")
            .expect("small-object cache should be enabled");
        let cache = layer.cache.clone();
        (layer.layer(MetadataBackend), cache)
    }

    async fn await_metadata_entry(
        cache: &storage_core_util::cache::LayerCache<SmallObjectCacheEntry>,
    ) -> SmallObjectCacheEntry {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(entry) = cache.get(&meta_key(IDENTITY, ApiVersion::V1Alpha)).await {
                    break entry.expect("metadata entry should not be an error");
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("metadata insert should complete")
    }

    fn assert_metadata_only(entry: SmallObjectCacheEntry) {
        assert_eq!(entry.resource_info.encoded_identity, IDENTITY);
        assert_eq!(entry.resource_info.data_object_size, Some(17));
        assert_eq!(
            entry.resource_info.last_modified_timestamp,
            Some(Timestamp { seconds: 23, nanos: 0 })
        );
        assert!(
            entry.bytes.is_none(),
            "metadata publication must not include body bytes"
        );
    }

    #[test]
    fn invalid_stat_identity_is_not_cacheable() {
        let mut info = resource_info();
        info.resource_identity.as_mut().unwrap().encoded_identity = "not a URL".to_string();
        let response = StatResponse {
            resource_info: Some(info),
        };

        assert!(stat_response_to_metadata_entry(&response).is_none());
    }

    #[tokio::test]
    async fn stat_publishes_metadata_under_meta_key_only() {
        let (service, cache) = service("v1alpha_stat_metadata_key").await;

        FileObject::stat(
            &service,
            Request::new(StatRequest {
                resource_address: "s3://bucket/object".to_string(),
            }),
        )
        .await
        .expect("stat should succeed");

        assert!(
            cache.get(IDENTITY).await.is_none(),
            "stat must not publish under the raw identity"
        );
        assert_metadata_only(await_metadata_entry(&cache).await);
    }

    #[tokio::test]
    async fn list_stat_publishes_metadata_under_meta_key_only() {
        let (service, cache) = service("v1alpha_list_stat_metadata_key").await;

        let mut stream = FileFolder::list_stat(
            &service,
            Request::new(ListStatRequest {
                include_native_metadata: false,
                folder: Some(FolderAddress {
                    uri: "s3://bucket/".to_string(),
                }),
            }),
            StreamingOptions::Streamed,
        )
        .await
        .expect("list_stat should succeed");
        while let Some(item) = stream.next().await {
            item.expect("list_stat item should succeed");
        }

        let entry = await_metadata_entry(&cache).await;
        assert!(
            cache.get(IDENTITY).await.is_none(),
            "list_stat must not publish under the raw identity"
        );
        assert_metadata_only(entry);
    }

    // A caller that leaves `include_native_metadata` unset (the proto default) must not be able to publish
    // an empty-map hint: v1alpha `read` treats a published hint as a completed stat and answers from it, so
    // an incomplete hint would make ListStat-then-Read return `{}` where a direct Read returns the keys.
    #[tokio::test]
    async fn list_stat_publishes_complete_hint_when_caller_omits_native_metadata() {
        let (service, cache) = service("v1alpha_list_stat_hint_completeness").await;

        let mut stream = FileFolder::list_stat(
            &service,
            Request::new(ListStatRequest {
                include_native_metadata: false,
                folder: Some(FolderAddress {
                    uri: "s3://bucket/".to_string(),
                }),
            }),
            StreamingOptions::Streamed,
        )
        .await
        .expect("list_stat should succeed");
        while let Some(item) = stream.next().await {
            item.expect("list_stat item should succeed");
        }

        let entry = await_metadata_entry(&cache).await;
        assert_eq!(
            entry.resource_info.native_metadata.get(NATIVE_KEY),
            Some(&native_value()),
            "the published hint must carry native metadata even when the caller did not request it"
        );
    }
}
