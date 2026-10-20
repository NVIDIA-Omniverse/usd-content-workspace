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

use crate::{
    Meta, MetadataConfig, MetadataStorage, PageResult, Policy, PolicyDbBatch, PolicyQueryBatch,
    PolicyStorage, PolicyUpsertResult, ResourceTypeMeta, ServiceMeta, StorageErr, StorageErrKind,
};
use async_trait::async_trait;
use permission_core::{process_policy, Action, Group, PolicyId, Principal, Resource};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::info;

const SYSTEM_CREATED_BY: &str = "SYSTEM";

/// Watches a file for modifications and caches its raw content.
/// Reloads the file only when the modification time changes.
pub struct WatchedFile {
    pub file_path: PathBuf,
    content: Arc<RwLock<String>>,
    last_modified: Arc<RwLock<Option<std::time::SystemTime>>>,
}

impl WatchedFile {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            content: Arc::new(RwLock::new(String::new())),
            last_modified: Arc::new(RwLock::new(None)),
        }
    }

    /// Reloads the file content if the file has been modified since the last read.
    /// Returns `true` if the file was reloaded, `false` if it was unchanged.
    pub async fn load(&self) -> Result<bool, StorageErr> {
        let file_meta = fs::metadata(&self.file_path)
            .await
            .map_err(|e| StorageErr {
                msg: format!(
                    "Failed to read file metadata for {:?}: {}",
                    self.file_path, e
                ),
                kind: StorageErrKind::Internal,
            })?;

        let modified = file_meta.modified().map_err(|e| StorageErr {
            msg: format!(
                "Failed to get modification time for {:?}: {}",
                self.file_path, e
            ),
            kind: StorageErrKind::Internal,
        })?;

        let should_reload = {
            let last = self.last_modified.read().await;
            last.is_none() || last.unwrap() < modified
        };

        if should_reload {
            let content = fs::read_to_string(&self.file_path)
                .await
                .map_err(|e| StorageErr {
                    msg: format!("Failed to read file {:?}: {}", self.file_path, e),
                    kind: StorageErrKind::Internal,
                })?;

            *self.content.write().await = content;
            *self.last_modified.write().await = Some(modified);
            info!("File reloaded: {:?}", self.file_path);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn content(&self) -> tokio::sync::RwLockReadGuard<'_, String> {
        self.content.read().await
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PolicyConfig {
    pub policies: Vec<PolicyEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PolicyEntry {
    pub policy: String,
    #[serde(default)]
    pub order: Option<i32>,
    /// Optional free-form administrator note. Preserved verbatim and surfaced
    /// alongside the policy; never participates in Cedar evaluation.
    #[serde(default)]
    pub comment: Option<String>,
}

fn matches_filter<T: PartialEq>(policy_field: &Option<T>, filter: Option<&T>) -> bool {
    match (policy_field, filter) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(val), Some(filter)) => val == filter,
    }
}

/// Mirrors the SQL `principal_group IS NULL OR principal_group = ANY($groups)`
/// membership predicate: an unscoped policy matches any request; a
/// group-scoped policy matches only when the requesting principal belongs to
/// that group.
fn matches_group_membership(policy_group: &Option<Group>, groups: &[String]) -> bool {
    match policy_group {
        None => true,
        Some(group) => groups.iter().any(|g| g == &group.id),
    }
}

/// Mirrors the SQL `resource_prefix IS NULL OR resource_prefix = ANY($prefixes)`
/// predicate: a policy without a hierarchy scope matches any request; a
/// `resource in <Type>::"<url>/"` policy matches only when its prefix is one of
/// the request resource's folder-hierarchy ancestors.
fn matches_hierarchy(policy_prefix: &Option<Resource>, prefixes: &[Resource]) -> bool {
    match policy_prefix {
        None => true,
        Some(prefix) => prefixes.iter().any(|p| p == prefix),
    }
}

pub struct ConfigFilePolicyStorage {
    file: WatchedFile,
    policies: RwLock<Vec<Policy>>,
}

impl ConfigFilePolicyStorage {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file: WatchedFile::new(file_path),
            policies: RwLock::new(Vec::new()),
        }
    }

    async fn load(&self) -> Result<(), StorageErr> {
        let changed = self.file.load().await?;
        if !changed {
            return Ok(());
        }

        let content = self.file.content().await;

        let config: PolicyConfig = serde_norway::from_str(&content).map_err(|e| StorageErr {
            msg: format!(
                "Failed to parse policy file {:?}: {}",
                self.file.file_path, e
            ),
            kind: StorageErrKind::Internal,
        })?;

        let mut policies: Vec<Policy> = config
            .policies
            .into_iter()
            .enumerate()
            .map(|(idx, entry)| {
                let processed = process_policy(&entry.policy, false).map_err(|e| StorageErr {
                    msg: format!(
                        "Failed to process policy entry {:?} from file {:?}: {}",
                        entry, self.file.file_path, e
                    ),
                    kind: StorageErrKind::Internal,
                })?;
                Ok(Policy {
                    id: (idx + 1) as PolicyId,
                    order: entry.order.unwrap_or(0),
                    text: entry.policy,
                    policy_hash: processed.canonical_hash,
                    action: processed.scopes.action,
                    principal: processed.scopes.principal,
                    group: processed.scopes.group,
                    resource: processed.scopes.resource,
                    resource_prefix: processed.scopes.resource_prefix,
                    created_at: String::new(),
                    created_by: SYSTEM_CREATED_BY.to_string(),
                    comment: entry.comment,
                })
            })
            .collect::<Result<Vec<_>, StorageErr>>()?;

        policies.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        *self.policies.write().await = policies;
        Ok(())
    }

    fn error_not_supported(operation: &str) -> StorageErr {
        StorageErr {
            msg: format!("Operation '{}' is not supported for config file policy storage. Config file policy storage is read-only.", operation),
            kind: StorageErrKind::NotSupported,
        }
    }
}

