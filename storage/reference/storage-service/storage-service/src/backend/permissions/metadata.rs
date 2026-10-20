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
use async_trait::async_trait;
use futures::{future::BoxFuture, StreamExt, TryStreamExt};
use snafu::IntoError;
use storage_core::traits::{BoxStream, Pagination, Request, Result, StreamingOptions};
use storage_service_permissions::types::ResourceType;
use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::UserMetadataValue;
use tracing::debug;

use crate::{
    backend::{
        metadata::{batch::*, batch_error_entry::Code, *},
        permissions::{
            action::Action,
            error::{
                Error, ForbiddenSnafu, MetadataKeyNotEditableSnafu, RequestResourceLocationMissingSnafu,
                RequestResourceUrlInvalidSnafu,
            },
            request::{PermissionRequestMetadata, RequestExt as _},
            BatchQueryBuilder, Permissions,
        },
    },
    common::request::RequestExt as _,
    error::InvalidArgumentError,
};

const ACL_KEY: &str = "acl";

const CONTINUATION_HANDLE_PREFIX: &str = "prm";

impl<S: Metadata> Permissions<S> {
    fn is_resource_location_permission_check_excluded(
        &self,
        resource_location: Option<&ResourceLocation>,
    ) -> Result<bool, Error> {
        resource_location
            .and_then(|location| get_resource_location_permission_query_params(location).ok())
            .map(|(resource, resource_type)| self.is_permission_check_excluded(resource, resource_type))
            .transpose()
            .map(|excluded| excluded.unwrap_or(false))
    }

