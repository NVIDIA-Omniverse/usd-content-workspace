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

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::models::{Group, GroupQuery, Page, User, UserQuery};

pub enum DeltaChange<T> {
    Upsert(T),
    Remove(String),
}

/// Membership additions and removals for a single group, derived from a delta sync.
pub struct MembershipChange {
    pub group_id: String,
    pub added_user_ids: Vec<String>,
    pub removed_user_ids: Vec<String>,
}

/// Result of fetching one Graph API delta page.
pub enum PageOutcome {
    /// More pages follow; the contained URL is the next `@odata.nextLink`.
    NextPage(String),
    /// This was the last page; the contained string is the `@odata.deltaLink` token
    /// to use for the next incremental sync.
    Done(String),
}

impl PageOutcome {
    /// Build from the `@odata.nextLink` / `@odata.deltaLink` pair present on every
    /// Graph delta response. Exactly one of the two must be set.
    pub fn from_links(
        next_link: Option<String>,
        delta_link: Option<String>,
    ) -> Result<Self, ProviderError> {
        if let Some(next) = next_link {
            Ok(Self::NextPage(next))
        } else if let Some(delta) = delta_link {
            Ok(Self::Done(delta))
        } else {
            Err(ProviderError::Internal(
                "delta response missing both nextLink and deltaLink".into(),
            ))
        }
    }
}

/// One page of user delta changes.
pub struct UserPageResult {
    pub changes: Vec<DeltaChange<User>>,
    pub outcome: PageOutcome,
}

/// One page of group delta changes, optionally including membership changes.
pub struct GroupPageResult {
    pub changes: Vec<DeltaChange<Group>>,
    pub membership_changes: Vec<MembershipChange>,
    pub outcome: PageOutcome,
}

/// Source of incremental (delta) directory data. Each method fetches exactly one
/// Graph API page. The caller drives pagination by passing the `NextPage` URL back
/// on the next call, and stores the `Done` delta token for future incremental syncs.
#[async_trait]
pub trait DeltaSyncSource: Send + Sync {
    /// Fetch one page of user deltas. Pass `None` to start a new full sync,
    /// or the `@odata.nextLink` / `@odata.deltaLink` URL from the previous page.
    async fn fetch_user_page(&self, url: Option<&str>) -> Result<UserPageResult, ProviderError>;

    /// Fetch one page of group deltas. Pass `None` to start a new full sync,
    /// or the `@odata.nextLink` / `@odata.deltaLink` URL from the previous page.
    async fn fetch_group_page(&self, url: Option<&str>) -> Result<GroupPageResult, ProviderError>;
}

/// The directory subset an enterprise application's app-role assignments
/// resolve to: the assigned groups, the direct members of each of those
/// groups, and the assigned service principals surfaced as users.
pub struct ScopedDirectory {
    pub groups: Vec<Group>,
    /// Direct members of each assigned group (group_id → set of user_ids).
    pub members_by_group: HashMap<String, HashSet<String>>,
    /// Assigned service principals mapped to users. Empty when surfacing
    /// service principals as users is disabled.
    pub service_principals: Vec<User>,
}

/// Source of the enterprise-application app-role assignment scope. Fetches the
/// current assigned groups, their members, and the assigned service principals
/// in one call; there is no delta stream, so each sync cycle re-fetches the
/// full current set and reconciles by replacement.
#[async_trait]
pub trait AppRoleAssignmentsSource: Send + Sync {
    /// Fetch the current app-role assignment scope.
    async fn fetch(&self) -> Result<ScopedDirectory, ProviderError>;

    /// Lightweight permission/reachability probe used at writer startup. Fails
    /// closed (returns an error) when the assignments cannot be read — most
    /// importantly a `403` when the configured service principal is not the
    /// authenticating application's own and `Application.Read.All` is missing.
    async fn probe(&self) -> Result<(), ProviderError>;
}

#[derive(Debug)]
pub enum ProviderError {
    NotFound(String),
    Unauthorized(String),
    Forbidden(String),
    InvalidCursor(String),
    Upstream(String),
    Internal(String),
    RateLimited { retry_after: u64 },
    DeltaExpired,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::Unauthorized(msg) => write!(f, "unauthorized: {msg}"),
            Self::Forbidden(msg) => write!(f, "forbidden: {msg}"),
            Self::InvalidCursor(msg) => write!(f, "invalid cursor: {msg}"),
            Self::Upstream(msg) => write!(f, "upstream error: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
            Self::RateLimited { retry_after } => {
                write!(f, "rate limited (retry after {retry_after}s)")
            }
            Self::DeltaExpired => write!(f, "delta token expired"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Abstraction over an identity provider (Entra ID, Okta, etc.).
/// API handlers depend only on this trait, making the IdP swappable.
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Search the directory with optional email/display_name filters. The implementation
    /// must translate filters into IdP-native query syntax and handle cursor-based pagination,
    /// returning at most `params.effective_limit()` results per page.
    async fn list_users(&self, params: &UserQuery) -> Result<Page<User>, ProviderError>;

    /// Fetch a full user profile from the IdP by opaque identifier. Returns `NotFound` if the
    /// identifier does not correspond to any account in the directory.
    async fn get_user(&self, user_id: &str) -> Result<User, ProviderError>;

    /// Resolve the caller's own identity from the `oid` claim in their Bearer token.
    /// Returns the same User model as `get_user`. Returns `Unauthorized` if the
    /// token is malformed or the `oid` claim is absent.
    async fn get_me(&self, bearer_token: &str) -> Result<User, ProviderError>;

    /// Search the directory for security and distribution groups with optional display_name
    /// filter. Handles cursor-based pagination the same way as user listing.
    async fn list_groups(&self, params: &GroupQuery) -> Result<Page<Group>, ProviderError>;

    /// Fetch full group metadata from the IdP by opaque identifier. Returns `NotFound` if the
    /// identifier does not match any group in the directory.
    async fn get_group(&self, group_id: &str) -> Result<Group, ProviderError>;

    /// List the user members of a specific group, with optional email/display_name filter
    /// and cursor-based pagination.
    async fn list_group_members(
        &self,
        group_id: &str,
        params: &UserQuery,
    ) -> Result<Page<User>, ProviderError>;

    /// Enumerate the group memberships of a specific user, with optional display_name filter
    /// and cursor-based pagination. Useful for permission checks and role resolution.
    async fn list_user_groups(
        &self,
        user_id: &str,
        params: &GroupQuery,
    ) -> Result<Page<Group>, ProviderError>;

    /// Confirm that a user holds membership in a specific group and return the group details.
    /// Returns `NotFound` if the user is not a member or either identifier is invalid.
    async fn get_user_group(&self, user_id: &str, group_id: &str) -> Result<Group, ProviderError>;

    /// Confirm that a user is a member of a specific group and return the user details.
    /// Returns `NotFound` if the user is not a member or either identifier is invalid.
    async fn get_group_member(&self, group_id: &str, user_id: &str) -> Result<User, ProviderError>;
}