#[async_trait]
impl PolicyStorage for ConfigFilePolicyStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        self.load().await
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        Ok(())
    }

    async fn set_system_policies(
        &self,
        _policies: Vec<PolicyDbBatch>,
    ) -> Result<Vec<Policy>, StorageErr> {
        Err(Self::error_not_supported("set_system_policies"))
    }

    async fn add_policy(
        &self,
        _policy: &str,
        _action: Option<&Action>,
        _principal: Option<&Principal>,
        _group: Option<&Group>,
        _resource: Option<&Resource>,
        _resource_prefix: Option<&Resource>,
        _order: i32,
        _created_by: &str,
        _comment: Option<&str>,
    ) -> Result<PolicyUpsertResult, StorageErr> {
        Err(Self::error_not_supported("add_policy"))
    }

    async fn add_policy_batch(
        &self,
        _batches: Vec<PolicyDbBatch>,
    ) -> Result<Vec<PolicyUpsertResult>, StorageErr> {
        Err(Self::error_not_supported("add_policy_batch"))
    }

    async fn update_policy(
        &self,
        _policy_id: PolicyId,
        _policy: &str,
        _action: Option<&Action>,
        _principal: Option<&Principal>,
        _group: Option<&Group>,
        _resource: Option<&Resource>,
        _resource_prefix: Option<&Resource>,
        _order: i32,
        _created_by: &str,
        _comment: Option<&str>,
    ) -> Result<Policy, StorageErr> {
        Err(Self::error_not_supported("update_policy"))
    }

    async fn get_policies(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        groups: &[String],
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        self.load().await?;
        let policies = self.policies.read().await;

        let mut filtered: Vec<Policy> = policies
            .iter()
            .filter(|p| {
                matches_filter(&p.principal, principal)
                    && matches_group_membership(&p.group, groups)
                    && matches_filter(&p.action, action)
                    && matches_filter(&p.resource, resource)
                    && matches_hierarchy(&p.resource_prefix, resource_prefixes)
            })
            .cloned()
            .collect();

        filtered.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        Ok(filtered)
    }

    async fn get_policies_batched(
        &self,
        principal: Option<&Principal>,
        groups: &[String],
        batches: &[PolicyQueryBatch],
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        if batches.is_empty() {
            return Ok(vec![]);
        }

        self.load().await?;
        let policies = self.policies.read().await;

        let actions: Vec<&Action> = batches.iter().map(|b| &b.action).collect();
        let resources: Vec<Option<&Resource>> =
            batches.iter().map(|b| b.resource.as_ref()).collect();

        let mut filtered: Vec<Policy> = policies
            .iter()
            .filter(|p| {
                matches_filter(&p.principal, principal)
                    && matches_group_membership(&p.group, groups)
                    && (p.action.is_none()
                        || p.action.as_ref().is_some_and(|a| actions.contains(&a)))
                    && (p.resource.is_none()
                        || p.resource
                            .as_ref()
                            .is_some_and(|r| resources.contains(&Some(r))))
                    && matches_hierarchy(&p.resource_prefix, resource_prefixes)
            })
            .cloned()
            .collect();

        filtered.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        Ok(filtered)
    }

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
    ) -> Result<PageResult<Policy>, StorageErr> {
        if page < 1 {
            return Err(StorageErr {
                msg: "`page` must be greater than 1.".to_string(),
                kind: StorageErrKind::InvalidPagination,
            });
        }

        if limit <= 0 || limit > 50 {
            return Err(StorageErr {
                msg: "`limit` must be in range between 0 and 50.".to_string(),
                kind: StorageErrKind::InvalidPagination,
            });
        }

        self.load().await?;
        let policies = self.policies.read().await;

        let filtered: Vec<Policy> = policies
            .iter()
            .filter(|p| {
                let principal_match = match principal {
                    Some(pr) if pr.id.is_empty() => p.principal.is_none(),
                    Some(pr) => p.principal.as_ref() == Some(pr),
                    None => true,
                };

                let group_match = match group {
                    Some(g) if g.id.is_empty() => p.group.is_none(),
                    Some(g) => p.group.as_ref() == Some(g),
                    None => true,
                };

                let action_match = match action {
                    Some(a) if a.name == "*" => {
                        p.action.as_ref().is_some_and(|pa| pa.service == a.service)
                    }
                    Some(a) if a.name.is_empty() => p.action.is_none(),
                    Some(a) => p.action.as_ref() == Some(a),
                    None => true,
                };

                let resource_match = match resource {
                    Some(r) if r.id.ends_with('*') => {
                        let id_prefix = r.id.strip_suffix('*').unwrap_or_default();
                        let prefix = format!(r#"{}::"{}"#, r.kind, id_prefix);
                        p.resource
                            .as_ref()
                            .is_some_and(|pr| pr.to_string().starts_with(&prefix))
                            || p.resource_prefix
                                .as_ref()
                                .is_some_and(|pr| pr.to_string().starts_with(&prefix))
                    }
                    Some(r) if r.id.is_empty() => {
                        p.resource.is_none() && p.resource_prefix.is_none()
                    }
                    Some(r) if !resource_prefixes.is_empty() => {
                        p.resource.as_ref() == Some(r)
                            || matches_hierarchy(&p.resource_prefix, resource_prefixes)
                                && p.resource_prefix.is_some()
                    }
                    Some(r) => {
                        p.resource.as_ref() == Some(r) || p.resource_prefix.as_ref() == Some(r)
                    }
                    None => true,
                };

                let order_match = order.is_none_or(|o| p.order == o);

                principal_match && group_match && action_match && resource_match && order_match
            })
            .cloned()
            .collect();

        let offset = ((page - 1) * limit) as usize;
        let end = (offset + limit as usize).min(filtered.len());

        let items: Vec<Policy> = if offset < filtered.len() {
            filtered[offset..end].to_vec()
        } else {
            Vec::new()
        };

        Ok(PageResult {
            page,
            page_size: items.len(),
            page_count: None,
            items,
        })
    }

    async fn get_policy_by_id(&self, policy_id: &PolicyId) -> Result<Policy, StorageErr> {
        self.load().await?;
        let policies = self.policies.read().await;

        policies
            .iter()
            .find(|p| p.id == *policy_id)
            .cloned()
            .ok_or_else(|| StorageErr {
                msg: "Policy not found.".to_string(),
                kind: StorageErrKind::DoesNotExist,
            })
    }

    async fn remove_policy(&self, _policy_id: &PolicyId) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("remove_policy"))
    }
}

