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

use bytes::Bytes;
use futures::StreamExt;
use storage_core::traits::{fileobject::v1alpha::*, BoxFuture, BoxStream, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::{
    backend::{metadata::Metadata, url_remapper::service::UrlRemapperService},
    common::{
        request::RequestExt,
        url_remapper::{
            error::UrlRewriteError, CheckpointMetadataCache, RewriteMode, RewriteResult, RuleTag, UrlRemapper,
        },
    },
    error::{InvalidArgumentError, UnexpectedError},
};

impl<T: Metadata> UrlRemapper<T> {
    /// Rewrite a mapped resource identity into an underlying resource identity.
    pub(super) async fn request_rewrite_read_resource_identity_v1alpha(
        &self,
        identity: &ResourceIdentity,
    ) -> Result<RewriteResult<ResourceIdentity>> {
        Ok(self
            .fwd_rewrite_versioned_url(&identity.encoded_identity, RewriteMode::Precise)
            .await
            .map_err(UrlRewriteError::into_not_found_error)?
            .map(|uri| ResourceIdentity { encoded_identity: uri }))
    }

    /// Rewrite a resource identity to a mapped resource identity.
    pub(super) async fn response_rewrite_resource_identity_v1alpha(
        &self,
        rule_tag: RuleTag,
        source: ResourceIdentity,
        cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<ResourceIdentity> {
        Ok(ResourceIdentity {
            encoded_identity: self
                .rev_rewrite_versioned_url_with_tag(rule_tag, &source.encoded_identity, cache)
                .await?
                .map(|(identity, _)| identity)
                .unwrap_or(source.encoded_identity),
        })
    }

    /// Rewrite all urls in a resource info.
    pub(in crate::backend::url_remapper) async fn response_rewrite_resource_info_v1alpha(
        &self,
        rule_tag: RuleTag,
        info: ResourceInfo,
        cache: Option<&mut CheckpointMetadataCache>,
    ) -> Result<ResourceInfo> {
        Ok(ResourceInfo {
            resource_identity: Some(
                self.response_rewrite_resource_identity_v1alpha(
                    rule_tag,
                    info.resource_identity
                        .ok_or_else(UnexpectedError::resource_identity_missing)?,
                    cache,
                )
                .await?,
            ),
            ..info
        })
    }

    /// Rewrite a read from address response.
    pub(super) async fn response_rewrite_read_from_address_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        response: ReadFromAddressResponse,
    ) -> Result<ReadFromAddressResponse> {
        use read_from_address_response::ReplyType;

        Ok(ReadFromAddressResponse {
            reply_type: match response.reply_type {
                Some(ReplyType::ResourceInfo(resource_info)) => Some(ReplyType::ResourceInfo(
                    self.response_rewrite_resource_info_v1alpha(rule_tag, resource_info, None)
                        .await?,
                )),
                other => other,
            },
        })
    }

    /// Rewrite a enumerate response.
    pub(super) fn response_rewrite_enumerate_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        response: EnumerateResponseWithContinueToken,
    ) -> EnumerateResponseWithContinueToken {
        EnumerateResponseWithContinueToken {
            response: EnumerateResponse {
                items: response
                    .response
                    .items
                    .into_iter()
                    .map(|address_info| AddressInfo {
                        resource_address: self
                            .rev_rewrite_unversioned_url_with_tag(
                                rule_tag,
                                &address_info.resource_address,
                                RewriteMode::Precise,
                            )
                            .unwrap_or(address_info.resource_address),
                        ..address_info
                    })
                    .collect(),
            },
            ..response
        }
    }

    /// Rewrite a resource identity in a write response.
    pub(super) fn response_rewrite_write_resource_identity_v1alpha(
        &self,
        rule_tag: RuleTag,
        identity: ResourceIdentity,
    ) -> ResourceIdentity {
        ResourceIdentity {
            encoded_identity: self
                .rev_rewrite_unversioned_url_with_tag(rule_tag, &identity.encoded_identity, RewriteMode::Precise)
                .unwrap_or(identity.encoded_identity),
        }
    }

    /// Rewrite a piece of resource info returned from a write call. This call skips reverse checkpoint mapping!
    pub(super) fn response_rewrite_write_resource_info_v1alpha(
        &self,
        rule_tag: RuleTag,
        resource_info: ResourceInfo,
    ) -> Result<ResourceInfo> {
        let resource_identity = resource_info
            .resource_identity
            .ok_or_else(|| InvalidArgumentError::new("Resource identity is missing from the resource info"))?;
        Ok(ResourceInfo {
            resource_identity: Some(self.response_rewrite_write_resource_identity_v1alpha(rule_tag, resource_identity)),
            metadata: resource_info.metadata,
        })
    }

    /// Rewrite a write response.
    pub(super) fn response_rewrite_write_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        response: WriteResponse,
    ) -> Result<WriteResponse> {
        use write_response::WriteResponseType;

        if let Some(WriteResponseType::ResourceInfo(resource_info)) = response.write_response_type {
            Ok(WriteResponse {
                write_response_type: Some(WriteResponseType::ResourceInfo(
                    self.response_rewrite_write_resource_info_v1alpha(rule_tag, resource_info)?,
                )),
            })
        } else {
            Ok(response)
        }
    }
}

