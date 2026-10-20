// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use async_stream::try_stream;
use futures::{future::BoxFuture, StreamExt, TryStreamExt};
use get_size2::GetSize;
use indexmap::IndexMap;
use moka::policy::EvictionPolicy;
use prost::Message;
use snafu::IntoError;
use std::{iter::FusedIterator, sync::Arc};
use storage_core::layer::Layer;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{debug, error, instrument, trace};
use uuid::Uuid;

use crate::{
    backend::metadata::{
        cache::{
            error::{Error, InitError},
            types::*,
        },
        *,
    },
    common::{error::PrettyPrintErrorExt as _, request::RequestExt},
};

mod error;
mod l2;
mod metrics;
mod types;

pub mod config;

const DEFAULT_PAGE_SIZE: usize = 100;
const DEFAULT_CACHE_KEY_PREFIX: &str = "metadata-cache:";

#[derive(Debug)]
pub struct MetadataCache {
    inner: Arc<MetadataCacheInner>,
    drop_guard: Arc<DropGuard>,
}

impl MetadataCache {
    pub async fn try_from_config(
        config: config::Config,
        redis: Option<fred::clients::Pool>,
        redis_namespace_prefix: Option<&str>,
    ) -> Result<Option<Self>, InitError> {
        if !config.enabled {
            return Ok(None);
        }

        let cache_entry_ttl = config.cache_time_to_live.unwrap_or(config::DEFAULT_CACHE_TIME_TO_LIVE);

        let cache = moka::future::Cache::builder()
            .eviction_policy(EvictionPolicy::tiny_lfu())
            .time_to_live(cache_entry_ttl)
            .max_capacity(config.cache_capacity.unwrap_or(config::DEFAULT_CACHE_CAPACITY).as_u64())
            .weigher(|k: &AnyKey, v: &ClaimedCacheValue| (k.get_size() + v.get_size()) as u32)
            .build();

        let redis_cache = if let (Some(redis), true) = (redis, config.l2_enabled) {
            let key_prefix = redis_namespace_prefix
                .map(|prefix| format!("{prefix}:{DEFAULT_CACHE_KEY_PREFIX}"))
                .unwrap_or_else(|| DEFAULT_CACHE_KEY_PREFIX.into());

            Some(l2::Cache::new(
                redis,
                key_prefix,
                cache_entry_ttl,
                config
                    .l2_claim_time_to_live
                    .unwrap_or(config::DEFAULT_L2_CLAIM_TIME_TO_LIVE),
            ))
        } else {
            None
        };

        let inner = Arc::new(MetadataCacheInner {
            metrics: metrics::Metrics::new(),
            page_size_byte_limit: config
                .page_size_limit
                .unwrap_or(config::DEFAULT_PAGE_SIZE_LIMIT)
                .as_u64() as usize,
            cache,
            redis_cache,
        });

        let cancel = CancellationToken::new();
        let drop_guard = cancel.clone().drop_guard();
        inner.clone().l2_subscribe_and_process_events(cancel).await?;

        Ok(Some(Self {
            drop_guard: Arc::new(drop_guard),
            inner,
        }))
    }
}

impl<S> Layer<S> for MetadataCache {
    type Service = MetadataCacheService<S>;

    fn layer(&self, service: S) -> Self::Service {
        MetadataCacheService {
            inner: self.inner.clone(),
            _drop_guard: self.drop_guard.clone(),
            service,
        }
    }
}

#[derive(Debug)]
pub struct MetadataCacheService<S> {
    service: S,
    inner: Arc<MetadataCacheInner>,
    _drop_guard: Arc<DropGuard>,
}