pub struct ConfigFileMetadataStorage {
    file: WatchedFile,
    metadata: RwLock<MetadataConfig>,
    /// Metadata injected via `set_system_metadata`. Re-applied on top of the
    /// file-derived configuration whenever the file is reloaded so the
    /// bootstrap-time entries (e.g. the permission-service's own intrinsic
    /// actions and resources) survive operator edits to the metadata file.
    system_metadata: RwLock<MetadataConfig>,
    default_id_claim: String,
}

impl ConfigFileMetadataStorage {
    pub fn new(file_path: PathBuf, default_id_claim: String) -> Self {
        Self {
            file: WatchedFile::new(file_path),
            metadata: RwLock::new(MetadataConfig { services: vec![] }),
            system_metadata: RwLock::new(MetadataConfig { services: vec![] }),
            default_id_claim,
        }
    }

    async fn load(&self) -> Result<(), StorageErr> {
        let changed = self.file.load().await?;
        if !changed {
            return Ok(());
        }

        let content = self.file.content().await;

        let mut config: MetadataConfig =
            serde_norway::from_str(&content).map_err(|e| StorageErr {
                msg: format!(
                    "Failed to parse metadata file {:?}: {}",
                    self.file.file_path, e
                ),
                kind: StorageErrKind::Internal,
            })?;

        config.merge(&*self.system_metadata.read().await);

        config
            .validate_unique_resource_types()
            .map_err(|msg| StorageErr {
                msg: format!("Invalid metadata file {:?}: {}", self.file.file_path, msg),
                kind: StorageErrKind::Internal,
            })?;

        *self.metadata.write().await = config;
        Ok(())
    }

