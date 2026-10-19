// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use async_trait::async_trait;
use permission_core::{Action, Group, PolicyId, Principal, Resource};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use tokio::task::JoinError;
use utoipa::ToSchema;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Policy {
    pub id: PolicyId,
    pub order: i32,
    pub text: String,
    /// MD5 hex digest of the canonical policy form (`policies.policy_hash`).
    /// Used as the Hash accelerator for the in-process PolicySet cache; not
    /// an API field.
    pub policy_hash: String,
    pub resource: Option<Resource>,
    /// Hierarchy scope inferred from a `resource in <Type>::"<url>/"` policy
    /// head. Mutually exclusive with `resource`; retrieved for authorization
    /// requests whose resource is a descendant (or equal) of this URL prefix.
    pub resource_prefix: Option<Resource>,
    pub action: Option<Action>,
    pub principal: Option<Principal>,
    /// Group scope inferred from a `principal in Group::"<id>"` policy head.
    /// Mutually exclusive with `principal`; both unset means the policy is not
    /// scoped to a single principal or group.
    pub group: Option<Group>,
    pub created_at: String,
    pub created_by: String,
    /// Optional free-form note supplied by administrators describing the
    /// purpose of the policy or who it was created for. Not used during
    /// authorization and excluded from the canonical policy hash.
    pub comment: Option<String>,
}

impl Display for Policy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text)
    }
}

#[derive(Clone, Debug)]
pub struct PolicyQueryBatch {
    pub action: Action,
    pub resource: Option<Resource>,
}

/// Outcome of an upsert (`add_policy` / `add_policy_batch`): the stored policy
/// plus whether it was newly inserted (`true`) or matched an existing row by
/// canonical hash and updated in place (`false`).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PolicyUpsertResult {
    pub policy: Policy,
    pub created: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PolicyDbBatch {
    pub policy: String,
    pub action: Option<Action>,
    pub principal: Option<Principal>,
    pub group: Option<Group>,
    pub resource: Option<Resource>,
    /// Hierarchy scope inferred from a `resource in <Type>::"<url>/"` policy
    /// head. Mutually exclusive with `resource`.
    pub resource_prefix: Option<Resource>,
    pub order: i32,
    pub system: bool,
    pub created_by: String,
    /// Optional free-form administrator note. Not used during authorization
    /// and excluded from the canonical policy hash so two batches with the
    /// same Cedar text but different comments still collapse on dedup.
    pub comment: Option<String>,
    /// Pre-computed MD5 hex digest of the canonical policy form. The storage
    /// layer uses it directly for dedup / `policy_hash` so it doesn't have to
    /// re-parse the Cedar text on the write path.
    pub canonical_hash: String,
}