#[async_trait::async_trait]
impl<S: Metadata> Metadata for MetadataCacheService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        let response = self
            .inner
            .get_metadata_batch_through_cache(
                request.requests.iter().map(|request| {
                    let resource_location = create_resource_location_ref(request.resource_location.as_ref())?;
                    Ok(ExpandedPointReadKeyRef {
                        resource_location,
                        metadata_key: &request.user_metadata_key,
                        namespace: Namespace::from_option(request.namespace.as_deref()),
                    })
                }),
                |request| {
                    self.service
                        .get_metadata_batch(Request::from_parts(metadata, extensions, request))
                },
            )
            .await?;

        Ok(response)
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let (metadata, extensions, request) = request.into_parts();
        let stream = self
            .inner
            .query_metadata_through_cache(
                ExpandedQueryKeyRef {
                    resource_location: create_resource_location_ref(request.resource_location.as_ref())?,
                    namespace: Namespace::from_option(request.namespace.as_deref()),
                },
                options,
                |request, options| {
                    self.service
                        .query_metadata(Request::from_parts(metadata, extensions, request), options)
                },
            )
            .await?;

        Ok(stream)
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        use update_metadata_batch_response_entry::Outcome;

        let keys_to_invalidate: Vec<_> = request
            .get_ref()
            .requests
            .iter()
            .map(|request| {
                (
                    request.resource_location.clone(),
                    request.user_metadata_key.clone(),
                    Namespace::from_option(request.namespace.as_deref()).into_owned(),
                )
            })
            .collect();
        let response = self.service.update_metadata_batch(request).await?;
        debug_assert_eq!(response.results.len(), keys_to_invalidate.len());

        self.inner
            .l1_l2_invalidate_and_update_metrics(response.results.iter().filter_map(|result| {
                debug_assert!(result.outcome.is_some());
                debug_assert!(keys_to_invalidate.len() > result.request_index as usize);

                if matches!(result.outcome, Some(Outcome::Success(_)))
                    || matches!(result.outcome, Some(Outcome::Error(ref e)) if batch_error_invalidates_cache(e))
                {
                    let (resource_location, metadata_key, namespace) =
                        &keys_to_invalidate[result.request_index as usize];
                    Some(PointReadKeyRef {
                        resource_uri: get_resource_uri_from_resource_location(resource_location.as_ref())
                            .unwrap_or_default(),
                        metadata_key,
                        namespace: namespace.to_ref(),
                    })
                } else {
                    None
                }
            }))
            .await;

        Ok(response)
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        use delete_metadata_batch_response_entry::Outcome;

        let keys_to_invalidate: Vec<_> = request
            .get_ref()
            .requests
            .iter()
            .map(|request| {
                (
                    request.resource_location.clone(),
                    request.user_metadata_key.clone(),
                    Namespace::from_option(request.namespace.as_deref()).into_owned(),
                )
            })
            .collect();
        let response = self.service.delete_metadata_batch(request).await?;

        debug_assert_eq!(response.results.len(), keys_to_invalidate.len());

        self.inner
            .l1_l2_invalidate_and_update_metrics(response.results.iter().filter_map(|result| {
                debug_assert!(result.outcome.is_some());
                debug_assert!(keys_to_invalidate.len() > result.request_index as usize);

                if matches!(result.outcome, Some(Outcome::Success(_)))
                    || matches!(result.outcome, Some(Outcome::Error(ref e)) if batch_error_invalidates_cache(e))
                {
                    let (resource_location, metadata_key, namespace) =
                        &keys_to_invalidate[result.request_index as usize];
                    Some(PointReadKeyRef {
                        resource_uri: get_resource_uri_from_resource_location(resource_location.as_ref())
                            .unwrap_or_default(),
                        metadata_key,
                        namespace: namespace.to_ref(),
                    })
                } else {
                    None
                }
            }))
            .await;

        Ok(response)
    }
}

#[derive(Debug)]
struct MetadataCacheInner {
    metrics: metrics::Metrics,
    cache: moka::future::Cache<AnyKey, ClaimedCacheValue>,
    page_size_byte_limit: usize,
    redis_cache: Option<l2::Cache>,
}