    fn error_not_supported(operation: &str) -> StorageErr {
        StorageErr {
            msg: format!("Operation '{}' is not supported for config file metadata storage. Config file metadata storage is read-only.", operation),
            kind: StorageErrKind::NotSupported,
        }
    }

    fn resolve_id_claim(&self, id_claim: &str) -> String {
        if id_claim.is_empty() {
            self.default_id_claim.clone()
        } else {
            id_claim.to_string()
        }
    }
}

#[async_trait]
impl MetadataStorage for ConfigFileMetadataStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        self.load().await
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        Ok(())
    }

    async fn set_system_metadata(&self, metadata: MetadataConfig) -> Result<(), StorageErr> {
        let mut merged = self.metadata.read().await.clone();
        merged.merge(&metadata);
        merged
            .validate_unique_resource_types()
            .map_err(|msg| StorageErr {
                msg: format!("Invalid system metadata: {}", msg),
                kind: StorageErrKind::Internal,
            })?;

        *self.metadata.write().await = merged;
        *self.system_metadata.write().await = metadata;
        Ok(())
    }

    async fn get_meta(
        &self,
        action: &Action,
        resource: Option<&Resource>,
    ) -> Result<Meta, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        let entry = config.find_service(&action.service);

        let service_meta = entry
            .map(|e| ServiceMeta {
                service: e.name.clone(),
                id_claim: self.resolve_id_claim(&e.principal.id_claim),
            })
            .unwrap_or_else(|| ServiceMeta {
                service: action.service.clone(),
                id_claim: self.default_id_claim.clone(),
            });

        let resource_meta = if let Some(resource) = resource {
            entry
                .and_then(|e| e.resource_types.iter().find(|rt| rt.kind == resource.kind))
                .map(|rt| ResourceTypeMeta {
                    service: action.service.clone(),
                    kind: rt.kind.clone(),
                    evaluation_priority: rt.evaluation_priority.clone(),
                    parent_type: rt.parent_type.clone(),
                })
                .unwrap_or_else(|| ResourceTypeMeta::new(&action.service, &resource.kind))
        } else {
            ResourceTypeMeta::new(&action.service, "")
        };

        Ok(Meta {
            service: service_meta,
            resource: resource_meta,
        })
    }

    async fn get_services(&self) -> Result<Vec<ServiceMeta>, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;
        Ok(config
            .services
            .iter()
            .map(|e| ServiceMeta {
                service: e.name.clone(),
                id_claim: self.resolve_id_claim(&e.principal.id_claim),
            })
            .collect())
    }

    async fn get_service_meta(&self, service: &str) -> Result<ServiceMeta, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        Ok(config
            .find_service(service)
            .map(|e| ServiceMeta {
                service: e.name.clone(),
                id_claim: self.resolve_id_claim(&e.principal.id_claim),
            })
            .unwrap_or_else(|| ServiceMeta {
                service: service.to_string(),
                id_claim: self.default_id_claim.clone(),
            }))
    }

    async fn set_service_meta(&self, _principal: &ServiceMeta) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("set_service_meta"))
    }

    async fn delete_service_meta(&self, _service: &str) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("delete_service_meta"))
    }

    async fn get_service_actions(&self, service: &str) -> Result<Vec<Action>, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        Ok(config
            .find_service(service)
            .map(|e| {
                e.actions
                    .iter()
                    .map(|a| Action {
                        name: a.clone(),
                        service: service.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn replace_service_actions(
        &self,
        _service: &str,
        _actions: Vec<Action>,
    ) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("replace_service_actions"))
    }

    async fn set_service_action(
        &self,
        _service: &str,
        _action: &str,
    ) -> Result<Action, StorageErr> {
        Err(Self::error_not_supported("set_service_action"))
    }

    async fn delete_service_action(&self, _service: &str, _action: &str) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("delete_service_action"))
    }

    async fn get_service_resource_types(
        &self,
        service: &str,
    ) -> Result<Vec<ResourceTypeMeta>, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        Ok(config
            .find_service(service)
            .map(|e| {
                e.resource_types
                    .iter()
                    .map(|rt| ResourceTypeMeta {
                        service: service.to_string(),
                        kind: rt.kind.clone(),
                        evaluation_priority: rt.evaluation_priority.clone(),
                        parent_type: rt.parent_type.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_resource_type(
        &self,
        service: &str,
        kind: &str,
    ) -> Result<ResourceTypeMeta, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        Ok(config
            .find_service(service)
            .and_then(|e| e.resource_types.iter().find(|rt| rt.kind == kind))
            .map(|rt| ResourceTypeMeta {
                service: service.to_string(),
                kind: rt.kind.clone(),
                evaluation_priority: rt.evaluation_priority.clone(),
                parent_type: rt.parent_type.clone(),
            })
            .unwrap_or_else(|| ResourceTypeMeta::new(service, kind)))
    }

    async fn get_resource_type_by_kind(
        &self,
        kind: &str,
    ) -> Result<Option<ResourceTypeMeta>, StorageErr> {
        self.load().await?;
        let config = self.metadata.read().await;

        Ok(config.services.iter().find_map(|service| {
            service
                .resource_types
                .iter()
                .find(|rt| rt.kind == kind)
                .map(|rt| ResourceTypeMeta {
                    service: service.name.clone(),
                    kind: rt.kind.clone(),
                    evaluation_priority: rt.evaluation_priority.clone(),
                    parent_type: rt.parent_type.clone(),
                })
        }))
    }

    async fn replace_resource_types(
        &self,
        _service: &str,
        _resource_types: Vec<ResourceTypeMeta>,
    ) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("replace_resource_types"))
    }

    async fn set_resource_type_meta(&self, _resource: &ResourceTypeMeta) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("set_resource_type_meta"))
    }

    async fn delete_resource_type_meta(
        &self,
        _service: &str,
        _kind: &str,
    ) -> Result<(), StorageErr> {
        Err(Self::error_not_supported("delete_resource_type_meta"))
    }
}