#[async_trait]
impl<T: Metadata, S: FileObject> FileObject for UrlRemapperService<T, S> {
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let RewriteResult::Mapped(address, rule_tag) = self
            .mapper
            .fwd_rewrite_versioned_url(&request.get_ref().resource_address, RewriteMode::Precise)
            .await
            .map_err(UrlRewriteError::into_invalid_argument_error)?
        else {
            return self.inner.stat(request).await;
        };

        let stat_result = self
            .inner
            .stat(request.map(|_| StatRequest {
                resource_address: address,
            }))
            .await?;
        let resource_info = stat_result
            .resource_info
            .ok_or_else(UnexpectedError::resource_info_missing)?;

        Ok(StatResponse {
            resource_info: Some(
                self.mapper
                    .response_rewrite_resource_info_v1alpha(rule_tag, resource_info, None)
                    .await?,
            ),
        })
    }

    async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        let RewriteResult::Mapped(address, _rule_tag) = self
            .mapper
            .fwd_rewrite_versioned_url(&request.get_ref().resource_address, RewriteMode::Precise)
            .await
            .map_err(UrlRewriteError::into_invalid_argument_error)?
        else {
            return self.inner.is_read_only(request).await;
        };

        self.inner
            .is_read_only(request.map(|_| IsReadOnlyRequest {
                resource_address: address,
            }))
            .await
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let (metadata, extensions, message) = request.into_parts();
        let resource_identity = message
            .resource_identity
            .ok_or_else(InvalidArgumentError::resource_identity_missing)?;
        let mapped_resource_identity = self
            .mapper
            .request_rewrite_read_resource_identity_v1alpha(&resource_identity)
            .await?;

        self.inner
            .read(Request::from_parts(
                metadata,
                extensions,
                ReadRequest {
                    resource_identity: Some(mapped_resource_identity.unwrap_or(resource_identity)),
                    ..message
                },
            ))
            .await
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let RewriteResult::Mapped(address, rule_tag) = self
            .mapper
            .fwd_rewrite_versioned_url(&request.get_ref().resource_address, RewriteMode::Precise)
            .await
            .map_err(UrlRewriteError::into_invalid_argument_error)?
        else {
            return self.inner.read_from_address(request).await;
        };

        let stream = self
            .inner
            .read_from_address(request.map(|message| ReadFromAddressRequest {
                resource_address: address,
                ..message
            }))
            .await?;

        Ok(Box::pin({
            let mapper = self.mapper.clone();
            stream.then(move |response_or_error| {
                let mapper = mapper.clone();
                async move {
                    match response_or_error {
                        Ok(response) => {
                            mapper
                                .response_rewrite_read_from_address_response_v1alpha(rule_tag, response)
                                .await
                        },
                        Err(error) => Err(error),
                    }
                }
            })
        }))
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let previous_version = request
            .get_ref()
            .previous_version
            .as_ref()
            .map(|id| id.encoded_identity.as_ref());
        let RewriteResult::Mapped((destination_resource_address, previous_version), rule_tag) = self
            .mapper
            .request_rewrite_associated_address_identity(
                &request.get_ref().destination_resource_address,
                previous_version,
            )
            .await?
        else {
            return self.inner.write(request, data_future).await;
        };

        let stream = self
            .inner
            .write(
                request.map(|message| WriteParameters {
                    destination_resource_address,
                    previous_version: previous_version.map(|id| ResourceIdentity { encoded_identity: id }),
                    ..message
                }),
                data_future,
            )
            .await?;

        Ok(Box::pin({
            let mapper = self.mapper.clone();
            stream.then(move |response_or_error| {
                let response = response_or_error
                    .map(|response| mapper.response_rewrite_write_response_v1alpha(rule_tag, response));
                async move { response? }
            })
        }))
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let previous_version = request
            .get_ref()
            .previous_version
            .as_ref()
            .map(|id| id.encoded_identity.as_ref());
        let RewriteResult::Mapped((resource_address, previous_version), _) = self
            .mapper
            .request_rewrite_associated_address_identity(&request.get_ref().resource_address, previous_version)
            .await?
        else {
            return self.inner.delete(request).await;
        };

        self.inner
            .delete(request.map(|_| DeleteRequest {
                resource_address,
                previous_version: previous_version.map(|id| ResourceIdentity { encoded_identity: id }),
            }))
            .await
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let resource_address = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().destination_resource_address, RewriteMode::Precise);

        self.inner
            .abort_multipart_upload(request.map(|message| AbortMultipartUploadRequest {
                destination_resource_address: resource_address.unwrap_or(message.destination_resource_address),
                ..message
            }))
            .await
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let RewriteResult::Mapped(destination_resource_address, rule_tag) = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().destination_resource_address, RewriteMode::Precise)
        else {
            return self.inner.complete_multipart_upload(request).await;
        };

        let response = self
            .inner
            .complete_multipart_upload(request.map(|message| CompleteMultipartUploadRequest {
                destination_resource_address,
                ..message
            }))
            .await?;

        Ok(CompleteMultipartUploadResponse {
            resource_info: if let Some(resource_info) = response.resource_info {
                Some(
                    self.mapper
                        .response_rewrite_write_resource_info_v1alpha(rule_tag, resource_info)?,
                )
            } else {
                None
            },
        })
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let resource_address = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().destination_resource_address, RewriteMode::Precise);

        self.inner
            .upload_part(request.map(|message| UploadPartRequest {
                destination_resource_address: resource_address.unwrap_or(message.destination_resource_address),
                ..message
            }))
            .await
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let RewriteResult::Mapped(destination_resource_address, rule_tag) = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().destination_resource_address, RewriteMode::Precise)
        else {
            return self.inner.complete_redirect_upload(request).await;
        };

        let response = self
            .inner
            .complete_redirect_upload(request.map(|message| CompleteRedirectUploadRequest {
                destination_resource_address,
                ..message
            }))
            .await?;

        Ok(CompleteRedirectUploadResponse {
            resource_info: if let Some(resource_info) = response.resource_info {
                Some(
                    self.mapper
                        .response_rewrite_write_resource_info_v1alpha(rule_tag, resource_info)?,
                )
            } else {
                None
            },
        })
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let resource_address = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().destination_resource_address, RewriteMode::Precise);

        self.inner
            .fetch_write_type_info(request.map(|message| FetchWriteTypeInfoRequest {
                destination_resource_address: resource_address.unwrap_or(message.destination_resource_address),
            }))
            .await
    }

    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        let message = request.get_ref();
        let maybe_rewritten_source = self
            .mapper
            .fwd_rewrite_versioned_url(
                &message
                    .source_resource_identity
                    .as_ref()
                    .ok_or_else(InvalidArgumentError::resource_identity_missing)?
                    .encoded_identity,
                RewriteMode::Precise,
            )
            .await?;
        let maybe_rewritten_destination = self
            .mapper
            .request_rewrite_associated_address_identity(
                &message.destination_resource_address,
                message
                    .previous_version
                    .as_ref()
                    .map(|identity| identity.encoded_identity.as_str()),
            )
            .await?;

        match (maybe_rewritten_source, maybe_rewritten_destination) {
            (RewriteResult::Unchanged, RewriteResult::Unchanged) => self.inner.copy(request).await,
            (RewriteResult::Mapped(identity, _), RewriteResult::Unchanged) => {
                self.inner
                    .copy(request.map(|message| CopyRequest {
                        source_resource_identity: Some(ResourceIdentity {
                            encoded_identity: identity,
                        }),
                        ..message
                    }))
                    .await
            },
            (maybe_source_identity, RewriteResult::Mapped((target_address, maybe_target_identity), rule_tag)) => {
                let response = self
                    .inner
                    .copy(request.map(|message| CopyRequest {
                        source_resource_identity: if let RewriteResult::Mapped(identity, _) = maybe_source_identity {
                            Some(ResourceIdentity {
                                encoded_identity: identity,
                            })
                        } else {
                            message.source_resource_identity
                        },
                        destination_resource_address: target_address,
                        previous_version: maybe_target_identity.map(|identity| ResourceIdentity {
                            encoded_identity: identity,
                        }),
                        fail_if_exists: message.fail_if_exists,
                    }))
                    .await?;

                Ok(CopyResponse {
                    resource_identity: response.resource_identity.map(|identity| {
                        self.mapper
                            .response_rewrite_write_resource_identity_v1alpha(rule_tag, identity)
                    }),
                })
            },
        }
    }

    async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse> {
        let (metadata, extensions, message) = request.into_parts();

        let source_address_identity = self
            .mapper
            .request_rewrite_associated_address_identity(
                &message.source_resource_address,
                message
                    .source_previous_version
                    .as_ref()
                    .map(|id| id.encoded_identity.as_ref()),
            )
            .await?;
        let destination_address_identity = self
            .mapper
            .request_rewrite_associated_address_identity(
                &message.destination_resource_address,
                message
                    .destination_previous_version
                    .as_ref()
                    .map(|id| id.encoded_identity.as_ref()),
            )
            .await?;

        let (source_resource_address, source_previous_version) =
            if let RewriteResult::Mapped((address, identity), _) = source_address_identity {
                (
                    address,
                    identity.map(|encoded_identity| ResourceIdentity { encoded_identity }),
                )
            } else {
                (message.source_resource_address, message.source_previous_version)
            };

        if let RewriteResult::Mapped((destination_resource_address, destination_identity), rule_tag) =
            destination_address_identity
        {
            let response = self
                .inner
                .move_object(Request::from_parts(
                    metadata,
                    extensions,
                    MoveRequest {
                        source_resource_address,
                        source_previous_version,
                        destination_resource_address,
                        destination_previous_version: destination_identity
                            .map(|encoded_identity| ResourceIdentity { encoded_identity }),
                        fail_if_exists: message.fail_if_exists,
                    },
                ))
                .await?;

            Ok(MoveResponse {
                resource_identity: response.resource_identity.map(|identity| {
                    self.mapper
                        .response_rewrite_write_resource_identity_v1alpha(rule_tag, identity)
                }),
            })
        } else {
            self.inner
                .move_object(Request::from_parts(
                    metadata,
                    extensions,
                    MoveRequest {
                        source_resource_address,
                        source_previous_version,
                        destination_resource_address: message.destination_resource_address,
                        destination_previous_version: message.destination_previous_version,
                        fail_if_exists: message.fail_if_exists,
                    },
                ))
                .await
        }
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let RewriteResult::Mapped(address, rule_tag) = self
            .mapper
            .fwd_rewrite_unversioned_url(&request.get_ref().resource_address, RewriteMode::Fuzzy)
        else {
            return self.inner.enumerate(request, streaming).await;
        };

        let stream = self
            .inner
            .enumerate(
                request.map(|_| EnumerateRequest {
                    resource_address: address,
                }),
                streaming,
            )
            .await?;

        Ok(Box::pin({
            let mapper = self.mapper.clone();
            stream.then(move |response_or_error| {
                let response_or_error = response_or_error
                    .map(|response| mapper.response_rewrite_enumerate_response_v1alpha(rule_tag, response));
                async move { response_or_error }
            })
        }))
    }
}