impl MetadataCacheInner {
    async fn query_metadata_through_cache<'a, F>(
        &self,
        key: ExpandedQueryKeyRef<'_>,
        options: StreamingOptions,
        make_request: F,
    ) -> Result<BoxStream<QueryMetadataResponsePage>>
    where
        F: FnOnce(
                QueryMetadataRequest,
                StreamingOptions,
            ) -> BoxFuture<'a, Result<BoxStream<QueryMetadataResponsePage>>>
            + 'a,
    {
        debug!(?key, "querying cached metadata");

        // Satisfy the query from the L1 cache.
        let l1_token = match self.l1_fetch_or_claim(AnyKeyRef::Query(key.to_query_key_ref())).await {
            ValueOrToken::Value(value) => match value {
                CacheValue::Query(values) => {
                    trace!(?key, "value resolved from l1");

                    return Ok(self.stream_cached_metadata_with_options(values, options));
                },
                value => {
                    error!(?key, ?value, "unexpected cached value under a query key");
                    ClaimToken::default()
                },
            },
            ValueOrToken::Token(token) => {
                trace!(?key, ?token, "key not found in l1");
                token
            },
        };

        // Satisfy the query from the L2 cache.
        let maybe_l2_token = if let Some(ref cache) = self.redis_cache {
            let maybe_values = cache
                .read_query_value(key.to_query_key_ref())
                .await
                .inspect_err(|error| error!(error = %error.pretty(), "redis cache read failed"));

            match maybe_values {
                Ok(ValueOrToken::Value(values)) => {
                    trace!(?key, "value resolved from l2");

                    let values = Arc::new(values);
                    self.l1_write(AnyKey::Query(key.to_query_key_ref().to_owned()), l1_token, || {
                        CacheValue::Query(values.clone())
                    })
                    .await;

                    self.update_cache_size_metrics();

                    return Ok(self.stream_cached_metadata_with_options(values, options));
                },
                Ok(ValueOrToken::Token(token)) => {
                    trace!(?key, ?token, "key not found in l2");
                    Some(token)
                },
                _ => None,
            }
        } else {
            None
        };

        // Query everything.
        let values = Arc::new(
            self.query_all_metadata(
                key.resource_location.to_owned(),
                key.namespace.into_owned(),
                make_request,
            )
            .await?,
        );

        // Populate the L1 cache.
        self.l1_write(AnyKey::Query(key.to_query_key_ref().to_owned()), l1_token, || {
            CacheValue::Query(values.clone())
        })
        .await;

        self.update_cache_size_metrics();

        // Populate the L2 cache.
        if let (Some(cache), Some(token)) = (self.redis_cache.as_ref(), maybe_l2_token) {
            let _ = cache
                .write_query_value(key.to_query_key_ref(), &values, token)
                .await
                .inspect_err(|error| error!(error = %error.pretty(), "redis cache write failed"));
        }

        Ok(self.stream_cached_metadata_with_options(values, options))
    }

    #[instrument(skip_all, fields(resource_location))]
    async fn query_all_metadata<'a, F>(
        &self,
        resource_location: ResourceLocation,
        namespace: Namespace<String>,
        make_request: F,
    ) -> Result<IndexMap<String, UserMetadataValue>>
    where
        F: FnOnce(
                QueryMetadataRequest,
                StreamingOptions,
            ) -> BoxFuture<'a, Result<BoxStream<QueryMetadataResponsePage>>>
            + 'a,
    {
        debug!(?resource_location, "querying all metadata");

        let mut results = IndexMap::new();
        let mut stream = make_request(
            QueryMetadataRequest {
                resource_location: Some(resource_location),
                namespace: match namespace {
                    Namespace::Default => None,
                    Namespace::Explicit(namespace) => {
                        debug_assert_ne!(namespace, DEFAULT_NAMESPACE_NAME);
                        Some(namespace)
                    },
                },
            },
            StreamingOptions::Streamed,
        )
        .await?;
        while let Some(page) = stream.try_next().await? {
            results.extend(page.response.results.into_iter().filter_map(|e| {
                debug_assert!(e.user_metadata.is_some());
                e.user_metadata.map(|value| (e.user_metadata_key, value))
            }));
        }

        results.sort_unstable_keys();

        Ok(results)
    }

    fn stream_cached_metadata_with_options(
        &self,
        values: Arc<IndexMap<String, UserMetadataValue>>,
        options: StreamingOptions,
    ) -> futures::stream::BoxStream<'static, Result<QueryMetadataResponsePage>> {
        debug_assert!(values.is_sorted());

        let StreamingOptions::Paginated(params) = options else {
            return Box::pin(stream_cached_query_response_with_page_size_limit(
                values,
                self.page_size_byte_limit,
            ));
        };

        let offset = if let Some(last_key) = params.continuation_handle {
            match values.binary_search_keys(&last_key) {
                Ok(pos) => pos + 1,
                Err(pos) => pos,
            }
        } else {
            0usize
        };

        let results_len = match params.max_page_size {
            None | Some(0) => DEFAULT_PAGE_SIZE,
            Some(size) => size as usize,
        }
        .min(values.len() - offset);

        let (_, values_slice) = values.as_slice().split_at(offset);
        let results: Vec<_> = values_slice
            .iter()
            .map(|(key, value)| QueryMetadataResponseEntry {
                user_metadata_key: key.clone(),
                user_metadata: Some(value.clone()),
            })
            .take(results_len)
            .collect();

        let response = QueryMetadataResponsePage {
            next_continuation_handle: if values_slice.len() > results_len {
                results.last().map(|entry| entry.user_metadata_key.clone())
            } else {
                None
            },
            response: QueryMetadataResponse { results },
        };

        Box::pin(try_stream! { yield response; })
    }

    async fn get_metadata_batch_through_cache<'a, 'b, F>(
        &self,
        keys: impl Iterator<Item = Result<ExpandedPointReadKeyRef<'a>, Error>> + 'a,
        make_request: F,
    ) -> Result<GetMetadataBatchResponse>
    where
        F: FnOnce(GetMetadataBatchRequest) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> + 'b,
    {
        use batch_error_entry::Code;
        use get_metadata_batch_response_entry::Outcome;

        #[derive(Debug)]
        enum RequestStatus<'b> {
            /// Request has been rejected.
            Rejected(Error),
            /// User metadata value has been resolved.
            Resolved(UserMetadataValue),
            /// User metadata value has been resolved as not found.
            ResolvedNotFound,
            /// User metadata value has not been resolved. Use the writeback token to populate
            /// the L1 cache results.
            NotResolved(ExpandedPointReadKeyRef<'b>, ClaimToken),
            /// User metadata value has not been resolved. Use the writeback tokens to populate
            /// the L1 and L2 cache results.
            NotResolvedL2(ExpandedPointReadKeyRef<'b>, ClaimToken, l2::ClaimToken),
        }

        let mut request_statuses = if let (_, Some(capacity)) = keys.size_hint() {
            Vec::with_capacity(capacity)
        } else {
            Vec::new()
        };

        // Resolve keys from the L1 cache.
        for maybe_key in keys {
            let key = match maybe_key {
                Ok(key) => key,
                Err(error) => {
                    request_statuses.push(RequestStatus::Rejected(error));
                    continue;
                },
            };

            let maybe_point_read_value = self
                .l1_fetch_or_claim(AnyKeyRef::PointRead(key.to_point_read_key_ref()))
                .await;
            let token = match maybe_point_read_value {
                ValueOrToken::Value(value) => match value {
                    CacheValue::PointReadNotFound => {
                        trace!(?key, "value resolved from l1");
                        request_statuses.push(RequestStatus::ResolvedNotFound);

                        continue;
                    },
                    CacheValue::PointReadFound(v) => {
                        trace!(?key, "absent resolved from l1");
                        request_statuses.push(RequestStatus::Resolved(v.as_ref().clone()));

                        continue;
                    },
                    value => {
                        error!(?key, ?value, "unexpected cached value under a point-read key");
                        ClaimToken::default()
                    },
                },
                ValueOrToken::Token(token) => {
                    trace!(?key, ?token, "key not found in l1");
                    token
                },
            };

            let maybe_query_value = self
                .l1_fetch(AnyKeyRef::Query(QueryKeyRef {
                    resource_uri: key.resource_location.resource_uri(),
                    namespace: key.namespace,
                }))
                .await;
            if let Some(CacheValue::Query(query_value)) = maybe_query_value {
                if let Some(value) = query_value.get(key.metadata_key) {
                    request_statuses.push(RequestStatus::Resolved(value.clone()));
                } else {
                    request_statuses.push(RequestStatus::ResolvedNotFound);
                }
                continue;
            }

            request_statuses.push(RequestStatus::NotResolved(key, token));
        }

        // Resolve keys from the L2 cache, if possible.
        if let Some(ref cache) = self.redis_cache {
            let maybe_read_results = cache
                .read_point_read_values(request_statuses.iter().filter_map(|status| {
                    debug_assert!(matches!(
                        status,
                        RequestStatus::Rejected(_)
                            | RequestStatus::NotResolved(_, _)
                            | RequestStatus::Resolved(_)
                            | RequestStatus::ResolvedNotFound
                    ));

                    if let RequestStatus::NotResolved(key, _token) = status {
                        Some(key.to_point_read_key_ref())
                    } else {
                        None
                    }
                }))
                .await
                .inspect_err(|error| error!(error = %error.pretty(), "redis cache lookup failed"));

            if let Ok(read_results) = maybe_read_results {
                for (request_status, read_result) in request_statuses
                    .iter_mut()
                    .filter(|status| matches!(status, RequestStatus::NotResolved(_, _)))
                    .zip(read_results)
                {
                    let RequestStatus::NotResolved(key, l1_token) = request_status else {
                        unreachable!("unexpected request status");
                    };

                    match read_result {
                        ValueOrToken::Value(value) => match value {
                            PointReadValue::Missing => {
                                trace!(?key, "absent resolved from l2");
                                self.l1_write(
                                    AnyKey::PointRead(key.to_point_read_key_ref().to_owned()),
                                    *l1_token,
                                    || CacheValue::PointReadNotFound,
                                )
                                .await;

                                *request_status = RequestStatus::ResolvedNotFound;
                            },
                            PointReadValue::Value(value) => {
                                trace!(?key, "value resolved from l2");
                                self.l1_write(
                                    AnyKey::PointRead(key.to_point_read_key_ref().to_owned()),
                                    *l1_token,
                                    || CacheValue::PointReadFound(Arc::new(value.clone())),
                                )
                                .await;

                                *request_status = RequestStatus::Resolved(value);
                            },
                        },
                        ValueOrToken::Token(l2_token) => {
                            trace!(?key, ?l2_token, "key not found in l2");
                            *request_status = RequestStatus::NotResolvedL2(*key, *l1_token, l2_token)
                        },
                    };
                }
            }
        }

        // todo: we could precompute the size of the response_map in advance.

        let mut response_map = Vec::new();
        for (request_index, request_status) in request_statuses.iter().enumerate() {
            if matches!(
                request_status,
                RequestStatus::NotResolved(_, _) | RequestStatus::NotResolvedL2(_, _, _)
            ) {
                response_map.push(request_index as u32);
            }
        }

        // Exit early if we don't have any pending requests.
        if response_map.is_empty() {
            debug!("all keys resolved through the cache");

            return Ok(GetMetadataBatchResponse {
                results: request_statuses
                    .into_iter()
                    .enumerate()
                    .map(|(request_index, status)| {
                        let outcome = match status {
                            RequestStatus::Resolved(value) => Outcome::Success(value),
                            RequestStatus::ResolvedNotFound => Outcome::Error(create_not_found_batch_error_entry()),
                            s => panic!("unexpected request status {s:?}"),
                        };

                        GetMetadataBatchResponseEntry {
                            request_index: request_index as u32,
                            outcome: Some(outcome),
                        }
                    })
                    .collect(),
            });
        }

        // todo: at least, we could have returned cached errors.

        let mut response = make_request(GetMetadataBatchRequest {
            requests: request_statuses
                .iter()
                .filter_map(|status| {
                    let key = match status {
                        RequestStatus::NotResolved(key, _) => key,
                        RequestStatus::NotResolvedL2(key, _, _) => key,
                        _ => return None,
                    };

                    Some(GetMetadataBatchRequestEntry {
                        resource_location: Some(key.resource_location.to_owned()),
                        namespace: match key.namespace {
                            Namespace::Default => None,
                            Namespace::Explicit(value) => Some(value.into()),
                        },
                        user_metadata_key: key.metadata_key.into(),
                    })
                })
                .collect(),
        })
        .await?;

        for result in response.results.iter_mut() {
            debug_assert!(result.outcome.is_some());
            debug_assert!((result.request_index as usize) < response_map.len());

            result.request_index = response_map[result.request_index as usize];
        }

        // Populate the L1 cache.
        for result in response.results.iter() {
            debug_assert!((result.request_index as usize) < request_statuses.len());
            debug_assert!(result.outcome.is_some());

            let (key, token) = match &request_statuses[result.request_index as usize] {
                RequestStatus::NotResolved(key, token) => (key, token),
                RequestStatus::NotResolvedL2(key, token, _) => (key, token),
                _ => continue,
            };

            if let Some(Outcome::Success(ref value)) = result.outcome {
                self.l1_write(
                    AnyKey::PointRead(key.to_point_read_key_ref().to_owned()),
                    *token,
                    || CacheValue::PointReadFound(Arc::new(value.clone())),
                )
                .await;
            } else if matches!(result.outcome, Some(Outcome::Error(ref e)) if e.code == Code::NotFound as i32) {
                self.l1_write(
                    AnyKey::PointRead(key.to_point_read_key_ref().to_owned()),
                    *token,
                    || CacheValue::PointReadNotFound,
                )
                .await;
            }
        }

        self.update_cache_size_metrics();

        // Populate the L2 cache, if needed.
        if let Some(ref cache) = self.redis_cache {
            let _ = cache
                .write_point_read_values(response.results.iter().filter_map(|result| {
                    debug_assert!((result.request_index as usize) < request_statuses.len());
                    debug_assert!(result.outcome.is_some());

                    if let RequestStatus::NotResolvedL2(key, _, token) = &request_statuses[result.request_index as usize] {
                        if let Some(Outcome::Success(ref value)) = result.outcome {
                            Some((key.to_point_read_key_ref(), PointReadValueRef::Value(value), *token))
                        } else if matches!(result.outcome, Some(Outcome::Error(ref e)) if e.code == Code::NotFound as i32) {
                            Some((key.to_point_read_key_ref(), PointReadValueRef::Missing, *token))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }))
                .await
                .inspect_err(|error| error!(error = %error.pretty(), "redis cache write failed"));
        }

        // Update the response with cached values.
        response.results.extend(
            request_statuses
                .into_iter()
                .enumerate()
                .filter_map(|(request_index, status)| {
                    let outcome = match status {
                        RequestStatus::Resolved(value) => Outcome::Success(value),
                        RequestStatus::ResolvedNotFound => Outcome::Error(create_not_found_batch_error_entry()),
                        RequestStatus::Rejected(error) => Outcome::Error(create_batch_error_entry_from_error(&error)),
                        _ => return None,
                    };

                    Some(GetMetadataBatchResponseEntry {
                        request_index: request_index as u32,
                        outcome: Some(outcome),
                    })
                }),
        );

        Ok(response)
    }

    /// Invalidate cache entries in the L1 and L2 caches and update cache metrics.
    async fn l1_l2_invalidate_and_update_metrics<'a>(
        &self,
        items: impl Iterator<Item = PointReadKeyRef<'a>> + Clone + 'a,
    ) {
        if let Some(ref redis) = self.redis_cache {
            let _ = redis
                .invalidate_keys(items.clone().flat_map(|item| {
                    [
                        AnyKeyRef::Query(QueryKeyRef {
                            resource_uri: item.resource_uri,
                            namespace: item.namespace,
                        }),
                        AnyKeyRef::PointRead(item),
                    ]
                }))
                .await
                .inspect_err(|error| error!(error = %error.pretty(), "redis cache invalidate failed"));
        }

        for item in items {
            self.l1_invalidate_one(item.resource_uri, item.metadata_key, item.namespace)
                .await;
        }

        self.update_cache_size_metrics();
    }

    /// Read from the L1 cache and obtain a write slot on a miss.
    async fn l1_fetch_or_claim(&self, key: AnyKeyRef<'_>) -> ValueOrToken<CacheValue, ClaimToken> {
        use moka::ops::compute::Op;

        self.metrics.cache_access_count.add(1);

        if let Some(cache_value) = self.cache.get(&key).await {
            let result = cache_value.into_any_value();
            if matches!(result, ValueOrToken::Token(_)) {
                self.metrics.cache_miss_count.add(1);
            }

            return result;
        }

        // Claim our right to update the entry.
        let result = self
            .cache
            .entry(key.to_owned())
            .and_compute_with(|entry| async move {
                if entry.is_some() {
                    // We don't want to overwrite an already existing token because some calls can create
                    // ghost values that are never satisfied.
                    Op::Nop
                } else {
                    Op::Put(ClaimedCacheValue::Claimed(ClaimToken::new_random()))
                }
            })
            .await
            .unwrap()
            .into_value()
            .into_any_value();

        if matches!(result, ValueOrToken::Token(_)) {
            self.metrics.cache_miss_count.add(1);
        }

        result
    }

    /// Read from the L1 cache.
    async fn l1_fetch(&self, key: AnyKeyRef<'_>) -> Option<CacheValue> {
        self.metrics.cache_access_count.add(1);

        if let Some(cache_value) = self.cache.get(&key).await {
            if let ValueOrToken::Value(v) = cache_value.into_any_value() {
                return Some(v);
            }
        }

        self.metrics.cache_miss_count.add(1);

        None
    }

    /// Write back into the L1 cache if the claim slot is still valid.
    async fn l1_write(&self, key: AnyKey, expected_token: ClaimToken, with: impl FnOnce() -> CacheValue + Send + Sync) {
        use moka::ops::compute::Op;

        self.cache
            .entry(key)
            .and_compute_with(move |entry| async move {
                if let Some(entry) = entry {
                    if matches!(entry.into_value(), ClaimedCacheValue::Claimed(token) if token == expected_token) {
                        Op::Put(with().into_claimed_cache_value())
                    } else {
                        Op::Nop
                    }
                } else {
                    Op::Nop
                }
            })
            .await;
    }

    /// Invalidate a cache entry.
    async fn l1_invalidate_one(&self, resource_uri: &str, metadata_key: &str, namespace: Namespace<&str>) {
        trace!(?resource_uri, ?metadata_key, ?namespace, "invalidating cache entries");

        self.cache
            .invalidate(&AnyKeyRef::PointRead(PointReadKeyRef {
                resource_uri,
                metadata_key,
                namespace,
            }))
            .await;
        self.cache
            .invalidate(&AnyKeyRef::Query(QueryKeyRef {
                resource_uri,
                namespace,
            }))
            .await;
    }

    async fn l2_subscribe_and_process_events(self: Arc<Self>, cancel: CancellationToken) -> Result<(), InitError> {
        use error::init_error::Subscribe;

        if let Some(ref cache) = self.redis_cache {
            debug!("subscribing to l2 events");

            let mut stream = cache
                .event_stream()
                .await
                .map_err(|source| Subscribe.into_error(Box::new(source)))?;
            tokio::spawn(cancel.run_until_cancelled_owned(async move {
                while let Some(event) = stream.next().await {
                    self.l2_handle_event(event).await;
                    self.update_cache_size_metrics();
                }
            }));

            Ok(())
        } else {
            debug!("l2 cache is disabled, keyspace events won't be watched");

            Ok(())
        }
    }

    async fn l2_handle_event(&self, event: Event) {
        debug!(?event, "processing a keyspace event");

        let key_ref = match event {
            Event::PointReadKeyInvalidate(ref key) => AnyKeyRef::PointRead(key.to_ref()),
            Event::QueryKeyInvalidate(ref key) => AnyKeyRef::Query(key.to_ref()),
            Event::Reconnect => {
                debug!("reconnect event received, invalidating the entire l1 cache");
                self.cache.invalidate_all();

                return;
            },
        };

        trace!(?event, ?key_ref, "invalidating cache key");

        self.cache.invalidate(&key_ref).await;
    }

    fn update_cache_size_metrics(&self) {
        self.metrics.cache_size.record(self.cache.weighted_size());
        self.metrics.cache_entries.record(self.cache.entry_count());
    }
}

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq)]
struct ClaimToken(Uuid);

