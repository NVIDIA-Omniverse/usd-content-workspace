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

use indexmap::IndexSet;
use serde::Deserialize;
use snafu::IntoError;
use std::{iter::FusedIterator, num::NonZero, sync::Arc};
use storage_core::layer::Layer;
use storage_service_permissions::{
    client::{Client, Config as ClientConfig},
    resource::{ParseError, ResourceUrl},
    types::{Condition, Decision, DecisionResponse, ResourceAction, ResourceActionRef, ResourceType},
};
use tracing::{debug, warn};

use crate::{
    backend::permissions::{
        action::{Action, ActionMap},
        error::{
            ClientInitSnafu, Error, ForbiddenSnafu, InitError, PermissionRequestFailedSnafu,
            RequestCredentialsInvalidSnafu, ResponseSizeMismatchSnafu,
        },
        request::PermissionRequestMetadata,
    },
    common::forward_headers::ForwardedHeaderNames,
};

pub mod capabilities;
pub mod error;
pub mod filefolder;
pub mod fileobject;
pub mod filter;
pub mod metadata;
pub mod versioning;

mod action;
mod request;
mod visibility;

use filter::PermissionRequestFilter;

/// Permission layer configuration.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct PermissionsConfig {
    pub enabled: bool,
    pub client: ClientConfig,
    pub action_names: ActionMap,
    pub include_acl_in_all_keys: bool,
    pub batch_size: NonZero<usize>,
    /// Incoming request headers copied onto the permission queries made on behalf of the caller.
    pub forward_headers: ForwardedHeaderNames,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client: Default::default(),
            action_names: Default::default(),
            include_acl_in_all_keys: false,
            batch_size: unsafe { NonZero::new_unchecked(100) },
            forward_headers: Default::default(),
        }
    }
}

/// Permission layer.
#[derive(Debug, Clone)]
pub struct PermissionsLayer {
    inner: Arc<PermissionLayerInner>,
}

impl PermissionsLayer {
    /// Create a layer from configuration.
    pub async fn try_from_config(
        config: PermissionsConfig,
        filter: PermissionRequestFilter,
    ) -> Result<Option<Self>, InitError> {
        if !config.enabled {
            return Ok(None);
        }

        let client = Client::try_from_config(config.client)
            .await
            .map_err(|source| ClientInitSnafu.into_error(source))?;

        Ok(Some(Self {
            inner: Arc::new(PermissionLayerInner {
                client,
                action_map: config.action_names,
                include_acl_in_all_keys: config.include_acl_in_all_keys,
                batch_size: config.batch_size,
                filter,
                forward_headers: config.forward_headers,
            }),
        }))
    }
}

impl<S> Layer<S> for PermissionsLayer {
    type Service = Permissions<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Permissions {
            inner: self.inner.clone(),
            next: inner,
        }
    }
}

/// A permission service.
pub struct Permissions<S> {
    next: S,
    inner: Arc<PermissionLayerInner>,
}