    /// Validate access for a resource location.
    async fn validate_access_resource_location_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        resource_location: Option<&ResourceLocation>,
        action: Action,
    ) -> Result<()> {
        use crate::backend::metadata::resource_location::Resource;

        let Some(resource) = resource_location
            .map(|location| location.resource.as_ref())
            .unwrap_or_default()
        else {
            return Err(InvalidArgumentError::resource_location_missing().into());
        };

        match resource {
            Resource::ResourceAddress(value) => {
                self.validate_object_access(permission_metadata, value, action).await?;
            },
            Resource::ResourceIdentity(value) => {
                self.validate_object_access(permission_metadata, &value.encoded_identity, action)
                    .await?;
            },
            Resource::FolderAddress(value) => {
                self.validate_folder_access(permission_metadata, &value.uri, action)
                    .await?;
            },
        }

        Ok(())
    }

    /// Validate access for resource locations and invoke a batch write operation.
    async fn batch_validate_access_resource_locations_v1alpha_and_invoke_write<
        'a,
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry,
        ResT: BatchResponse<ResponseEntry = ResE>,
    >(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        action: Action,
        mut requests: Vec<ReqE>,
        invoke: impl FnOnce(Vec<ReqE>) -> BoxFuture<'a, Result<ResT>> + 'a,
    ) -> Result<ResT> {
        enum RequestState {
            /// Request rejected.
            Rejected { error: Error },
            /// Request has been queued for access validation.
            PendingValidateAccess,
            /// Request is permitted and can be forwarded to the next layer.
            PendingForward,
        }

        let mut batch = self.batch();
        let mut request_states = Vec::with_capacity(requests.len());
        for entry in requests.iter() {
            if is_acl_request(entry.user_metadata_key(), entry.namespace()) {
                request_states.push(RequestState::Rejected {
                    error: MetadataKeyNotEditableSnafu.build(),
                });
                continue;
            }

            let resource_location = entry.resource_location();
            // A bypassed resource is treated as allowed and kept out of the batch.
            match self.is_resource_location_permission_check_excluded(resource_location) {
                Ok(true) => {
                    request_states.push(RequestState::PendingForward);
                    continue;
                },
                Ok(false) => {},
                Err(error) => {
                    request_states.push(RequestState::Rejected { error });
                    continue;
                },
            }

            request_states.push(
                match try_add_resource_location_hierarchically(&mut batch, resource_location, action) {
                    Ok(()) => RequestState::PendingValidateAccess,
                    Err(error) => RequestState::Rejected { error },
                },
            );
        }

        let mut retain_iter = batch
            .execute(permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        for request_state in request_states.iter_mut() {
            if matches!(request_state, RequestState::PendingValidateAccess) {
                *request_state = if retain_iter.next() == Some(true) {
                    RequestState::PendingForward
                } else {
                    RequestState::Rejected {
                        error: ForbiddenSnafu.build(),
                    }
                };
            }
        }
        debug_assert_eq!(retain_iter.next(), None);

        // Filter out the rejected requests, mapping the kept indexes when the
        // two sets diverge.
        let mut response_map: Option<Vec<u32>> = None;
        let mut request_index = 0usize;
        requests.retain(|_entry| {
            let retain = matches!(request_states[request_index], RequestState::PendingForward);
            if retain {
                if let Some(response_map) = response_map.as_mut() {
                    response_map.push(request_index as u32);
                }
            } else if response_map.is_none() {
                response_map = Some((0..request_index as u32).collect());
            }

            request_index += 1;
            retain
        });

        let rejected_entries =
            request_states
                .into_iter()
                .enumerate()
                .filter_map(|(request_index, state)| match state {
                    RequestState::Rejected { error } => {
                        Some(ResE::from_error_entry(request_index as u32, create_error_entry(&error)))
                    },
                    _ => None,
                });

        // Nothing to forward. All valid requests have been rejected.
        if requests.is_empty() {
            return Ok(ResT::from_entries(rejected_entries));
        }

        let mut response = invoke(requests).await?;
        if let Some(response_map) = response_map.as_ref() {
            response.update_request_indexes(response_map);
        }

        response.extend(rejected_entries);

        Ok(response)
    }

    /// Create a new query response stream or continue streaming from an early interruption point.
    /// Response continuation handles will be modified.
    async fn stream_query_response_with_acls_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        use super::error::InvalidContinuationHandleSnafu;

        let StreamingOptions::Paginated(options) = options else {
            return self
                .stream_query_response_with_acls_v1alpha_streamed(permission_metadata, request)
                .await;
        };

        let continuation_state = QueryContinuationState::try_from_continuation_handle(options.continuation_handle)
            .map_err(|_| InvalidContinuationHandleSnafu.build())?;

        let resource_location = request.get_ref().resource_location.as_ref();
        match continuation_state {
            QueryContinuationState::Start => {
                debug!(?resource_location, "resuming the response stream from start");
                self.stream_query_response_with_acls_v1alpha_paginated(
                    permission_metadata,
                    request,
                    options.max_page_size,
                )
                .await
            },
            QueryContinuationState::StartNext => {
                debug!(
                    ?resource_location,
                    "resuming with an empty downstream continuation handle"
                );
                let stream = self
                    .stream_query_response_without_acls_v1alpha(
                        permission_metadata,
                        request,
                        StreamingOptions::Paginated(Pagination {
                            continuation_handle: None,
                            max_page_size: options.max_page_size,
                        }),
                    )
                    .await?
                    .and_then(update_continuation_in_query_metadata_response_page)
                    .boxed();

                Ok(stream)
            },
            QueryContinuationState::ContinueNext(handle) => {
                debug!(
                    ?handle,
                    ?resource_location,
                    "resuming query with the downstream continuation handle"
                );
                let stream = self
                    .stream_query_response_without_acls_v1alpha(
                        permission_metadata,
                        request,
                        StreamingOptions::Paginated(Pagination {
                            continuation_handle: Some(handle),
                            max_page_size: options.max_page_size,
                        }),
                    )
                    .await?
                    .and_then(update_continuation_in_query_metadata_response_page)
                    .boxed();

                Ok(stream)
            },
        }
    }

    /// Start streaming query responses from the very beginning, including the ACLs.
    /// Continuation handles will be omitted from the responses.
    async fn stream_query_response_with_acls_v1alpha_streamed(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        request: Request<QueryMetadataRequest>,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let resource_location = request.get_ref().resource_location.as_ref();
        let acls_value = self
            .validate_query_access_and_get_acls_value(permission_metadata, resource_location)
            .await?;
        let mut stream = self.next.query_metadata(request, StreamingOptions::Streamed).await?;

        Ok(Box::pin(try_stream! {
            yield QueryMetadataResponsePage {
                response: QueryMetadataResponse {
                    results: vec![create_acls_query_response_entry(acls_value)],
                },
                next_continuation_handle: None,
            };

            while let Some(page) = stream.try_next().await? {
                yield page;
            }
        }))
    }

    /// Fetch query responses from the very beginning, including the ACLs.
    /// Response continuation handles will be modified.
    async fn stream_query_response_with_acls_v1alpha_paginated(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        request: Request<QueryMetadataRequest>,
        max_page_size: Option<u32>,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let resource_location = request.get_ref().resource_location.as_ref();
        let acls_value = self
            .validate_query_access_and_get_acls_value(permission_metadata, resource_location)
            .await?;

        // Paginated response streams return a single response.
        let page = if max_page_size == Some(1) {
            QueryMetadataResponsePage {
                response: QueryMetadataResponse {
                    results: vec![create_acls_query_response_entry(acls_value)],
                },
                next_continuation_handle: Some(CONTINUATION_HANDLE_PREFIX.into()),
            }
        } else {
            let mut stream = self
                .next
                .query_metadata(
                    request,
                    StreamingOptions::Paginated(Pagination {
                        continuation_handle: None,
                        max_page_size: max_page_size.map(|value| value.max(1) - 1),
                    }),
                )
                .await?;

            if let Some(QueryMetadataResponsePage {
                mut response,
                next_continuation_handle,
            }) = stream.try_next().await?
            {
                response.results.push(create_acls_query_response_entry(acls_value));
                QueryMetadataResponsePage {
                    next_continuation_handle: next_continuation_handle
                        .map(|value| format!("{}:{}", CONTINUATION_HANDLE_PREFIX, value)),
                    response,
                }
            } else {
                QueryMetadataResponsePage {
                    response: QueryMetadataResponse {
                        results: vec![create_acls_query_response_entry(acls_value)],
                    },
                    next_continuation_handle: None,
                }
            }
        };

        Ok(futures::stream::once(async move { Ok(page) }).boxed())
    }

    /// Validate access to the resource location and return the ACLs.
    async fn validate_query_access_and_get_acls_value(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        resource_location: Option<&ResourceLocation>,
    ) -> Result<UserMetadataValue, Error> {
        use super::error::ForbiddenSnafu;

        let all_action_names = self.inner.action_map.unique_action_names();

        // A bypassed resource grants every action without querying the service.
        if self.is_resource_location_permission_check_excluded(resource_location)? {
            return Ok(create_acls_value(all_action_names.iter().map(|name| name.to_string())));
        }

        let mut batch = self.batch();
        for action_name in all_action_names {
            try_add_resource_location_hierarchically_with_action_name(&mut batch, resource_location, action_name)?;
        }

        let batch_query_result = batch.execute(permission_metadata).await?;

        let query_metadata_action_name = self.inner.action_map.get_action_name(Action::QueryMetadata);
        let mut allowed_action_names = Vec::with_capacity(all_action_names.len());
        let mut is_query_metadata_allowed = false;
        for (action_name, allowed) in all_action_names
            .iter()
            .zip(batch_query_result.into_retain_iter_with_deny_on_missing())
        {
            if allowed {
                allowed_action_names.push(action_name);

                is_query_metadata_allowed |= action_name == query_metadata_action_name;
            }
        }

        if !is_query_metadata_allowed {
            return ForbiddenSnafu.fail();
        }

        Ok(create_acls_value(
            allowed_action_names.into_iter().map(|name| name.clone().into_owned()),
        ))
    }

    /// Validate access to the resource and stream query responses.
    /// Response continuation handles won't be modified.
    async fn stream_query_response_without_acls_v1alpha(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let resource_location = request.get_ref().resource_location.as_ref();
        self.validate_access_resource_location_v1alpha(permission_metadata, resource_location, Action::QueryMetadata)
            .await?;

        self.next.query_metadata(request, options).await
    }
}