impl ClaimToken {
    fn new_random() -> Self {
        Self(Uuid::new_v4())
    }
}

impl get_size2::GetSize for ClaimToken {
    fn get_heap_size(&self) -> usize {
        0
    }
}

#[derive(Debug, Clone, get_size2::GetSize)]
enum ClaimedCacheValue {
    PointReadNotFound,
    PointReadFound(Arc<UserMetadataValue>),
    Query(Arc<QueryValue>),
    Claimed(ClaimToken),
}

impl ClaimedCacheValue {
    fn into_any_value(self) -> ValueOrToken<CacheValue, ClaimToken> {
        match self {
            Self::PointReadNotFound => ValueOrToken::Value(CacheValue::PointReadNotFound),
            Self::PointReadFound(v) => ValueOrToken::Value(CacheValue::PointReadFound(v)),
            Self::Query(v) => ValueOrToken::Value(CacheValue::Query(v)),
            Self::Claimed(t) => ValueOrToken::Token(t),
        }
    }
}

#[derive(Debug, Clone)]
enum CacheValue {
    PointReadNotFound,
    PointReadFound(Arc<UserMetadataValue>),
    Query(Arc<QueryValue>),
}

impl CacheValue {
    fn into_claimed_cache_value(self) -> ClaimedCacheValue {
        match self {
            Self::PointReadNotFound => ClaimedCacheValue::PointReadNotFound,
            Self::PointReadFound(v) => ClaimedCacheValue::PointReadFound(v),
            Self::Query(v) => ClaimedCacheValue::Query(v),
        }
    }
}