impl<S> Permissions<S> {
    /// Begin a batch query.
    pub(self) fn batch<'a>(&'a self) -> BatchQueryBuilder<'a> {
        self.inner.batch()
    }

    /// Whether the resource resolves to an endpoint that opted out of permission
    /// enforcement.
    pub(self) fn is_permission_check_excluded(
        &self,
        resource: &str,
        resource_type: ResourceType,
    ) -> Result<bool, Error> {
        self.inner.filter.is_excluded(resource, resource_type)
    }

    /// Validate access to an object.
    pub(self) async fn validate_object_access(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        resource: &str,
        action: Action,
    ) -> Result<(), Error> {
        self.validate_access(permission_metadata, resource, Some(ResourceType::Object), action)
            .await
    }

    /// Validate access to a folder.
    pub(self) async fn validate_folder_access(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        resource: &str,
        action: Action,
    ) -> Result<(), Error> {
        self.validate_access(permission_metadata, resource, Some(ResourceType::Folder), action)
            .await
    }

    /// Validate access to an operation that has a source and a target.
    pub(self) async fn validate_binop_object_access(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        source_resource: &str,
        source_action: Action,
        target_resource: &str,
        target_action: Action,
    ) -> Result<(), Error> {
        debug!(
            ?source_resource,
            ?source_action,
            ?target_resource,
            ?target_action,
            "evaluating object access"
        );

        // Bypass is per-resource: a public source must not grant access to a
        // private target and vice versa, so each side is evaluated on its own.
        let source_bypassed = self.is_permission_check_excluded(source_resource, ResourceType::Object)?;
        let target_bypassed = self.is_permission_check_excluded(target_resource, ResourceType::Object)?;

        let mut batch = self.batch();
        if !source_bypassed {
            batch
                .add_resource_hierarchically(source_resource, Some(ResourceType::Object), source_action)
                .map_err(Error::request_resource_url_invalid)?;
        }

        if !target_bypassed {
            batch
                .add_resource_hierarchically(target_resource, Some(ResourceType::Object), target_action)
                .map_err(Error::request_resource_url_invalid)?;
        }

        let mut retain_iter = batch
            .execute(permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        for (resource, action, bypassed) in [
            (source_resource, source_action, source_bypassed),
            (target_resource, target_action, target_bypassed),
        ] {
            if bypassed {
                debug!(?resource, ?action, "access to the object is bypassed");
            } else if retain_iter.next() == Some(true) {
                debug!(?resource, ?action, "access to the object is allowed");
            } else {
                debug!(?resource, ?action, "access to the object is denied");

                return ForbiddenSnafu.fail();
            }
        }

        Ok(())
    }

    /// Validate access to an entity, with an entity type hint.
    async fn validate_access(
        &self,
        permission_metadata: &PermissionRequestMetadata,
        resource: &str,
        resource_type: Option<ResourceType>,
        action: Action,
    ) -> Result<(), Error> {
        debug!(?resource, ?resource_type, ?action, "evaluating access to a resource");

        // An unspecified resource type is deduced from the URL the same way the
        // permission query parser does, so the bypass decision matches the type
        // the query is actually validated against.
        let resource_type = resource_type.unwrap_or_else(|| deduce_resource_type(resource));
        if self.is_permission_check_excluded(resource, resource_type)? {
            debug!(?resource, "access to the resource is bypassed");
            return Ok(());
        }

        let mut batch = self.batch();
        batch
            .add_resource_hierarchically(resource, Some(resource_type), action)
            .map_err(Error::request_resource_url_invalid)?;

        let result = batch.execute(permission_metadata).await?;
        if result.into_retain_iter_with_deny_on_missing().next() == Some(true) {
            debug!("access to the resource is allowed");
        } else {
            debug!("access to the resource is forbidden");

            return ForbiddenSnafu.fail();
        }

        Ok(())
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for Permissions<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Permissions")
            .field("next", &self.next)
            .field("inner", &self.inner)
            .finish()
    }
}

#[derive(Debug)]
pub struct BatchQueryBuilder<'a> {
    shared: &'a PermissionLayerInner,
    queries: IndexSet<ResourceAction>,
    lookup: Vec<usize>,
}

impl BatchQueryBuilder<'_> {
    /// Add a resource.
    pub(self) fn add_resource(
        &mut self,
        resource: &str,
        resource_type: Option<ResourceType>,
        action: Action,
    ) -> Result<(), ParseError> {
        let (resource, resource_type) = ResourceUrl::parse(resource, resource_type)?.into_components();

        // Normally, we won't see duplicates when we use this method.
        let (index, _) = self.queries.insert_full(ResourceAction {
            resource,
            resource_type: get_resource_type_name(resource_type).into(),
            action: self.shared.action_map.get_action_name(action).into(),
        });

        // Push the lookup sequence length and the lookup index for the just inserted item.
        self.lookup.extend_from_slice(&[1, index]);

        Ok(())
    }

    /// Add a resource with all its parents. They will be evaluated as a whole.
    pub(self) fn add_resource_hierarchically(
        &mut self,
        resource: &str,
        resource_type: Option<ResourceType>,
        action: Action,
    ) -> Result<(), ParseError> {
        let action = self.shared.action_map.get_action_name(action);
        self.add_resource_hierarchically_with_action_name(resource, resource_type, action)
    }

    /// Add a resource with all its parents. They will be evaluated as a whole.
    /// This method will either add the entire hierarchy or nothing.
    pub(self) fn add_resource_hierarchically_with_action_name(
        &mut self,
        resource: &str,
        resource_type: Option<ResourceType>,
        action: &str,
    ) -> Result<(), ParseError> {
        let resource = ResourceUrl::parse(resource, resource_type)?;

        // We don't know the sequence length and we will also insert the first element afte exhausting all the parents.
        let sequence_length_index = self.lookup.len();
        self.lookup.extend_from_slice(&[0, 0]);

        for (idx, (parent_resource, parent_resource_type)) in resource.iter_hierarchy().enumerate() {
            if idx != 0 {
                let resource_type = get_resource_type_name(parent_resource_type);
                let resource_action_ref = ResourceActionRef {
                    resource: parent_resource,
                    resource_type,
                    action,
                };
                let index = self.queries.get_index_of(&resource_action_ref).unwrap_or_else(|| {
                    let (index, _) = self.queries.insert_full(ResourceAction {
                        resource: parent_resource.into(),
                        resource_type: resource_type.into(),
                        action: action.into(),
                    });
                    index
                });

                self.lookup.push(index);
            }
        }

        // Finally, push the first element.
        let (resource, resource_type) = resource.into_components();
        let (index, _) = self.queries.insert_full(ResourceAction {
            resource,
            resource_type: get_resource_type_name(resource_type).into(),
            action: action.into(),
        });

        self.lookup[sequence_length_index] = self.lookup.len() - sequence_length_index - 1;
        self.lookup[sequence_length_index + 1] = index;

        Ok(())
    }

    /// Execute the query.
    pub(self) async fn execute(
        self,
        permission_metadata: &PermissionRequestMetadata,
    ) -> Result<BatchQueryResult, Error> {
        debug!(queries = ?self.queries, lookup = ?self.lookup, "submitting a query batch");

        // Every resource opted out of enforcement (e.g. a binop with both sides on
        // bypassed endpoints), so there is nothing to ask the permission service.
        if self.queries.is_empty() {
            return Ok(Default::default());
        }

        let expected_decisions_len = self.queries.len();
        let results = self
            .shared
            .client
            .query(
                &permission_metadata.0,
                self.queries,
                Condition::None,
                self.shared.batch_size,
            )
            .await
            .map_err(map_client_error)?;

        if results.len() != expected_decisions_len {
            return ResponseSizeMismatchSnafu {
                expected: expected_decisions_len,
                actual: results.len(),
            }
            .fail();
        }

        Ok(BatchQueryResult {
            results,
            lookup: self.lookup,
        })
    }
}