#[async_trait]
impl<S> Metadata for Permissions<S>
where
    S: Metadata,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.next)
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        enum RequestStatus {
            /// Request rejected.
            Rejected { error: Error },
            /// Request has been queued for access validation.
            PendingValidateAccess { batch_request_id: usize },
            /// Requests has been queued for ACL query.
            PendingQueryAcl { first_batch_request_id: usize },
            /// Request fulfilled.
            Ready { value: UserMetadataValue },
            /// Request is permitted and can be forwarded to the next layer.
            PendingForward,
        }

        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let (metadata, extensions, request) = request.into_parts();

        let mut requests = request.requests;
        let mut request_statuses = Vec::with_capacity(requests.len());
        let mut batch = self.batch();
        let acl_action_names = self.inner.action_map.unique_action_names();
        {
            let mut batch_request_id = 0usize;
            'outer: for entry in requests.iter() {
                // A bypassed resource is satisfied without querying the permission
                // service: forwarded for a normal read, all actions allowed for an ACL.
                let bypassed =
                    match self.is_resource_location_permission_check_excluded(entry.resource_location.as_ref()) {
                        Ok(excluded) => excluded,
                        Err(error) => {
                            request_statuses.push(RequestStatus::Rejected { error });
                            continue;
                        },
                    };

                if is_acl_request(&entry.user_metadata_key, entry.namespace.as_deref()) {
                    if bypassed {
                        request_statuses.push(RequestStatus::Ready {
                            value: create_acls_value(acl_action_names.iter().map(|name| name.to_string())),
                        });
                        continue;
                    }

                    let first_batch_request_id = batch_request_id;
                    for action_name in acl_action_names {
                        let maybe_error = try_add_resource_location_hierarchically_with_action_name(
                            &mut batch,
                            entry.resource_location.as_ref(),
                            action_name,
                        );
                        if let Err(error) = maybe_error {
                            debug_assert_eq!(first_batch_request_id, batch_request_id);

                            request_statuses.push(RequestStatus::Rejected { error });
                            continue 'outer;
                        }

                        batch_request_id += 1;
                    }

                    request_statuses.push(RequestStatus::PendingQueryAcl { first_batch_request_id });
                } else if bypassed {
                    request_statuses.push(RequestStatus::PendingForward);
                } else {
                    let maybe_error = try_add_resource_location_hierarchically(
                        &mut batch,
                        entry.resource_location.as_ref(),
                        Action::GetMetadata,
                    );
                    if let Err(error) = maybe_error {
                        request_statuses.push(RequestStatus::Rejected { error });
                        continue;
                    }

                    request_statuses.push(RequestStatus::PendingValidateAccess { batch_request_id });
                    batch_request_id += 1;
                }
            }
        }

        debug_assert_eq!(request_statuses.len(), requests.len());

        // Reject requests and satisfy ACL queries.
        let mut retain_iter = batch
            .execute(&permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing()
            .enumerate();

        for request_status in request_statuses.iter_mut() {
            if let RequestStatus::PendingValidateAccess { batch_request_id } = request_status {
                let allowed = loop {
                    if let Some((current_batch_request_id, allowed)) = retain_iter.next() {
                        debug_assert!(current_batch_request_id <= *batch_request_id);
                        if current_batch_request_id == *batch_request_id {
                            break allowed;
                        }
                    } else {
                        panic!("unexpected end of the batch retain iterator");
                    }
                };

                if allowed {
                    *request_status = RequestStatus::PendingForward;
                } else {
                    *request_status = RequestStatus::Rejected {
                        error: ForbiddenSnafu.build(),
                    };
                }
            } else if let RequestStatus::PendingQueryAcl { first_batch_request_id } = request_status {
                let get_metadata_action_name = self.inner.action_map.get_action_name(Action::GetMetadata);
                let mut allowed_action_names = Vec::with_capacity(acl_action_names.len());
                let mut is_get_metadata_allowed = false;
                for (action_offset, action_name) in acl_action_names.iter().enumerate() {
                    let action_batch_request_id = *first_batch_request_id + action_offset;
                    let allowed = loop {
                        if let Some((current_batch_request_id, allowed)) = retain_iter.next() {
                            debug_assert!(current_batch_request_id <= action_batch_request_id);
                            if current_batch_request_id == action_batch_request_id {
                                break allowed;
                            }
                        } else {
                            panic!("unexpected end of the batch retain iterator");
                        }
                    };

                    if allowed {
                        allowed_action_names.push(action_name);
                        is_get_metadata_allowed |= action_name == get_metadata_action_name;
                    }
                }

                if is_get_metadata_allowed {
                    // In tests, ACL values must be consistently sorted.
                    #[cfg(feature = "test")]
                    allowed_action_names.sort_unstable();

                    *request_status = RequestStatus::Ready {
                        value: create_acls_value(allowed_action_names.into_iter().map(|value| value.to_string())),
                    };
                } else {
                    *request_status = RequestStatus::Rejected {
                        error: ForbiddenSnafu.build(),
                    };
                }
            }
        }

        debug_assert_eq!(retain_iter.next(), None);

        // Filter out already satisfied and rejected requests.
        let mut response_map: Option<Vec<u32>> = None;
        let mut request_index = 0usize;
        requests.retain(|_entry| {
            debug_assert!(matches!(
                request_statuses[request_index],
                RequestStatus::PendingForward | RequestStatus::Rejected { .. } | RequestStatus::Ready { .. }
            ));

            let retain = if let RequestStatus::PendingForward = request_statuses[request_index] {
                if let Some(response_map) = response_map.as_mut() {
                    response_map.push(request_index as u32);
                }

                true
            } else {
                if response_map.is_none() {
                    response_map = Some((0..request_index as u32).collect());
                }

                false
            };

            request_index += 1;

            retain
        });

        // Forward requests, if any.
        let mut results = if !requests.is_empty() {
            let mut results = self
                .next
                .get_metadata_batch(Request::from_parts(
                    metadata,
                    extensions,
                    GetMetadataBatchRequest { requests },
                ))
                .await?
                .results;

            // Update the response indices, if needed.
            if let Some(response_map) = response_map {
                for response_entry in results.iter_mut() {
                    response_entry.request_index = response_map[response_entry.request_index as usize];
                }
            }

            results
        } else {
            Default::default()
        };

        // Include the rejections and ACL query results.
        results.extend(
            request_statuses
                .into_iter()
                .enumerate()
                .filter_map(|(request_index, status)| {
                    debug_assert!(matches!(
                        status,
                        RequestStatus::PendingForward | RequestStatus::Rejected { .. } | RequestStatus::Ready { .. }
                    ));

                    if let RequestStatus::Rejected { error } = status {
                        Some(GetMetadataBatchResponseEntry {
                            request_index: request_index as u32,
                            outcome: Some(get_metadata_batch_response_entry::Outcome::Error(create_error_entry(
                                &error,
                            ))),
                        })
                    } else if let RequestStatus::Ready { value } = status {
                        Some(GetMetadataBatchResponseEntry {
                            request_index: request_index as u32,
                            outcome: Some(get_metadata_batch_response_entry::Outcome::Success(value)),
                        })
                    } else {
                        None
                    }
                }),
        );

        Ok(GetMetadataBatchResponse { results })
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        if self.inner.include_acl_in_all_keys
            && matches!(
                request.get_ref().namespace.as_deref(),
                None | Some(DEFAULT_NAMESPACE_NAME)
            )
        {
            self.stream_query_response_with_acls_v1alpha(&permission_metadata, request, options)
                .await
        } else {
            self.stream_query_response_without_acls_v1alpha(&permission_metadata, request, options)
                .await
        }
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let (metadata, extensions, request) = request.into_parts();
        self.batch_validate_access_resource_locations_v1alpha_and_invoke_write(
            &permission_metadata,
            Action::UpdateMetadata,
            request.requests,
            |requests| {
                self.next.update_metadata_batch(Request::from_parts(
                    metadata,
                    extensions,
                    UpdateMetadataBatchRequest { requests },
                ))
            },
        )
        .await
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let (metadata, extensions, request) = request.into_parts();
        self.batch_validate_access_resource_locations_v1alpha_and_invoke_write(
            &permission_metadata,
            Action::DeleteMetadata,
            request.requests,
            |requests| {
                self.next.delete_metadata_batch(Request::from_parts(
                    metadata,
                    extensions,
                    DeleteMetadataBatchRequest { requests },
                ))
            },
        )
        .await
    }
}

