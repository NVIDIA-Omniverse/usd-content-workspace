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

use std::{
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
};

use futures::future::BoxFuture;
use storage_core::{
    layer::Layer,
    traits::{BoxStream, Request},
};
use tracing::debug;

use crate::{
    backend::metadata::{batch::*, *},
    common::{request::RequestExt, vec::VecExt},
};

/// Rejects duplicate update/delete entries in a metadata batch call.
pub struct RejectDuplicates;

impl<S> Layer<S> for RejectDuplicates {
    type Service = RejectDuplicatesService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RejectDuplicatesService { inner }
    }
}

#[derive(Debug)]
pub struct RejectDuplicatesService<T> {
    inner: T,
}

impl<T> RejectDuplicatesService<T> {
    fn discard_duplicates_and_invoke_batch_write<'a, ResE, ReqE, ResT, I>(
        &self,
        mut requests: Vec<ReqE>,
        invoke: I,
    ) -> BoxFuture<'a, Result<ResT>>
    where
        ReqE: BatchRequestEntry + DedupKey + Send + Sync + 'static,
        ResE: BatchResponseEntry + Send + Sync + 'static,
        ResT: BatchResponse<ResponseEntry = ResE> + Send + Sync + 'static,
        I: FnOnce(Vec<ReqE>) -> BoxFuture<'a, Result<ResT>> + 'a,
    {
        // Nothing to dedup if we've only got one request.
        if requests.len() <= 1 {
            debug!(requests_len = ?requests.len(), "skipping the duplicate check");

            return invoke(requests);
        }

        let mut dedup_map = HashMap::with_capacity(requests.len());
        let mut duplicate_count = 0usize;
        for request_entry in requests.iter() {
            match dedup_map.entry(request_entry.dedup_key()) {
                Entry::Vacant(entry) => {
                    entry.insert(1usize);
                },
                Entry::Occupied(mut entry) => {
                    if *entry.get() == 1 {
                        duplicate_count += 2;
                    } else {
                        duplicate_count += 1;
                    }

                    *entry.get_mut() += 1;
                },
            }
        }

        // Nothing to discard. Forward the request.
        if dedup_map.len() == requests.len() {
            drop(dedup_map);

            debug!("no duplicate batch write entries were detected, forwarding the request");

            return invoke(requests);
        }

        debug!("one or more duplicates were detected");

        let mut response_map = Vec::with_capacity(requests.len() - duplicate_count);
        let mut duplicate_request_idx = Vec::with_capacity(duplicate_count);
        for (request_idx, request_entry) in requests.iter().enumerate() {
            if dedup_map.get(&request_entry.dedup_key()).copied() != Some(1) {
                duplicate_request_idx.push(request_idx);
            } else {
                response_map.push(request_idx as u32);
            }
        }

        drop(dedup_map);

        debug_assert_eq!(response_map.len(), requests.len() - duplicate_count);
        debug_assert_eq!(duplicate_request_idx.len(), duplicate_count);

        // Nothing to forward - the batch comprises rejections only.
        if response_map.is_empty() {
            let response = ResT::from_entries(
                (0..requests.len() as u32)
                    .map(|request_index| ResE::from_error_entry(request_index, create_duplicate_error_entry())),
            );

            return Box::pin(async move { Ok(response) });
        }

        debug_assert!(duplicate_request_idx.is_sorted());

        requests.retain_indexed({
            let mut duplicate_request_idx_iter = duplicate_request_idx.iter().copied().peekable();
            move |request_idx, _item| {
                if let Some(next_duplicate_request_idx) = duplicate_request_idx_iter.peek() {
                    debug_assert!(request_idx <= *next_duplicate_request_idx);
                    if request_idx == *next_duplicate_request_idx {
                        duplicate_request_idx_iter.next();
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
        });

        debug!("request partially filtered, falling back to split-combine");

        let response_future = invoke(requests);
        Box::pin(async move {
            let mut response = response_future.await?;

            response.update_request_indexes(&response_map);
            response.extend(
                duplicate_request_idx
                    .into_iter()
                    .map(|request_index| ResE::from_error_entry(request_index as u32, create_duplicate_error_entry())),
            );

            Ok(response)
        })
    }
}

impl<T: Metadata> Metadata for RejectDuplicatesService<T> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn get_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<GetMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> {
        self.inner.get_metadata_batch(request)
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        self.inner.query_metadata(request, options)
    }

    fn update_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<UpdateMetadataBatchResponse>> {
        let (metadata, extensions, request) = request.into_parts();
        self.discard_duplicates_and_invoke_batch_write(request.requests, |requests| {
            self.inner.update_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                UpdateMetadataBatchRequest { requests },
            ))
        })
    }

    fn delete_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<DeleteMetadataBatchResponse>> {
        let (metadata, extensions, request) = request.into_parts();
        self.discard_duplicates_and_invoke_batch_write(request.requests, |requests| {
            self.inner.delete_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                DeleteMetadataBatchRequest { requests },
            ))
        })
    }
}

trait DedupKey {
    fn dedup_key<'a>(&'a self) -> impl Hash + Eq + 'a;
}

impl DedupKey for UpdateMetadataBatchRequestEntry {
    fn dedup_key<'a>(&'a self) -> impl Hash + Eq + 'a {
        get_dedup_key(
            self.resource_location.as_ref(),
            &self.user_metadata_key,
            self.namespace.as_deref(),
        )
    }
}

impl DedupKey for DeleteMetadataBatchRequestEntry {
    fn dedup_key<'a>(&'a self) -> impl Hash + Eq + 'a {
        get_dedup_key(
            self.resource_location.as_ref(),
            &self.user_metadata_key,
            self.namespace.as_deref(),
        )
    }
}

fn get_dedup_key<'a, 'b, 'c>(
    resource_location: Option<&'a ResourceLocation>,
    metadata_key: &'b str,
    namespace: Option<&'c str>,
) -> (&'a str, &'b str, &'c str) {
    use resource_location::Resource;

    let resource_uri = resource_location
        .map(|value| value.resource.as_ref())
        .unwrap_or_default()
        .map(|value| match value {
            Resource::ResourceAddress(value) => value.as_str(),
            Resource::ResourceIdentity(value) => value.encoded_identity.as_str(),
            Resource::FolderAddress(value) => value.uri.as_str(),
        })
        .unwrap_or_default();
    let namespace = namespace.unwrap_or(DEFAULT_NAMESPACE_NAME);

    (resource_uri, metadata_key, namespace)
}

fn create_duplicate_error_entry() -> BatchErrorEntry {
    use batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::InvalidArgument as i32,
        message: "duplicate batch entry".into(),
    }
}