/// Apparently, the Rust compiler has some limitation that makes us invent unicorns.
struct QueryMetadataResponseEntryIter<'a> {
    inner: indexmap::map::Iter<'a, String, UserMetadataValue>,
}

impl Iterator for QueryMetadataResponseEntryIter<'_> {
    type Item = (usize, QueryMetadataResponseEntry);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((key, value)) = self.inner.next() {
            let entry = QueryMetadataResponseEntry {
                user_metadata_key: key.clone(),
                user_metadata: Some(value.clone()),
            };

            Some((entry.encoded_len(), entry))
        } else {
            None
        }
    }
}

impl FusedIterator for QueryMetadataResponseEntryIter<'_> {}

fn get_resource_uri_from_resource_location(value: Option<&ResourceLocation>) -> Option<&str> {
    use resource_location::Resource;

    match value?.resource.as_ref()? {
        Resource::ResourceAddress(value) => Some(value),
        Resource::ResourceIdentity(value) => Some(&value.encoded_identity),
        Resource::FolderAddress(value) => Some(&value.uri),
    }
}

fn batch_error_invalidates_cache(error: &BatchErrorEntry) -> bool {
    use batch_error_entry::Code;

    error.code != Code::InvalidArgument as i32
        && error.code != Code::PermissionDenied as i32
        && error.code != Code::FailedPrecondition as i32
}