enum QueryContinuationState {
    Start,
    StartNext,
    ContinueNext(String),
}

impl QueryContinuationState {
    fn try_from_continuation_handle(value: Option<String>) -> Result<Self, String> {
        let Some(mut value) = value else {
            return Ok(Self::Start);
        };

        if let Some(suffix) = value.strip_prefix(CONTINUATION_HANDLE_PREFIX) {
            if suffix.is_empty() {
                return Ok(Self::StartNext);
            } else if suffix.starts_with(":") {
                value.drain(0..=CONTINUATION_HANDLE_PREFIX.len());
                return Ok(Self::ContinueNext(value));
            }
        }

        Err(value)
    }
}

/// Return the resource uri and the resource type for a resource location.
fn get_resource_location_permission_query_params(
    resource_location: &ResourceLocation,
) -> Result<(&str, ResourceType), Error> {
    use crate::backend::metadata::resource_location::Resource;

    let (resource, resource_type) = match resource_location
        .resource
        .as_ref()
        .ok_or_else(|| RequestResourceLocationMissingSnafu.build())?
    {
        Resource::ResourceAddress(value) => (value.as_str(), ResourceType::Object),
        Resource::ResourceIdentity(value) => (value.encoded_identity.as_str(), ResourceType::Object),
        Resource::FolderAddress(value) => (value.uri.as_str(), ResourceType::Folder),
    };

    Ok((resource, resource_type))
}