#[derive(Debug, Default)]
pub struct BatchQueryResult {
    results: Vec<DecisionResponse>,
    lookup: Vec<usize>,
}

impl BatchQueryResult {
    pub fn into_retain_iter_with_deny_on_missing(self) -> impl Iterator<Item = bool> {
        self.into_retain_iter(Decision::Deny)
    }

    pub fn into_retain_iter_with_allow_on_missing(self) -> impl Iterator<Item = bool> {
        self.into_retain_iter(Decision::Allow)
    }

    pub fn into_retain_iter(self, on_missing: Decision) -> impl Iterator<Item = bool> {
        self.into_decision_iter(on_missing)
            .map(|decision| decision == Decision::Allow)
    }

    pub fn into_decision_iter(self, on_missing: Decision) -> impl Iterator<Item = Decision> {
        BatchQueryResultIter {
            results: self.results,
            lookup_iter: self.lookup.into_iter(),
            on_missing,
        }
    }
}

#[derive(Debug)]
pub struct BatchQueryResultIter {
    results: Vec<DecisionResponse>,
    lookup_iter: std::vec::IntoIter<usize>,
    on_missing: Decision,
}

impl Iterator for BatchQueryResultIter {
    type Item = Decision;

    fn next(&mut self) -> Option<Self::Item> {
        let length = self.lookup_iter.next()?;

        let mut decision = DecisionResponse::Missing;
        for _ in 0..length {
            let lookup_index = self.lookup_iter.next()?;
            if decision == DecisionResponse::Missing {
                decision = match self.results[lookup_index] {
                    DecisionResponse::Allow => DecisionResponse::Allow,
                    DecisionResponse::Deny => DecisionResponse::Deny,
                    DecisionResponse::Missing => DecisionResponse::Missing,
                    DecisionResponse::Skip => {
                        warn!(?lookup_index, current = ?decision, "unexpected skip decision encountered");
                        DecisionResponse::Deny
                    },
                };
            }
        }

        Some(match decision {
            DecisionResponse::Allow => Decision::Allow,
            DecisionResponse::Missing => self.on_missing,
            DecisionResponse::Deny | DecisionResponse::Skip => Decision::Deny,
        })
    }
}

impl FusedIterator for BatchQueryResultIter {}

/// Permission layer detail.
#[derive(Debug)]
struct PermissionLayerInner {
    client: Client,
    action_map: ActionMap,
    include_acl_in_all_keys: bool,
    batch_size: NonZero<usize>,
    filter: PermissionRequestFilter,
    forward_headers: ForwardedHeaderNames,
}

impl PermissionLayerInner {
    fn batch<'a>(&'a self) -> BatchQueryBuilder<'a> {
        BatchQueryBuilder {
            shared: self,
            queries: Default::default(),
            lookup: Default::default(),
        }
    }
}

/// Deduce the resource type of a URL the same way `ResourceUrl::parse` does:
/// a path with a trailing slash addresses a folder, anything else an object.
fn deduce_resource_type(resource: &str) -> ResourceType {
    let path = resource
        .split_once(['?', '#'])
        .map(|(prefix, _)| prefix)
        .unwrap_or(resource);
    if path.ends_with('/') {
        ResourceType::Folder
    } else {
        ResourceType::Object
    }
}

/// Return the name of a resource type.
fn get_resource_type_name(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Folder => "folder",
        ResourceType::Object => "object",
    }
}

/// Convert special cases of the client error to the corresponding service errors.
#[track_caller]
fn map_client_error(source: storage_service_permissions::client::error::Error) -> Error {
    if matches!(
        source,
        storage_service_permissions::client::error::Error::InvalidPrincipal { .. }
    ) {
        RequestCredentialsInvalidSnafu.into_error(source)
    } else {
        PermissionRequestFailedSnafu.into_error(source)
    }
}