fn stream_cached_query_response_with_page_size_limit(
    items: Arc<IndexMap<String, UserMetadataValue>>,
    page_size_limit: usize,
) -> BoxStream<QueryMetadataResponsePage> {
    Box::pin(try_stream! {
        let mut items = QueryMetadataResponseEntryIter{ inner: items.iter() }.peekable();
        'outer: while let Some((first_len, first_entry)) = items.next() {
            let mut results = vec![first_entry];
            let mut results_len = first_len;

            while let Some((next_len, _)) = items.peek() {
                if results_len + *next_len > page_size_limit {
                    yield QueryMetadataResponsePage {
                        response: QueryMetadataResponse { results },
                        next_continuation_handle: None,
                    };
                    continue 'outer;
                }

                let (next_len, next_entry) = items.next().unwrap();
                results.push(next_entry);
                results_len += next_len;
            }

            yield QueryMetadataResponsePage {
                response: QueryMetadataResponse { results },
                next_continuation_handle: None,
            };
            break;
        }
    })
}

fn create_batch_error_entry(code: batch_error_entry::Code, message: impl Into<String>) -> BatchErrorEntry {
    BatchErrorEntry {
        code: code as i32,
        message: message.into(),
    }
}

fn create_not_found_batch_error_entry() -> BatchErrorEntry {
    create_batch_error_entry(batch_error_entry::Code::NotFound, "metadata key not found")
}

fn create_batch_error_entry_from_error(error: &Error) -> BatchErrorEntry {
    use batch_error_entry::Code;

    match error {
        Error::ResourceLocationMissing { .. } => create_batch_error_entry(Code::InvalidArgument, error.to_string()),
    }
}

fn create_resource_location_ref<'a>(
    resource_location: Option<&'a ResourceLocation>,
) -> Result<ResourceLocationRef<'a>, Error> {
    use error::error::ResourceLocationMissing;
    use resource_location::Resource;

    let result = match resource_location
        .ok_or_else(|| ResourceLocationMissing.build())?
        .resource
        .as_ref()
        .ok_or_else(|| ResourceLocationMissing.build())?
    {
        Resource::ResourceAddress(value) => ResourceLocationRef::ResourceAddress(value),
        Resource::ResourceIdentity(value) => ResourceLocationRef::ResourceIdentity(&value.encoded_identity),
        Resource::FolderAddress(value) => ResourceLocationRef::FolderAddress(&value.uri),
    };

    Ok(result)
}