/// Add a resource location to the batch query with an action name string.
fn try_add_resource_location_hierarchically_with_action_name(
    batch: &mut BatchQueryBuilder<'_>,
    resource_location: Option<&ResourceLocation>,
    action: &str,
) -> Result<(), Error> {
    let resource_location = resource_location.ok_or_else(|| RequestResourceLocationMissingSnafu.build())?;

    let (resource, resource_type) = get_resource_location_permission_query_params(resource_location)?;

    batch
        .add_resource_hierarchically_with_action_name(resource, Some(resource_type), action)
        .map_err(|source| RequestResourceUrlInvalidSnafu.into_error(source))?;

    Ok(())
}

/// Add a resource location to the batch query.
fn try_add_resource_location_hierarchically(
    batch: &mut BatchQueryBuilder<'_>,
    resource_location: Option<&ResourceLocation>,
    action: Action,
) -> Result<(), Error> {
    let resource_location = resource_location.ok_or_else(|| RequestResourceLocationMissingSnafu.build())?;

    let (resource, resource_type) = get_resource_location_permission_query_params(resource_location)?;

    batch
        .add_resource_hierarchically(resource, Some(resource_type), action)
        .map_err(|source| RequestResourceUrlInvalidSnafu.into_error(source))?;

    Ok(())
}

