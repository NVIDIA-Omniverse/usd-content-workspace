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

use futures::future::{join_all, BoxFuture};
use storage_core::traits::{BoxStream, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::{
    backend::{
        metadata::{batch::*, batch_error_entry::Code, resource_location::Resource, *},
        url_remapper::service::UrlRemapperService,
    },
    common::{
        request::RequestExt,
        url_remapper::{error::UrlRewriteError, RewriteMode, RewriteResult},
    },
};

impl<T: Metadata, S> UrlRemapperService<T, S> {
    /// Rewrite resource location values in a batch, invoke the backend function and remap results.
    async fn fwd_rewrite_resource_locations_v1alpha_and_invoke<
        'a,
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry,
        ResT: BatchResponse<ResponseEntry = ResE>,
    >(
        &self,
        mut requests: Vec<ReqE>,
        invoke: impl FnOnce(Vec<ReqE>) -> BoxFuture<'a, Result<ResT>> + 'a,
    ) -> Result<ResT> {
        // TODO: Implement a better subroutine that makes a single backend call.
        let rewrite_results = join_all(
            requests
                .iter_mut()
                .map(|entry| entry.resource_location_mut())
                .map(|location| self.fwd_mutate_resource_location_v1alpha(location)),
        )
        .await;

        debug_assert_eq!(rewrite_results.len(), requests.len());

        let mut errors = Vec::new();

        let mut response_map: Option<Vec<_>> = None;
        let mut rewrite_results = rewrite_results.into_iter().enumerate();
        requests.retain(|_entry| {
            // Should never trigger.
            let Some((original_request_entry_idx, rewrite_result)) = rewrite_results.next() else {
                return false;
            };

            if let Err(error) = rewrite_result {
                errors.push(ResE::from_error_entry(
                    original_request_entry_idx as u32,
                    create_batch_error_entry(&error),
                ));
                if response_map.is_none() {
                    response_map = Some((0..original_request_entry_idx as u32).collect());
                }

                false
            } else {
                if let Some(response_map) = response_map.as_mut() {
                    response_map.push(original_request_entry_idx as u32);
                }

                true
            }
        });

        let response = if let Some(response_map) = response_map {
            if requests.is_empty() {
                ResT::from_entries(errors)
            } else {
                let mut response = invoke(requests).await?;
                response.update_request_indexes(&response_map);
                response.extend(errors);

                response
            }
        } else {
            invoke(requests).await?
        };

        Ok(response)
    }

    /// Rewrite a single resource location value.
    async fn fwd_rewrite_resource_location_v1alpha(
        &self,
        mut location: Option<ResourceLocation>,
    ) -> Result<Option<ResourceLocation>, UrlRewriteError> {
        self.fwd_mutate_resource_location_v1alpha(location.as_mut()).await?;
        Ok(location)
    }

    /// Mutate a resource location in place.
    async fn fwd_mutate_resource_location_v1alpha(
        &self,
        location: Option<&mut ResourceLocation>,
    ) -> Result<(), UrlRewriteError> {
        // TODO: Technically, this must be an error.
        let Some(resource) = location.map(|location| location.resource.as_mut()).unwrap_or_default() else {
            return Ok(());
        };

        match resource {
            Resource::ResourceAddress(value) => {
                if let RewriteResult::Mapped(mapped, _) = self
                    .mapper
                    .fwd_rewrite_versioned_url(value, RewriteMode::Precise)
                    .await?
                {
                    *value = mapped;
                }
            },
            Resource::ResourceIdentity(value) => {
                if let RewriteResult::Mapped(mapped, _) = self
                    .mapper
                    .fwd_rewrite_versioned_url(&value.encoded_identity, RewriteMode::Precise)
                    .await?
                {
                    value.encoded_identity = mapped;
                }
            },
            Resource::FolderAddress(value) => {
                if let RewriteResult::Mapped(mapped, _) =
                    self.mapper.fwd_rewrite_unversioned_url(&value.uri, RewriteMode::Fuzzy)
                {
                    value.uri = mapped;
                }
            },
        }

        Ok(())
    }
}

#[async_trait]
impl<T: Metadata, S: Metadata> Metadata for UrlRemapperService<T, S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.fwd_rewrite_resource_locations_v1alpha_and_invoke(request.requests, move |requests| {
            self.inner.get_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                GetMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let (metadata, extensions, request) = request.into_parts();
        let resource_location = self
            .fwd_rewrite_resource_location_v1alpha(request.resource_location)
            .await
            .map_err(|error| error.into_not_found_error())?;

        self.inner
            .query_metadata(
                Request::from_parts(
                    metadata,
                    extensions,
                    QueryMetadataRequest {
                        resource_location,
                        ..request
                    },
                ),
                options,
            )
            .await
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.fwd_rewrite_resource_locations_v1alpha_and_invoke(request.requests, move |requests| {
            self.inner.update_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                UpdateMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.fwd_rewrite_resource_locations_v1alpha_and_invoke(request.requests, move |requests| {
            self.inner.delete_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                DeleteMetadataBatchRequest { requests },
            ))
        })
        .await
    }
}

fn create_batch_error_entry(error: &UrlRewriteError) -> BatchErrorEntry {
    match error {
        UrlRewriteError::MissingCheckpointMetadata(..) => BatchErrorEntry {
            code: Code::NotFound as i32,
            message: error.to_string(),
        },
        _ => BatchErrorEntry {
            code: Code::Internal as i32,
            message: error.to_string(),
        },
    }
}
