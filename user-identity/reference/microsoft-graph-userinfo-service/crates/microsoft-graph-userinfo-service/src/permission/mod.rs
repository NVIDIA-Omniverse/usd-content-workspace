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

pub mod client;

use std::fmt;
use std::hash::Hash;
use std::time::Duration;

use moka::future::Cache;
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::metrics;
use crate::models;

pub use client::{
    AuthorizationDecision, AuthorizationResponse, PermissionClient, PermissionError, Resource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ListUsers,
    GetUser,
    ListGroups,
    GetGroup,
    ListGroupMembers,
    GetGroupMember,
    ListUserGroups,
    GetUserGroup,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListUsers => "list-users",
            Self::GetUser => "get-user",
            Self::ListGroups => "list-groups",
            Self::GetGroup => "get-group",
            Self::ListGroupMembers => "list-group-members",
            Self::GetGroupMember => "get-group-member",
            Self::ListUserGroups => "list-user-groups",
            Self::GetUserGroup => "get-user-group",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub struct PermissionConfig {
    pub url: String,
    pub service_name: String,
    pub cache_ttl: Duration,
    pub cache_max_capacity: u64,
}

/// Enforces authorization by calling the Omniverse Permission Service and
/// caching decisions in memory. The cache is keyed by a SHA-256 fingerprint
/// of the token together with the action and optional resource, with a
/// configurable TTL.
pub struct PermissionChecker {
    client: PermissionClient,
    cache: Cache<CacheKey, AuthorizationDecision>,
}

/// Composite cache key combining a cryptographic fingerprint of the bearer
/// token with the action and optional resource identity. The token is hashed
/// with SHA-256 so the key space is collision-resistant against attacker-
/// crafted tokens and does not leak raw credentials into cache memory.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CacheKey {
    token_fingerprint: [u8; 32],
    action: Action,
    resource: Option<ResourceKey>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ResourceKey {
    id: String,
    resource_type: String,
}

impl From<&Resource> for ResourceKey {
    fn from(r: &Resource) -> Self {
        Self {
            id: r.id.clone(),
            resource_type: r.resource_type.clone(),
        }
    }
}

impl PermissionChecker {
    pub fn new(config: PermissionConfig) -> Self {
        let client = PermissionClient::new(config.url, config.service_name);
        let cache = Cache::builder()
            .time_to_live(config.cache_ttl)
            .max_capacity(config.cache_max_capacity)
            .build();
        Self { client, cache }
    }

    /// `extra_headers` are the incoming request headers selected for forwarding
    /// to the Permission Service. They carry request-scoped context (such as a
    /// correlation id) and do not influence the authorization decision, so the
    /// decision cache is keyed only by token, action, and resource.
    pub async fn check(
        &self,
        bearer_token: &str,
        action: Action,
        resource: Option<Resource>,
        extra_headers: reqwest::header::HeaderMap,
    ) -> Result<AuthorizationDecision, PermissionError> {
        let key = cache_key(bearer_token, action, resource.as_ref());

        if let Some(decision) = self.cache.get(&key).await {
            debug!(%action, "permission decision cache hit");
            metrics::METRICS.permission_cache_hits.add(1, &[]);
            return Ok(decision);
        }

        metrics::METRICS.permission_cache_misses.add(1, &[]);
        let response = self
            .client
            .check_authorization(bearer_token, action.as_str(), resource, extra_headers)
            .await?;
        let decision = response.decision;
        self.cache.insert(key, decision.clone()).await;
        debug!(%action, ?decision, "permission decision cached");
        Ok(decision)
    }

    /// Batch-checks an action for each resource and returns a boolean vec
    /// aligned with `resources` indicating which items are allowed.
    /// Already-cached decisions are served from cache; only uncached items
    /// hit the Permission Service via the batch endpoint.
    pub async fn batch_check(
        &self,
        bearer_token: &str,
        action: Action,
        resources: Vec<Resource>,
        extra_headers: reqwest::header::HeaderMap,
    ) -> Result<Vec<bool>, PermissionError> {
        if resources.is_empty() {
            return Ok(vec![]);
        }

        let mut results = vec![false; resources.len()];
        let mut uncached: Vec<(usize, Resource)> = Vec::new();

        for (i, resource) in resources.into_iter().enumerate() {
            let key = cache_key(bearer_token, action, Some(&resource));
            if let Some(decision) = self.cache.get(&key).await {
                results[i] = is_allowed(&decision);
                metrics::METRICS.permission_cache_hits.add(1, &[]);
            } else {
                uncached.push((i, resource));
                metrics::METRICS.permission_cache_misses.add(1, &[]);
            }
        }

        if uncached.is_empty() {
            debug!(%action, "batch permission check fully cached");
            return Ok(results);
        }

        let (indices, batch_resources): (Vec<usize>, Vec<Resource>) = uncached.into_iter().unzip();
        let responses = self
            .client
            .check_authorization_batch(
                bearer_token,
                action.as_str(),
                batch_resources.clone(),
                extra_headers,
            )
            .await?;

        for (j, idx) in indices.into_iter().enumerate() {
            let decision = responses
                .get(j)
                .map(|r| r.decision.clone())
                .unwrap_or(AuthorizationDecision::Deny);
            let key = cache_key(bearer_token, action, Some(&batch_resources[j]));
            results[idx] = is_allowed(&decision);
            self.cache.insert(key, decision).await;
        }

        debug!(%action, "batch permission check completed");
        Ok(results)
    }
}

fn is_allowed(decision: &AuthorizationDecision) -> bool {
    matches!(
        decision,
        AuthorizationDecision::Allow | AuthorizationDecision::Skip
    )
}

impl From<&models::User> for Resource {
    fn from(u: &models::User) -> Self {
        Resource {
            id: u.id.clone(),
            resource_type: "User".to_owned(),
            data: serde_json::to_value(u).ok(),
        }
    }
}

impl From<&models::Group> for Resource {
    fn from(g: &models::Group) -> Self {
        Resource {
            id: g.id.clone(),
            resource_type: "Group".to_owned(),
            data: serde_json::to_value(g).ok(),
        }
    }
}

/// Descriptor for a single REST route's permission requirements.
pub struct ActionDescriptor {
    pub action: Action,
    resource_type: Option<&'static str>,
    /// 0-based index into `/`-split segments of the URI path where the
    /// resource ID appears (e.g. 4 for `/api/v1alpha/users/{userId}`).
    resource_id_segment: Option<usize>,
}

impl ActionDescriptor {
    pub fn extract_resource(&self, uri_path: &str) -> Option<Resource> {
        let resource_type = self.resource_type?;
        let segment_idx = self.resource_id_segment?;
        let id = uri_path
            .split('/')
            .nth(segment_idx)
            .filter(|s| !s.is_empty())?;
        Some(Resource {
            id: id.to_string(),
            resource_type: resource_type.to_string(),
            data: None,
        })
    }
}

/// Maps a REST route pattern (from `MatchedPath`) to its permission descriptor.
pub fn action_for_route(path: &str) -> Option<ActionDescriptor> {
    match path {
        "/api/v1alpha/users/me" => None,
        "/api/v1alpha/users" => Some(ActionDescriptor {
            action: Action::ListUsers,
            resource_type: None,
            resource_id_segment: None,
        }),
        "/api/v1alpha/users/{userId}" => Some(ActionDescriptor {
            action: Action::GetUser,
            resource_type: Some("User"),
            resource_id_segment: Some(4),
        }),
        "/api/v1alpha/users/{userId}/groups" => Some(ActionDescriptor {
            action: Action::ListUserGroups,
            resource_type: Some("User"),
            resource_id_segment: Some(4),
        }),
        "/api/v1alpha/users/{userId}/groups/{groupId}" => Some(ActionDescriptor {
            action: Action::GetUserGroup,
            resource_type: Some("User"),
            resource_id_segment: Some(4),
        }),
        "/api/v1alpha/groups" => Some(ActionDescriptor {
            action: Action::ListGroups,
            resource_type: None,
            resource_id_segment: None,
        }),
        "/api/v1alpha/groups/{groupId}" => Some(ActionDescriptor {
            action: Action::GetGroup,
            resource_type: Some("Group"),
            resource_id_segment: Some(4),
        }),
        "/api/v1alpha/groups/{groupId}/users" => Some(ActionDescriptor {
            action: Action::ListGroupMembers,
            resource_type: Some("Group"),
            resource_id_segment: Some(4),
        }),
        "/api/v1alpha/groups/{groupId}/users/{userId}" => Some(ActionDescriptor {
            action: Action::GetGroupMember,
            resource_type: Some("Group"),
            resource_id_segment: Some(4),
        }),
        _ => None,
    }
}

fn cache_key(token: &str, action: Action, resource: Option<&Resource>) -> CacheKey {
    CacheKey {
        token_fingerprint: Sha256::digest(token.as_bytes()).into(),
        action,
        resource: resource.map(ResourceKey::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha256_of(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }

    #[test]
    fn cache_key_token_fingerprint_is_sha256_of_token() {
        let token = "header.payload.signature";
        let key = cache_key(token, Action::GetUser, None);
        assert_eq!(key.token_fingerprint, sha256_of(token));
    }

    #[test]
    fn cache_key_is_equal_for_identical_inputs() {
        let res = Resource::user("abc");
        let a = cache_key("tok", Action::GetUser, Some(&res));
        let b = cache_key("tok", Action::GetUser, Some(&res));
        assert_eq!(a, b);
    }

    #[test]
    fn cache_key_distinguishes_tokens_actions_and_resources() {
        let res_one = Resource::user("id-1");
        let res_two = Resource::user("id-2");
        let res_group = Resource::group("id-1");

        let base = cache_key("tok-a", Action::GetUser, Some(&res_one));

        assert_ne!(base, cache_key("tok-b", Action::GetUser, Some(&res_one)));
        assert_ne!(base, cache_key("tok-a", Action::GetGroup, Some(&res_one)));
        assert_ne!(base, cache_key("tok-a", Action::GetUser, Some(&res_two)));
        assert_ne!(base, cache_key("tok-a", Action::GetUser, Some(&res_group)));
        assert_ne!(base, cache_key("tok-a", Action::GetUser, None));
    }
}