fn create_error_entry(error: &Error) -> BatchErrorEntry {
    match error {
        Error::Filter { .. } => create_batch_error_entry_v1alpha(Code::Internal, error.to_string()),
        Error::RequestCredentialsMissing { .. }
        | Error::RequestCredentialsDecode { .. }
        | Error::RequestCredentialsBearerExpected { .. }
        | Error::RequestCredentialsInvalid { .. }
        | Error::Forbidden { .. } => {
            create_batch_error_entry_v1alpha(Code::PermissionDenied, "access to the resource is forbidden")
        },
        Error::RequestResourceUrlInvalid { .. }
        | Error::RequestResourceIdentityMissing { .. }
        | Error::RequestFolderAddressMissing { .. }
        | Error::MetadataKeyNotEditable { .. }
        | Error::RequestResourceLocationMissing { .. }
        | Error::InvalidContinuationHandle { .. } => {
            create_batch_error_entry_v1alpha(Code::InvalidArgument, error.to_string())
        },
        Error::ResponseResourceUrlInvalid { .. }
        | Error::PermissionRequestFailed { .. }
        | Error::ResponseSizeMismatch { .. } => create_batch_error_entry_v1alpha(Code::Internal, "unexpected error"),
    }
}

fn create_acls_value(actions: impl IntoIterator<Item = impl Into<String>>) -> UserMetadataValue {
    use storage_service_protos::google::protobuf::{value::Kind, ListValue, Value};

    UserMetadataValue {
        value: Some(Value {
            kind: Some(Kind::ListValue(ListValue {
                values: actions
                    .into_iter()
                    .map(|action| Value {
                        kind: Some(Kind::StringValue(action.into())),
                    })
                    .collect(),
            })),
        }),
        etag: Default::default(),
    }
}

fn is_acl_request(user_metadata_key: &str, namespace: Option<&str>) -> bool {
    user_metadata_key == ACL_KEY && matches!(namespace, None | Some("default"))
}

async fn update_continuation_in_query_metadata_response_page<E>(
    value: QueryMetadataResponsePage,
) -> Result<QueryMetadataResponsePage, E> {
    Ok(QueryMetadataResponsePage {
        response: value.response,
        next_continuation_handle: value
            .next_continuation_handle
            .map(|value| format!("{}:{}", CONTINUATION_HANDLE_PREFIX, value)),
    })
}

fn create_acls_query_response_entry(value: UserMetadataValue) -> QueryMetadataResponseEntry {
    QueryMetadataResponseEntry {
        user_metadata: Some(value),
        user_metadata_key: ACL_KEY.into(),
    }
}