#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait PolicyStorage: Sync + Send {
    /// Called on the application startup.
    /// Implement this to initialize the storage and prepare it to be used in the application
    /// (e.g. create tables and indexes, run migrations, etc).
    async fn initialize(&self) -> Result<(), StorageErr>;

    /// Called when storage is no longer used by the application.
    /// Implement this to destroy storage and its data
    /// (e.g. drop tables).
    async fn destroy(&self) -> Result<(), StorageErr>;

    /// Replaces the system policies with the specified list.
    async fn set_system_policies(
        &self,
        policies: Vec<PolicyDbBatch>,
    ) -> Result<Vec<Policy>, StorageErr>;

    /// Adds a new policy for specified action, principal and resource.
    /// Action, principal and resource can be omited for unscoped policies.
    /// An optional `comment` stores administrator metadata about the policy.
    /// On `policy_hash` conflict the existing row is updated in place; the
    /// returned [`PolicyUpsertResult::created`] is `false` in that case and
    /// `true` for a brand-new insert.
    async fn add_policy(
        &self,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<PolicyUpsertResult, StorageErr>;

    /// Adds multiple policies in one single transaction. The `created` flag on
    /// each [`PolicyUpsertResult`] follows the same semantics as
    /// [`Self::add_policy`].
    async fn add_policy_batch(
        &self,
        batches: Vec<PolicyDbBatch>,
    ) -> Result<Vec<PolicyUpsertResult>, StorageErr>;

    /// Updates or inserts policy with the specified ID.
    /// Action, principal and resource can be omited for unscoped policies.
    /// An optional `comment` stores administrator metadata about the policy.
    async fn update_policy(
        &self,
        policy_id: PolicyId,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<Policy, StorageErr>;

    /// Returns related policies for the specified action, principal and
    /// resource. `groups` carries the requesting principal's group
    /// memberships: a group-scoped policy is returned when its group is in
    /// `groups`, in addition to unscoped and principal-matching policies.
    /// `resource_prefixes` carries the folder-hierarchy ancestors of the
    /// request resource (see `permission_core::resource_hierarchy_ancestors`):
    /// a `resource in <Type>::"<url>/"` policy is returned when its prefix is
    /// one of these ancestors, in addition to unscoped and exact-resource
    /// matches.
    async fn get_policies(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        groups: &[String],
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr>;

    /// Returns policies for actions and resources specified in batches.
    /// `groups` carries the requesting principal's group memberships, used the
    /// same way as in [`Self::get_policies`]. `resource_prefixes` is the union
    /// of folder-hierarchy ancestors across every batch resource, used to
    /// select matching `resource in <Type>::"<url>/"` policies.
    async fn get_policies_batched(
        &self,
        principal: Option<&Principal>,
        groups: &[String],
        batches: &[PolicyQueryBatch],
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr>;

    /// Returns policies from the storage using offset pagination. `group` is an
    /// exact-match administrative filter (list every policy scoped to one
    /// specific group), distinct from the membership-based `groups` filter used
    /// during authorization.
    async fn get_policies_paginated(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
        order: Option<i32>,
        page: i64,
        limit: i64,
    ) -> Result<PageResult<Policy>, StorageErr>;

    /// Returns policy with the specified ID.
    /// If the policy does not exist, returns an error with PolicyStorageErrKind::DoesNotExist.
    async fn get_policy_by_id(&self, policy_id: &PolicyId) -> Result<Policy, StorageErr>;

    /// Removes policy with the specified ID from the storage.
    /// If the policy does not exist, returns Ok.
    async fn remove_policy(&self, policy_id: &PolicyId) -> Result<(), StorageErr>;
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub page_size: usize,
    pub page_count: Option<i64>,
}

impl<T> PageResult<T> {
    pub fn new(items: Vec<T>, page: i64, page_count: Option<i64>) -> Self {
        Self {
            page,
            page_size: items.len(),
            page_count,
            items,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StorageErr {
    pub msg: String,
    pub kind: StorageErrKind,
}

impl Display for StorageErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl From<tokio_postgres::Error> for StorageErr {
    fn from(e: tokio_postgres::Error) -> Self {
        Self {
            msg: e.to_string(),
            kind: StorageErrKind::Internal,
        }
    }
}

impl From<deadpool_postgres::PoolError> for StorageErr {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self {
            msg: e.to_string(),
            kind: StorageErrKind::Internal,
        }
    }
}

impl From<JoinError> for StorageErr {
    fn from(value: JoinError) -> Self {
        Self {
            msg: value.to_string(),
            kind: StorageErrKind::Internal,
        }
    }
}

impl From<&str> for StorageErr {
    fn from(e: &str) -> Self {
        Self {
            msg: e.to_string(),
            kind: StorageErrKind::Internal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StorageErrKind {
    Internal,
    Constraint,
    DoesNotExist,
    InvalidPagination,
    NotSupported,
    /// The operation targets a system policy, which is managed by the service
    /// and cannot be modified or removed through the API.
    Forbidden,
    /// The write conflicts with an existing entry, such as registering a
    /// resource type whose kind is already owned by another service.
    Conflict,
}

pub fn combine_policies(policies: &[Policy]) -> String {
    policies
        .iter()
        .map(|policy| policy.text.clone().trim().to_string())
        .collect::<Vec<String>>()
        .join("\n")
}

#[async_trait]
pub trait MetadataStorage: Sync + Send {
    /// Called on the application startup.
    /// Implement this to initialize the storage and prepare it to be used in the application
    /// (e.g. create tables and indexes, run migrations, etc).
    async fn initialize(&self) -> Result<(), StorageErr>;

    /// Called when storage is no longer used by the application.
    /// Implement this to destroy storage and its data
    /// (e.g. drop tables).
    async fn destroy(&self) -> Result<(), StorageErr>;

    /// Replaces all system metadata (services, actions, resource types marked as system)
    /// with the provided configuration.
    async fn set_system_metadata(&self, metadata: MetadataConfig) -> Result<(), StorageErr>;

    /// Gets metadata for specified action and resource.
    /// If correspoding metadata does not exist, returns Ok with default metadata.
    async fn get_meta(
        &self,
        action: &Action,
        resource: Option<&Resource>,
    ) -> Result<Meta, StorageErr>;

    /// Returns all known services.
    async fn get_services(&self) -> Result<Vec<ServiceMeta>, StorageErr>;

    /// Gets metadata for the specified service.
    /// If metadata does not exist, returns Ok with default metadata.
    async fn get_service_meta(&self, service: &str) -> Result<ServiceMeta, StorageErr>;

    /// Adds or replaces metadata for  the specified service.
    async fn set_service_meta(&self, principal: &ServiceMeta) -> Result<(), StorageErr>;

    /// Removes metadata for the specified service.
    /// If metadata does not exist, returns Ok.
    async fn delete_service_meta(&self, service: &str) -> Result<(), StorageErr>;

    /// Returns known actions for the specified service.
    async fn get_service_actions(&self, service: &str) -> Result<Vec<Action>, StorageErr>;

    /// Replaces all known actions for the specified service with the specified value.
    async fn replace_service_actions(
        &self,
        service: &str,
        actions: Vec<Action>,
    ) -> Result<(), StorageErr>;

    /// Registers the specified service and action name as a known action.
    async fn set_service_action(&self, service: &str, action: &str) -> Result<Action, StorageErr>;

    /// Unregisters the specified action.
    /// If the action does not exist, returns Ok.
    async fn delete_service_action(&self, service: &str, action: &str) -> Result<(), StorageErr>;

    /// Returns all resource types for the specified service.
    async fn get_service_resource_types(
        &self,
        service: &str,
    ) -> Result<Vec<ResourceTypeMeta>, StorageErr>;

    /// Gets metadata for the specified resource type.
    /// If metadata does not exist, returns Ok with default metadata.
    async fn get_resource_type(
        &self,
        service: &str,
        kind: &str,
    ) -> Result<ResourceTypeMeta, StorageErr>;

    /// Resolves a resource type by its `kind` alone, independent of the owning
    /// service. Resource type kinds are globally unique, so at most one match
    /// exists. Returns `None` when the kind is not registered. Used by the
    /// meta-permission path to resolve a resource's hierarchy `parentType`
    /// without knowing its service.
    async fn get_resource_type_by_kind(
        &self,
        kind: &str,
    ) -> Result<Option<ResourceTypeMeta>, StorageErr>;

    /// Replaces all known resource types for the specified service with the specified value.
    async fn replace_resource_types(
        &self,
        service: &str,
        resource_types: Vec<ResourceTypeMeta>,
    ) -> Result<(), StorageErr>;

    /// Adds or replaces metadata for the specified resource type.
    async fn set_resource_type_meta(&self, resource: &ResourceTypeMeta) -> Result<(), StorageErr>;

    /// Removes metadata for the specified resource type.
    /// If metadata does not exist, returns Ok.
    async fn delete_resource_type_meta(&self, service: &str, kind: &str) -> Result<(), StorageErr>;
}

#[derive(Clone, Debug, Deserialize)]
pub struct MetadataConfig {
    #[serde(default)]
    pub services: Vec<MetadataServiceConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MetadataServiceConfig {
    pub name: String,

    #[serde(default)]
    pub principal: MetadataPrincipalConfig,

    #[serde(default)]
    pub actions: Vec<String>,

    #[serde(default)]
    #[serde(alias = "resourceTypes")]
    pub resource_types: Vec<MetadataResourceTypeConfig>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MetadataPrincipalConfig {
    #[serde(default)]
    #[serde(rename = "idClaim")]
    pub id_claim: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MetadataResourceTypeConfig {
    #[serde(rename = "type")]
    pub kind: String,

    #[serde(default)]
    #[serde(alias = "evaluationPriority")]
    pub evaluation_priority: EvaluationPriority,

    /// Type of this resource's hierarchy parent used to resolve
    /// `resource in <parentType>::"<url>/"` scopes (e.g. `object` files whose
    /// parent is a `folder`). Unset means the resource is its own parent type.
    #[serde(default)]
    #[serde(alias = "parentType")]
    pub parent_type: Option<String>,
}

impl MetadataConfig {
    pub fn find_service(&self, service: &str) -> Option<&MetadataServiceConfig> {
        self.services.iter().find(|s| s.name == service)
    }

    /// Ensures every resource type `kind` is registered under a single service.
    /// Returns an error naming the kinds that appear under more than one
    /// service, so global `kind -> service` resolution stays deterministic.
    pub fn validate_unique_resource_types(&self) -> Result<(), String> {
        let mut owners: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for service in &self.services {
            for rt in &service.resource_types {
                owners
                    .entry(rt.kind.as_str())
                    .or_default()
                    .push(service.name.as_str());
            }
        }

        let mut conflicts: Vec<String> = owners
            .iter()
            .filter(|(_, services)| services.len() > 1)
            .map(|(kind, services)| format!("{} (services: {})", kind, services.join(", ")))
            .collect();

        if conflicts.is_empty() {
            return Ok(());
        }

        conflicts.sort();
        Err(format!(
            "resource type kinds must be globally unique across services; conflicts: {}",
            conflicts.join("; ")
        ))
    }

    /// Adds entries from `other` that are missing from this configuration.
    /// `self` always wins on conflicts: when both configurations declare the
    /// same service, only actions and resource types that are not already
    /// present in `self` are pulled in from `other`. Services absent from
    /// `self` are appended as-is.
    pub fn merge(&mut self, other: &MetadataConfig) {
        for svc in &other.services {
            match self.services.iter_mut().find(|s| s.name == svc.name) {
                Some(existing) => {
                    for action in &svc.actions {
                        if !existing.actions.contains(action) {
                            existing.actions.push(action.clone());
                        }
                    }
                    for rt in &svc.resource_types {
                        if !existing.resource_types.iter().any(|r| r.kind == rt.kind) {
                            existing.resource_types.push(rt.clone());
                        }
                    }
                }
                None => self.services.push(svc.clone()),
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct Meta {
    pub service: ServiceMeta,
    pub resource: ResourceTypeMeta,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ServiceMeta {
    pub service: String,

    #[serde(default)]
    #[serde(alias = "idClaim")]
    pub id_claim: String,
}

impl ServiceMeta {
    pub fn new(service: &str) -> Self {
        ServiceMeta {
            service: service.to_string(),
            id_claim: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ResourceTypeMeta {
    pub service: String,

    #[serde(rename = "type")]
    pub kind: String,

    #[serde(default)]
    #[serde(alias = "evaluationPriority")]
    pub evaluation_priority: EvaluationPriority,

    /// Type of this resource's hierarchy parent used to resolve
    /// `resource in <parentType>::"<url>/"` scopes. Unset means the resource is
    /// its own parent type.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "parentType")]
    pub parent_type: Option<String>,
}

impl ResourceTypeMeta {
    pub fn new(service: &str, kind: &str) -> Self {
        ResourceTypeMeta {
            service: service.to_string(),
            kind: kind.to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        }
    }

    /// Resolves the hierarchy parent type: the explicit `parent_type` if set,
    /// otherwise the resource's own `kind`.
    pub fn parent_kind(&self) -> &str {
        self.parent_type.as_deref().unwrap_or(&self.kind)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EvaluationPriority {
    #[default]
    Forbid,
    Permit,
}

impl Display for EvaluationPriority {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EvaluationPriority::Forbid => write!(f, "forbid"),
            EvaluationPriority::Permit => write!(f, "permit"),
        }
    }
}

impl TryFrom<&str> for EvaluationPriority {
    type Error = &'static str;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "forbid" => Ok(EvaluationPriority::Forbid),
            "permit" => Ok(EvaluationPriority::Permit),
            "" => Ok(EvaluationPriority::Forbid),
            _ => Err("invalid evaluation priority"),
        }
    }
}

impl TryFrom<Option<&str>> for EvaluationPriority {
    type Error = &'static str;

    fn try_from(s: Option<&str>) -> Result<Self, Self::Error> {
        match s {
            None => Ok(EvaluationPriority::Forbid),
            Some(s) => EvaluationPriority::try_from(s),
        }
    }
}

pub mod config_file;
pub mod pool;
pub mod postgres;

pub use pool::{ConnectionPool, DirectPool};

#[cfg(test)]
mod tests;
