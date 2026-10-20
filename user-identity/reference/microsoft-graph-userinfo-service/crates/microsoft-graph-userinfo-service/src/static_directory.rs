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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use tracing::warn;

use crate::models::parse_display_name_filter;
use crate::models::{DisplayNameFilter, Group, GroupQuery, Page, User, UserQuery};
use crate::provider::{IdentityProvider, ProviderError};

/// An identity provider decorator that overlays a static directory on top of an
/// inner provider. Static entities are merged into every read regardless of how
/// the inner provider sources its data (live identity provider or cache) and
/// regardless of which resource kinds the inner provider caches.
///
/// Static entries take precedence over inner entries with the same id
/// (static-wins). In list responses static items are returned first, followed by
/// the inner provider's items with overridden ids removed.
pub struct StaticDirectoryProvider {
    inner: Arc<dyn IdentityProvider>,
    directory: Arc<StaticDirectory>,
}

impl StaticDirectoryProvider {
    pub fn new(inner: Arc<dyn IdentityProvider>, directory: Arc<StaticDirectory>) -> Self {
        Self { inner, directory }
    }

    async fn resolve_member_users(&self, group_id: &str) -> Result<Vec<User>, ProviderError> {
        let mut users = Vec::new();
        if let Some(ids) = self.directory.member_ids_of_group(group_id) {
            for id in ids {
                if let Some(user) = self.directory.user(id) {
                    users.push(user.clone());
                } else {
                    match self.inner.get_user(id).await {
                        Ok(user) => users.push(user),
                        Err(ProviderError::NotFound(_)) => {
                            warn!(
                                user_id = %id,
                                group_id = %group_id,
                                "static membership references an unknown user; skipping"
                            );
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        users.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(users)
    }

    async fn resolve_user_groups(&self, user_id: &str) -> Result<Vec<Group>, ProviderError> {
        let mut groups = Vec::new();
        if let Some(ids) = self.directory.group_ids_of_user(user_id) {
            for id in ids {
                if let Some(group) = self.directory.group(id) {
                    groups.push(group.clone());
                } else {
                    match self.inner.get_group(id).await {
                        Ok(group) => groups.push(group),
                        Err(ProviderError::NotFound(_)) => {
                            warn!(
                                group_id = %id,
                                user_id = %user_id,
                                "static membership references an unknown group; skipping"
                            );
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        groups.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(groups)
    }
}

#[async_trait]
impl IdentityProvider for StaticDirectoryProvider {
    async fn list_users(&self, params: &UserQuery) -> Result<Page<User>, ProviderError> {
        let static_items: Vec<User> = self
            .directory
            .users()
            .filter(|u| user_matches(u, params))
            .cloned()
            .collect();
        match plan_list(
            &static_items,
            params.cursor.as_deref(),
            params.effective_limit(),
        )? {
            ListStep::Done(page) => Ok(page),
            ListStep::NeedInner {
                prefix,
                inner_cursor,
                inner_limit,
            } => {
                let inner_params = UserQuery {
                    email: params.email.clone(),
                    display_name: params.display_name.clone(),
                    limit: inner_limit,
                    cursor: inner_cursor,
                };
                let inner_page = self.inner.list_users(&inner_params).await?;
                Ok(assemble(prefix, inner_page, &self.directory.user_id_set()))
            }
        }
    }

    async fn get_user(&self, user_id: &str) -> Result<User, ProviderError> {
        match self.directory.user(user_id) {
            Some(user) => Ok(user.clone()),
            None => self.inner.get_user(user_id).await,
        }
    }

    async fn get_me(&self, bearer_token: &str) -> Result<User, ProviderError> {
        if let Some(oid) = crate::http::decode_token_oid(bearer_token)
            && let Some(user) = self.directory.user(&oid)
        {
            return Ok(user.clone());
        }
        self.inner.get_me(bearer_token).await
    }

    async fn list_groups(&self, params: &GroupQuery) -> Result<Page<Group>, ProviderError> {
        let static_items: Vec<Group> = self
            .directory
            .groups()
            .filter(|g| group_matches(g, params.display_name.as_deref()))
            .cloned()
            .collect();
        match plan_list(
            &static_items,
            params.cursor.as_deref(),
            params.effective_limit(),
        )? {
            ListStep::Done(page) => Ok(page),
            ListStep::NeedInner {
                prefix,
                inner_cursor,
                inner_limit,
            } => {
                let inner_params = GroupQuery {
                    display_name: params.display_name.clone(),
                    limit: inner_limit,
                    cursor: inner_cursor,
                };
                let inner_page = self.inner.list_groups(&inner_params).await?;
                Ok(assemble(prefix, inner_page, &self.directory.group_id_set()))
            }
        }
    }

    async fn get_group(&self, group_id: &str) -> Result<Group, ProviderError> {
        match self.directory.group(group_id) {
            Some(group) => Ok(group.clone()),
            None => self.inner.get_group(group_id).await,
        }
    }

    async fn list_group_members(
        &self,
        group_id: &str,
        params: &UserQuery,
    ) -> Result<Page<User>, ProviderError> {
        let static_items: Vec<User> = self
            .resolve_member_users(group_id)
            .await?
            .into_iter()
            .filter(|u| user_matches(u, params))
            .collect();
        match plan_list(
            &static_items,
            params.cursor.as_deref(),
            params.effective_limit(),
        )? {
            ListStep::Done(page) => Ok(page),
            ListStep::NeedInner {
                prefix,
                inner_cursor,
                inner_limit,
            } => {
                let inner_params = UserQuery {
                    email: params.email.clone(),
                    display_name: params.display_name.clone(),
                    limit: inner_limit,
                    cursor: inner_cursor,
                };
                let inner_page = self
                    .inner_group_members_or_empty(group_id, &inner_params)
                    .await?;
                Ok(assemble(
                    prefix,
                    inner_page,
                    &self.directory.member_id_set(group_id),
                ))
            }
        }
    }

    async fn list_user_groups(
        &self,
        user_id: &str,
        params: &GroupQuery,
    ) -> Result<Page<Group>, ProviderError> {
        let static_items: Vec<Group> = self
            .resolve_user_groups(user_id)
            .await?
            .into_iter()
            .filter(|g| group_matches(g, params.display_name.as_deref()))
            .collect();
        match plan_list(
            &static_items,
            params.cursor.as_deref(),
            params.effective_limit(),
        )? {
            ListStep::Done(page) => Ok(page),
            ListStep::NeedInner {
                prefix,
                inner_cursor,
                inner_limit,
            } => {
                let inner_params = GroupQuery {
                    display_name: params.display_name.clone(),
                    limit: inner_limit,
                    cursor: inner_cursor,
                };
                let inner_page = self
                    .inner_user_groups_or_empty(user_id, &inner_params)
                    .await?;
                Ok(assemble(
                    prefix,
                    inner_page,
                    &self.directory.group_id_set_of_user(user_id),
                ))
            }
        }
    }

    async fn get_user_group(&self, user_id: &str, group_id: &str) -> Result<Group, ProviderError> {
        // Honor a static membership only when both endpoints resolve (in the
        // overlay or the inner provider); a dangling membership must not
        // synthesize a phantom result.
        if self.directory.is_member(group_id, user_id)
            && optional(self.get_user(user_id).await)?.is_some()
            && let Some(group) = optional(self.get_group(group_id).await)?
        {
            return Ok(group);
        }
        self.inner.get_user_group(user_id, group_id).await
    }

    async fn get_group_member(&self, group_id: &str, user_id: &str) -> Result<User, ProviderError> {
        if self.directory.is_member(group_id, user_id)
            && optional(self.get_group(group_id).await)?.is_some()
            && let Some(user) = optional(self.get_user(user_id).await)?
        {
            return Ok(user);
        }
        self.inner.get_group_member(group_id, user_id).await
    }
}

impl StaticDirectoryProvider {
    /// Call the inner provider's group-member listing, mapping `NotFound` to an
    /// empty page when the group exists only in the static directory.
    async fn inner_group_members_or_empty(
        &self,
        group_id: &str,
        params: &UserQuery,
    ) -> Result<Page<User>, ProviderError> {
        match self.inner.list_group_members(group_id, params).await {
            Ok(page) => Ok(page),
            Err(ProviderError::NotFound(_)) if self.directory.knows_group(group_id) => {
                Ok(empty_page())
            }
            Err(e) => Err(e),
        }
    }

    /// Call the inner provider's user-group listing, mapping `NotFound` to an
    /// empty page when the user exists only in the static directory.
    async fn inner_user_groups_or_empty(
        &self,
        user_id: &str,
        params: &GroupQuery,
    ) -> Result<Page<Group>, ProviderError> {
        match self.inner.list_user_groups(user_id, params).await {
            Ok(page) => Ok(page),
            Err(ProviderError::NotFound(_)) if self.directory.knows_user(user_id) => {
                Ok(empty_page())
            }
            Err(e) => Err(e),
        }
    }
}

/// Additional users, groups, and memberships supplied by the operator. Stored as
/// id-keyed maps so duplicate ids collapse with last-wins and lookups are direct.
#[derive(Debug, Clone, Default)]
pub struct StaticDirectory {
    users: BTreeMap<String, User>,
    groups: BTreeMap<String, Group>,
    members_by_group: HashMap<String, HashSet<String>>,
    groups_by_user: HashMap<String, HashSet<String>>,
}

impl StaticDirectory {
    pub fn load(path: &Path) -> Result<Self, StaticDirectoryError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| StaticDirectoryError::Read(path.display().to_string(), e))?;
        Self::from_json(&contents)
    }

    pub(crate) fn from_json(contents: &str) -> Result<Self, StaticDirectoryError> {
        let file: StaticDirectoryFile =
            serde_json::from_str(contents).map_err(StaticDirectoryError::Parse)?;
        Ok(Self::from_file(file))
    }

    fn from_file(file: StaticDirectoryFile) -> Self {
        let mut users: BTreeMap<String, User> = BTreeMap::new();
        for user in file.users {
            if users.insert(user.id.clone(), user.clone()).is_some() {
                warn!(id = %user.id, "duplicate static user id; keeping last occurrence");
            }
        }

        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        for group in file.groups {
            if groups.insert(group.id.clone(), group.clone()).is_some() {
                warn!(id = %group.id, "duplicate static group id; keeping last occurrence");
            }
        }

        let mut members_by_group: HashMap<String, HashSet<String>> = HashMap::new();
        let mut groups_by_user: HashMap<String, HashSet<String>> = HashMap::new();
        for membership in file.memberships {
            let members = members_by_group
                .entry(membership.group_id.clone())
                .or_default();
            for user_id in membership.user_ids {
                members.insert(user_id.clone());
                groups_by_user
                    .entry(user_id)
                    .or_default()
                    .insert(membership.group_id.clone());
            }
        }

        let directory = Self {
            users,
            groups,
            members_by_group,
            groups_by_user,
        };
        directory.warn_dangling_memberships();
        directory
    }

    /// Log a warning for memberships that reference ids absent from the overlay.
    /// Such references may still resolve against the inner provider, so this is
    /// advisory only.
    fn warn_dangling_memberships(&self) {
        for (group_id, user_ids) in &self.members_by_group {
            if !self.groups.contains_key(group_id) {
                warn!(
                    group_id = %group_id,
                    "static membership references a group not defined in the static directory; \
                     it will only resolve if the group exists in the inner directory"
                );
            }
            for user_id in user_ids {
                if !self.users.contains_key(user_id) {
                    warn!(
                        user_id = %user_id,
                        group_id = %group_id,
                        "static membership references a user not defined in the static directory; \
                         it will only resolve if the user exists in the inner directory"
                    );
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.groups.is_empty() && self.members_by_group.is_empty()
    }

    fn user(&self, id: &str) -> Option<&User> {
        self.users.get(id)
    }

    fn group(&self, id: &str) -> Option<&Group> {
        self.groups.get(id)
    }

    fn users(&self) -> impl Iterator<Item = &User> {
        self.users.values()
    }

    fn groups(&self) -> impl Iterator<Item = &Group> {
        self.groups.values()
    }

    fn member_ids_of_group(&self, group_id: &str) -> Option<&HashSet<String>> {
        self.members_by_group.get(group_id)
    }

    fn group_ids_of_user(&self, user_id: &str) -> Option<&HashSet<String>> {
        self.groups_by_user.get(user_id)
    }

    fn is_member(&self, group_id: &str, user_id: &str) -> bool {
        self.members_by_group
            .get(group_id)
            .is_some_and(|members| members.contains(user_id))
    }

    fn knows_group(&self, group_id: &str) -> bool {
        self.groups.contains_key(group_id)
    }

    fn knows_user(&self, user_id: &str) -> bool {
        self.users.contains_key(user_id)
    }

    fn user_id_set(&self) -> HashSet<String> {
        self.users.keys().cloned().collect()
    }

    fn group_id_set(&self) -> HashSet<String> {
        self.groups.keys().cloned().collect()
    }

    fn member_id_set(&self, group_id: &str) -> HashSet<String> {
        self.members_by_group
            .get(group_id)
            .cloned()
            .unwrap_or_default()
    }

    fn group_id_set_of_user(&self, user_id: &str) -> HashSet<String> {
        self.groups_by_user
            .get(user_id)
            .cloned()
            .unwrap_or_default()
    }
}

/// The next step in serving a list request: either a fully-formed page (no inner
/// call needed) or a static prefix plus the inner query parameters to append.
enum ListStep<T> {
    Done(Page<T>),
    NeedInner {
        prefix: Vec<T>,
        inner_cursor: Option<String>,
        inner_limit: u32,
    },
}

/// Decide how to serve one page of a list request, returning static items first.
/// `static_items` must already be filtered and sorted by id.
fn plan_list<T: Identified + Clone>(
    static_items: &[T],
    cursor: Option<&str>,
    limit: u32,
) -> Result<ListStep<T>, ProviderError> {
    let limit = limit as usize;
    if limit == 0 {
        return Ok(ListStep::Done(empty_page()));
    }

    let after = match decode_phase(cursor)? {
        Phase::Inner(inner_cursor) => {
            return Ok(ListStep::NeedInner {
                prefix: Vec::new(),
                inner_cursor,
                inner_limit: limit as u32,
            });
        }
        Phase::StaticStart => None,
        Phase::StaticAfter(id) => Some(id),
    };

    let start = match &after {
        Some(id) => static_items
            .iter()
            .position(|item| item.ident() > id.as_str())
            .unwrap_or(static_items.len()),
        None => 0,
    };
    let remaining = &static_items[start..];

    if remaining.len() > limit {
        let page = remaining[..limit].to_vec();
        let last_id = page.last().unwrap().ident().to_string();
        return Ok(ListStep::Done(Page {
            items: page,
            next_cursor: Some(encode_static_cursor(&last_id)),
        }));
    }

    let prefix = remaining.to_vec();
    let fill = limit - prefix.len();
    if fill == 0 {
        return Ok(ListStep::Done(Page {
            items: prefix,
            next_cursor: Some(encode_inner_start()),
        }));
    }

    Ok(ListStep::NeedInner {
        prefix,
        inner_cursor: None,
        inner_limit: fill as u32,
    })
}

/// Append the inner page to the static prefix, dropping inner items whose id is
/// overridden by the static directory, and re-encode the inner cursor.
fn assemble<T: Identified>(
    mut prefix: Vec<T>,
    inner_page: Page<T>,
    overridden: &HashSet<String>,
) -> Page<T> {
    for item in inner_page.items {
        if !overridden.contains(item.ident()) {
            prefix.push(item);
        }
    }
    Page {
        items: prefix,
        next_cursor: inner_page.next_cursor.as_deref().map(encode_inner_cursor),
    }
}

enum Phase {
    StaticStart,
    StaticAfter(String),
    Inner(Option<String>),
}

fn decode_phase(cursor: Option<&str>) -> Result<Phase, ProviderError> {
    let Some(cursor) = cursor else {
        return Ok(Phase::StaticStart);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    let decoded = String::from_utf8(bytes)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    if let Some(rest) = decoded.strip_prefix("s:") {
        Ok(Phase::StaticAfter(rest.to_string()))
    } else if let Some(rest) = decoded.strip_prefix("i:") {
        if rest.is_empty() {
            Ok(Phase::Inner(None))
        } else {
            Ok(Phase::Inner(Some(rest.to_string())))
        }
    } else {
        Err(ProviderError::InvalidCursor("invalid cursor format".into()))
    }
}

fn encode_static_cursor(last_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("s:{last_id}"))
}

fn encode_inner_start() -> String {
    URL_SAFE_NO_PAD.encode("i:")
}

fn encode_inner_cursor(inner_cursor: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("i:{inner_cursor}"))
}

fn empty_page<T>() -> Page<T> {
    Page {
        items: Vec::new(),
        next_cursor: None,
    }
}

/// Turn a lookup into an optional one: a `NotFound` result becomes `Ok(None)`
/// while every other error is propagated, so a missing entity is distinguished
/// from a transport or internal failure.
fn optional<T>(result: Result<T, ProviderError>) -> Result<Option<T>, ProviderError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(ProviderError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn user_matches(user: &User, params: &UserQuery) -> bool {
    if let Some(ref email) = params.email
        && user.email.as_deref() != Some(email.as_str())
    {
        return false;
    }
    if let Some(ref display_name) = params.display_name {
        return display_name_matches(&user.display_name, display_name);
    }
    true
}

fn group_matches(group: &Group, display_name: Option<&str>) -> bool {
    match display_name {
        Some(filter) => display_name_matches(&group.display_name, filter),
        None => true,
    }
}

fn display_name_matches(name: &str, filter: &str) -> bool {
    let name = name.to_lowercase();
    match parse_display_name_filter(filter) {
        DisplayNameFilter::Prefix(prefix) => name.starts_with(&prefix.to_lowercase()),
        DisplayNameFilter::Search(needle) => name.contains(&needle.to_lowercase()),
    }
}

trait Identified {
    fn ident(&self) -> &str;
}

impl Identified for User {
    fn ident(&self) -> &str {
        &self.id
    }
}

impl Identified for Group {
    fn ident(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Default, Deserialize)]
struct StaticDirectoryFile {
    #[serde(default)]
    users: Vec<User>,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    memberships: Vec<StaticMembership>,
}

#[derive(Debug, Deserialize)]
struct StaticMembership {
    group_id: String,
    #[serde(default)]
    user_ids: Vec<String>,
}

#[derive(Debug)]
pub enum StaticDirectoryError {
    Read(String, std::io::Error),
    Parse(serde_json::Error),
}

impl fmt::Display for StaticDirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StaticDirectoryError::Read(path, e) => {
                write!(f, "failed to read static directory file '{path}': {e}")
            }
            StaticDirectoryError::Parse(e) => {
                write!(f, "failed to parse static directory file: {e}")
            }
        }
    }
}

impl std::error::Error for StaticDirectoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StaticDirectoryError::Read(_, e) => Some(e),
            StaticDirectoryError::Parse(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_json() -> &'static str {
        r#"
        {
          "users": [
            { "id": "svc-1", "display_name": "Service Account", "username": "svc",
              "email": "svc@example.com", "active": true }
          ],
          "groups": [
            { "id": "grp-1", "display_name": "Local Admins", "description": "static" }
          ],
          "memberships": [
            { "group_id": "grp-1", "user_ids": ["svc-1"] }
          ]
        }
        "#
    }

    fn test_user(id: &str, name: &str) -> User {
        User {
            id: id.into(),
            display_name: name.into(),
            given_name: None,
            family_name: None,
            email: Some(format!("{id}@example.com")),
            username: id.into(),
            active: Some(true),
            job_title: None,
        }
    }

    fn test_group(id: &str, name: &str) -> Group {
        Group {
            id: id.into(),
            display_name: name.into(),
            description: None,
        }
    }

    fn user_query() -> UserQuery {
        UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        }
    }

    fn group_query() -> GroupQuery {
        GroupQuery {
            display_name: None,
            limit: 20,
            cursor: None,
        }
    }

    // ----- StaticDirectory parsing -----

    #[test]
    fn parses_full_file() {
        let dir = StaticDirectory::from_json(full_json()).unwrap();
        assert!(dir.user("svc-1").is_some());
        assert!(dir.group("grp-1").is_some());
        assert!(dir.is_member("grp-1", "svc-1"));
        assert!(!dir.is_empty());
    }

    #[test]
    fn empty_object_yields_empty_directory() {
        let dir = StaticDirectory::from_json("{}").unwrap();
        assert!(dir.is_empty());
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(StaticDirectory::from_json("{ not json").is_err());
    }

    #[test]
    fn missing_required_user_field_is_an_error() {
        let json = r#"{ "users": [ { "id": "u1", "display_name": "No Username" } ] }"#;
        assert!(StaticDirectory::from_json(json).is_err());
    }

    #[test]
    fn duplicate_user_id_keeps_last() {
        let json = r#"
        {
          "users": [
            { "id": "u1", "display_name": "First", "username": "a" },
            { "id": "u1", "display_name": "Second", "username": "b" }
          ]
        }
        "#;
        let dir = StaticDirectory::from_json(json).unwrap();
        assert_eq!(dir.user("u1").unwrap().display_name, "Second");
    }

    #[test]
    fn builds_reverse_membership_index() {
        let dir = StaticDirectory::from_json(full_json()).unwrap();
        let groups = dir.group_ids_of_user("svc-1").unwrap();
        assert!(groups.contains("grp-1"));
    }

    // ----- Mock inner provider -----

    #[derive(Default)]
    struct MockInner {
        users: Vec<User>,
        groups: Vec<Group>,
        members: HashMap<String, Vec<User>>,
        user_groups: HashMap<String, Vec<Group>>,
    }

    #[async_trait]
    impl IdentityProvider for MockInner {
        async fn list_users(&self, _params: &UserQuery) -> Result<Page<User>, ProviderError> {
            Ok(Page {
                items: self.users.clone(),
                next_cursor: None,
            })
        }
        async fn get_user(&self, user_id: &str) -> Result<User, ProviderError> {
            self.users
                .iter()
                .find(|u| u.id == user_id)
                .cloned()
                .ok_or_else(|| ProviderError::NotFound(user_id.into()))
        }
        async fn get_me(&self, bearer_token: &str) -> Result<User, ProviderError> {
            let oid = crate::http::decode_token_oid(bearer_token)
                .ok_or_else(|| ProviderError::Unauthorized("missing oid".into()))?;
            self.get_user(&oid).await
        }
        async fn list_groups(&self, _params: &GroupQuery) -> Result<Page<Group>, ProviderError> {
            Ok(Page {
                items: self.groups.clone(),
                next_cursor: None,
            })
        }
        async fn get_group(&self, group_id: &str) -> Result<Group, ProviderError> {
            self.groups
                .iter()
                .find(|g| g.id == group_id)
                .cloned()
                .ok_or_else(|| ProviderError::NotFound(group_id.into()))
        }
        async fn list_group_members(
            &self,
            group_id: &str,
            _params: &UserQuery,
        ) -> Result<Page<User>, ProviderError> {
            match self.members.get(group_id) {
                Some(users) => Ok(Page {
                    items: users.clone(),
                    next_cursor: None,
                }),
                None => Err(ProviderError::NotFound(group_id.into())),
            }
        }
        async fn list_user_groups(
            &self,
            user_id: &str,
            _params: &GroupQuery,
        ) -> Result<Page<Group>, ProviderError> {
            match self.user_groups.get(user_id) {
                Some(groups) => Ok(Page {
                    items: groups.clone(),
                    next_cursor: None,
                }),
                None => Err(ProviderError::NotFound(user_id.into())),
            }
        }
        async fn get_user_group(
            &self,
            _user_id: &str,
            group_id: &str,
        ) -> Result<Group, ProviderError> {
            Err(ProviderError::NotFound(group_id.into()))
        }
        async fn get_group_member(
            &self,
            _group_id: &str,
            user_id: &str,
        ) -> Result<User, ProviderError> {
            Err(ProviderError::NotFound(user_id.into()))
        }
    }

    fn provider(dir: StaticDirectory, inner: MockInner) -> StaticDirectoryProvider {
        StaticDirectoryProvider::new(Arc::new(inner), Arc::new(dir))
    }

    // ----- Overlay behavior -----

    #[tokio::test]
    async fn list_users_returns_static_first_then_inner() {
        let dir = StaticDirectory::from_json(
            r#"{ "users": [ { "id": "s1", "display_name": "Static One", "username": "s1" } ] }"#,
        )
        .unwrap();
        let inner = MockInner {
            users: vec![test_user("u1", "Graph One")],
            ..Default::default()
        };
        let page = provider(dir, inner)
            .list_users(&user_query())
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "u1"]);
    }

    #[tokio::test]
    async fn static_user_overrides_inner_with_same_id() {
        let dir = StaticDirectory::from_json(
            r#"{ "users": [ { "id": "u1", "display_name": "Overridden", "username": "ovr" } ] }"#,
        )
        .unwrap();
        let inner = MockInner {
            users: vec![test_user("u1", "Graph One")],
            ..Default::default()
        };
        let p = provider(dir, inner);

        let page = p.list_users(&user_query()).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].display_name, "Overridden");

        let user = p.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Overridden");
    }

    #[tokio::test]
    async fn get_user_falls_back_to_inner() {
        let dir = StaticDirectory::default();
        let inner = MockInner {
            users: vec![test_user("u1", "Graph One")],
            ..Default::default()
        };
        let user = provider(dir, inner).get_user("u1").await.unwrap();
        assert_eq!(user.id, "u1");
    }

    #[tokio::test]
    async fn list_groups_returns_static_first() {
        let dir = StaticDirectory::from_json(
            r#"{ "groups": [ { "id": "sg", "display_name": "Static Group" } ] }"#,
        )
        .unwrap();
        let inner = MockInner {
            groups: vec![test_group("g1", "Graph Group")],
            ..Default::default()
        };
        let page = provider(dir, inner)
            .list_groups(&group_query())
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["sg", "g1"]);
    }

    #[tokio::test]
    async fn list_group_members_unions_static_and_inner() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [ { "id": "s1", "display_name": "Static One", "username": "s1" } ],
              "groups": [ { "id": "g1", "display_name": "Group" } ],
              "memberships": [ { "group_id": "g1", "user_ids": ["s1"] } ]
            }
            "#,
        )
        .unwrap();
        let inner = MockInner {
            members: HashMap::from([("g1".to_string(), vec![test_user("u1", "Graph One")])]),
            ..Default::default()
        };
        let page = provider(dir, inner)
            .list_group_members("g1", &user_query())
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "u1"]);
    }

    #[tokio::test]
    async fn list_group_members_of_static_only_group() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [ { "id": "s1", "display_name": "Static One", "username": "s1" } ],
              "groups": [ { "id": "sg", "display_name": "Static Group" } ],
              "memberships": [ { "group_id": "sg", "user_ids": ["s1"] } ]
            }
            "#,
        )
        .unwrap();
        let page = provider(dir, MockInner::default())
            .list_group_members("sg", &user_query())
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["s1"]);
    }

    #[tokio::test]
    async fn list_user_groups_unions_static_and_inner() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "groups": [ { "id": "sg", "display_name": "Static Group" } ],
              "memberships": [ { "group_id": "sg", "user_ids": ["u1"] } ]
            }
            "#,
        )
        .unwrap();
        let inner = MockInner {
            user_groups: HashMap::from([("u1".to_string(), vec![test_group("g1", "Graph Group")])]),
            ..Default::default()
        };
        let page = provider(dir, inner)
            .list_user_groups("u1", &group_query())
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["sg", "g1"]);
    }

    #[tokio::test]
    async fn get_user_group_resolves_static_membership() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [ { "id": "u1", "display_name": "User One", "username": "u1" } ],
              "groups": [ { "id": "sg", "display_name": "Static Group" } ],
              "memberships": [ { "group_id": "sg", "user_ids": ["u1"] } ]
            }
            "#,
        )
        .unwrap();
        let p = provider(dir, MockInner::default());
        let group = p.get_user_group("u1", "sg").await.unwrap();
        assert_eq!(group.id, "sg");
        let err = p.get_user_group("u1", "other").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_user_group_with_unresolvable_user_is_not_found() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "groups": [ { "id": "sg", "display_name": "Static Group" } ],
              "memberships": [ { "group_id": "sg", "user_ids": ["ghost"] } ]
            }
            "#,
        )
        .unwrap();
        let err = provider(dir, MockInner::default())
            .get_user_group("ghost", "sg")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_group_member_resolves_static_membership() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [ { "id": "s1", "display_name": "Static One", "username": "s1" } ],
              "groups": [ { "id": "g1", "display_name": "Group" } ],
              "memberships": [ { "group_id": "g1", "user_ids": ["s1"] } ]
            }
            "#,
        )
        .unwrap();
        let user = provider(dir, MockInner::default())
            .get_group_member("g1", "s1")
            .await
            .unwrap();
        assert_eq!(user.id, "s1");
    }

    #[tokio::test]
    async fn get_group_member_with_unresolvable_group_is_not_found() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [ { "id": "s1", "display_name": "Static One", "username": "s1" } ],
              "memberships": [ { "group_id": "ghost", "user_ids": ["s1"] } ]
            }
            "#,
        )
        .unwrap();
        let err = provider(dir, MockInner::default())
            .get_group_member("ghost", "s1")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_users_paginates_static_then_inner() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [
                { "id": "s1", "display_name": "S1", "username": "s1" },
                { "id": "s2", "display_name": "S2", "username": "s2" }
              ]
            }
            "#,
        )
        .unwrap();
        let inner = MockInner {
            users: vec![test_user("u1", "U1")],
            ..Default::default()
        };
        let p = provider(dir, inner);

        let first = p
            .list_users(&UserQuery {
                email: None,
                display_name: None,
                limit: 2,
                cursor: None,
            })
            .await
            .unwrap();
        let ids: Vec<&str> = first.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "s2"]);
        assert!(first.next_cursor.is_some());

        let second = p
            .list_users(&UserQuery {
                email: None,
                display_name: None,
                limit: 2,
                cursor: first.next_cursor,
            })
            .await
            .unwrap();
        let ids: Vec<&str> = second.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["u1"]);
        assert!(second.next_cursor.is_none());
    }

    fn jwt_with_oid(oid: &str) -> String {
        #[derive(serde::Serialize)]
        struct Claims<'a> {
            oid: &'a str,
        }
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &Claims { oid },
            &jsonwebtoken::EncodingKey::from_secret(b"test"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn get_me_resolves_static_service_account() {
        let dir = StaticDirectory::from_json(
            r#"{ "users": [ { "id": "svc-1", "display_name": "Service Account", "username": "svc" } ] }"#,
        )
        .unwrap();
        let user = provider(dir, MockInner::default())
            .get_me(&jwt_with_oid("svc-1"))
            .await
            .unwrap();
        assert_eq!(user.id, "svc-1");
        assert_eq!(user.display_name, "Service Account");
    }

    #[tokio::test]
    async fn get_me_falls_back_to_inner() {
        let inner = MockInner {
            users: vec![test_user("u1", "Graph One")],
            ..Default::default()
        };
        let user = provider(StaticDirectory::default(), inner)
            .get_me(&jwt_with_oid("u1"))
            .await
            .unwrap();
        assert_eq!(user.id, "u1");
    }

    #[tokio::test]
    async fn list_users_filters_static_by_email() {
        let dir = StaticDirectory::from_json(
            r#"
            {
              "users": [
                { "id": "s1", "display_name": "S1", "username": "s1", "email": "match@example.com" },
                { "id": "s2", "display_name": "S2", "username": "s2", "email": "other@example.com" }
              ]
            }
            "#,
        )
        .unwrap();
        let page = provider(dir, MockInner::default())
            .list_users(&UserQuery {
                email: Some("match@example.com".into()),
                display_name: None,
                limit: 20,
                cursor: None,
            })
            .await
            .unwrap();
        let ids: Vec<&str> = page.items.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["s1"]);
    }
}
