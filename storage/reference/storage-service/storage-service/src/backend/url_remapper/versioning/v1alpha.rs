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

use futures::{future::BoxFuture, StreamExt};
use storage_core::traits::{
    fileobject::v1alpha::{ResourceIdentity, ResourceInfo},
    versioning::v1alpha::*,
    BoxStream, Request, Result, StreamingOptions,
};
use tonic::async_trait;
use tracing::warn;

use crate::{
    backend::{metadata::Metadata, url_remapper::service::UrlRemapperService},
    common::{
        request::RequestExt,
        url_remapper::{CheckpointMetadataCache, RewriteMode, RewriteResult, RuleTag, UrlRemapper},
    },
    error::UnexpectedError,
};

impl<T: Metadata> UrlRemapper<T> {
    /// Rewrite a piece of resource info.
    async fn response_rewrite_version_info_v1alpha(
        &self,
        rule_tag: RuleTag,
        value: VersionInfo,
        cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<VersionInfo> {
        // Nucleus resource addresses and identities are the same if there is checkpoint metadata for that state.
        // Therefore, we resolve the checkpoint url from the provided identity and use it for both values. If the
        // identity in the response cannot be mapped, we fall back to the original identity, and the checkpoint
        // address is rewritten using standard rules by replacing the url prefix.
        let resource_info = value.resource_info.ok_or_else(UnexpectedError::resource_info_missing)?;
        let resource_identity = resource_info
            .resource_identity
            .ok_or_else(UnexpectedError::resource_identity_missing)?
            .encoded_identity;
        let (resource_address, resource_identity) = if let Some(resource_address) = value.resource_address {
            if let Some((checkpoint_url, is_checkpoint_url)) = self
                .rev_rewrite_versioned_url_with_tag(rule_tag, &resource_identity, cache)
                .await
                .map_err(UnexpectedError::url_rewrite_failed)?
            {
                let resource_address = if is_checkpoint_url {
                    checkpoint_url.clone()
                } else {
                    self.rev_rewrite_unversioned_url_with_tag(rule_tag, &resource_address, RewriteMode::Precise)
                        .unwrap_or(resource_address)
                };

                (Some(resource_address), checkpoint_url)
            } else {
                warn!(
                    ?resource_identity,
                    ?resource_address,
                    "failed to reverse-rewrite a resource identity in a version info returned for a remapped address"
                );

                (
                    Some(
                        self.rev_rewrite_unversioned_url_with_tag(rule_tag, &resource_address, RewriteMode::Precise)
                            .unwrap_or(resource_address),
                    ),
                    resource_identity,
                )
            }
        } else {
            (
                None,
                self.rev_rewrite_versioned_url_with_tag(rule_tag, &resource_identity, cache)
                    .await
                    .map_err(UnexpectedError::url_rewrite_failed)?
                    .map(|(identity, _)| identity)
                    .unwrap_or(resource_identity),
            )
        };

        Ok(VersionInfo {
            resource_info: Some(ResourceInfo {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: resource_identity,
                }),
                ..resource_info
            }),
            resource_address,
            ..value
        })
    }

    /// Rewrite a response of the enumerate versions call.
    async fn response_rewrite_versions_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        response: EnumerateVersionsResponseWithContinueToken,
    ) -> Result<EnumerateVersionsResponseWithContinueToken> {
        // We don't need cache if there's only one item in the response.
        let mut cache = if response.response.items.len() > 1 {
            Some(Default::default())
        } else {
            None
        };

        let mut items = Vec::with_capacity(response.response.items.len());
        for resource_info in response.response.items.into_iter() {
            items.push(
                self.response_rewrite_version_info_v1alpha(rule_tag, resource_info, cache.as_mut())
                    .await?,
            );
        }

        Ok(EnumerateVersionsResponseWithContinueToken {
            response: EnumerateVersionsResponse {
                items,
                ..response.response
            },
            ..response
        })
    }
}

#[async_trait]
impl<T: Metadata, S: Versioning> Versioning for UrlRemapperService<T, S> {
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let RewriteResult::Mapped(address, rule_tag) = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().resource_address, RewriteMode::Precise)
        else {
            return self
                .inner
                .enumerate_versions(request, streaming, version_id_token)
                .await;
        };

        let stream = self
            .inner
            .enumerate_versions(
                request.map(|_| EnumerateVersionsRequest {
                    resource_address: address,
                }),
                streaming,
                version_id_token,
            )
            .await?;

        Ok(Box::pin({
            let mapper = self.mapper.clone();
            stream.then(move |response_or_error| {
                let mapper = mapper.clone();
                async move {
                    match response_or_error {
                        Ok(response) => {
                            mapper
                                .response_rewrite_versions_response_v1alpha(rule_tag, response)
                                .await
                        },
                        Err(error) => Err(error),
                    }
                }
            })
        }))
    }

    fn is_versioned<'a: 'b, 'b>(
        &'a self,
        request: Request<IsVersionedRequest>,
    ) -> BoxFuture<'b, Result<IsVersionedResponse>> {
        let RewriteResult::Mapped(address, _rule_tag) = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().resource_address, RewriteMode::Precise)
        else {
            return self.inner.is_versioned(request);
        };

        self.inner.is_versioned(request.map(|_| IsVersionedRequest {
            resource_address: address,
        }))
    }
}
