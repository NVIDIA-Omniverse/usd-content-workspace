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
use std::sync::Arc;

use crate::models::{
    DisplayNameFilter, Group, GroupQuery, Page, User, UserQuery, parse_display_name_filter,
};
use crate::provider::{
    AppRoleAssignmentsSource, DeltaChange, DeltaSyncSource, GroupPageResult, IdentityProvider,
    MembershipChange, PageOutcome, ProviderError, ScopedDirectory, UserPageResult,
};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::time::{Duration, Instant};
use tracing::info;

use super::client::{EntraClient, EntraError, ListQuery};

pub struct EntraProvider {
    client: Arc<EntraClient>,
    max_page_size: u32,
}

impl EntraProvider {
    pub fn new(client: Arc<EntraClient>, max_page_size: u32) -> Self {
        Self {
            client,
            max_page_size,
        }
    }
}

fn odata_escape(value: &str) -> String {
    value.replace('\'', "''")
}

/// Escape a term that is interpolated inside the double-quoted clause of an
/// OData `$search` expression. Per Microsoft Graph's KQL rules the clause is
/// wrapped in `"..."` and embedded `"` or `\` must be prefixed with a
/// backslash; every other character is passed through and URL-encoded by the
/// HTTP client.
fn odata_search_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Decode an opaque pagination cursor back into the Graph `@odata.nextLink`
/// URL it wraps. The decoded URL is pinned to `expected_url` — the endpoint
/// the current operation calls for its first page: it must resolve to the
/// same origin (scheme, host, and port) and the same path, including any
/// route identifiers baked into `expected_url`. A cursor that points at a
/// different origin or a different Graph path is rejected, so a caller can
/// neither coerce the service into sending its Graph app token to an
/// unrelated host nor reuse a cursor to read a resource outside the operation
/// it was issued for (for example, another group's members).
fn decode_cursor(cursor: &str, expected_url: &str) -> Result<String, ProviderError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    let decoded = String::from_utf8(bytes)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    let cursor_url = reqwest::Url::parse(&decoded)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    let expected = reqwest::Url::parse(expected_url)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    if cursor_url.origin() != expected.origin() || cursor_url.path() != expected.path() {
        return Err(ProviderError::InvalidCursor("invalid cursor".into()));
    }
    Ok(decoded)
}

fn encode_cursor(next_link: &str) -> String {
    URL_SAFE_NO_PAD.encode(next_link)
}

fn build_display_name_query(display_name: Option<&str>) -> (Option<String>, Option<String>) {
    match display_name.map(parse_display_name_filter) {
        Some(DisplayNameFilter::Prefix(p)) => (
            Some(format!("startsWith(displayName,'{}')", odata_escape(p))),
            None,
        ),
        Some(DisplayNameFilter::Search(s)) => (
            None,
            Some(format!("\"displayName:{}\"", odata_search_escape(s))),
        ),
        None => (None, None),
    }
}

/// Build OData `$filter` and `$search` strings from a `UserQuery`.
fn build_user_query(params: &UserQuery) -> (Option<String>, Option<String>) {
    let mut filters: Vec<String> = Vec::new();
    if let Some(ref email) = params.email {
        filters.push(format!("mail eq '{}'", odata_escape(email)));
    }

    let mut search: Option<String> = None;
    if let Some(ref dn) = params.display_name {
        match parse_display_name_filter(dn) {
            DisplayNameFilter::Prefix(p) => {
                filters.push(format!("startsWith(displayName,'{}')", odata_escape(p)));
            }
            DisplayNameFilter::Search(s) => {
                search = Some(format!("\"displayName:{}\"", odata_search_escape(s)));
            }
        }
    }

    let filter = if filters.is_empty() {
        None
    } else {
        Some(filters.join(" and "))
    };

    (filter, search)
}

impl From<EntraError> for ProviderError {
    fn from(e: EntraError) -> Self {
        match e {
            EntraError::Api { status, body }
                if status == 410
                    || body.contains("syncStateNotFound")
                    || body.contains("resyncRequired") =>
            {
                Self::DeltaExpired
            }
            EntraError::InvalidToken(reason) => Self::Unauthorized(reason),
            EntraError::Api { status: 401, body } => Self::Unauthorized(body),
            EntraError::Api { status: 403, body } => Self::Forbidden(body),
            EntraError::Api { status: 404, body } => Self::NotFound(body),
            // Microsoft Graph rejects ID-by-id lookups whose identifier is not
            // a well-formed GUID with `400 Request_BadRequest /
            // "Invalid object identifier"` (and similar variants) instead of
            // `404`. From the user-info API contract perspective the
            // resource simply does not exist, so collapse these onto
            // `NotFound` to keep the contract uniform regardless of upstream
            // ID-validation quirks.
            EntraError::Api { status: 400, body } if is_graph_invalid_identifier(&body) => {
                Self::NotFound(body)
            }
            EntraError::Api { status, body } => {
                Self::Upstream(format!("Graph API {status}: {body}"))
            }
            EntraError::Http(e) => Self::Upstream(e.to_string()),
            EntraError::TokenFile { path, source } => Self::Upstream(format!(
                "failed to read federated token from {path}: {source}"
            )),
            EntraError::RateLimited { retry_after } => Self::RateLimited { retry_after },
        }
    }
}

/// Returns `true` when a Graph `400 Bad Request` body indicates that the
/// supplied identifier is malformed or refers to a non-existent object.
/// Used to fold these responses onto [`ProviderError::NotFound`] so the
/// public API contract stays uniform regardless of whether Graph chose to
/// validate the ID format up front (`400`) or look it up and miss (`404`).
fn is_graph_invalid_identifier(body: &str) -> bool {
    // Phrases observed from Graph; matched case-insensitively to stay
    // resilient to message-casing changes across API versions.
    const NEEDLES: &[&str] = &[
        "invalid object identifier",
        "request_resourcenotfound",
        "resource not found for the segment",
        "does not exist or one of its queried reference-property objects are not present",
    ];
    let lower = body.to_ascii_lowercase();
    NEEDLES.iter().any(|n| lower.contains(n))
}

#[async_trait]
impl IdentityProvider for EntraProvider {
    async fn list_users(&self, params: &UserQuery) -> Result<Page<User>, ProviderError> {
        let next_link = params
            .cursor
            .as_deref()
            .map(|c| decode_cursor(c, &self.client.list_users_url()))
            .transpose()?;
        let (filter, search) = build_user_query(params);

        let resp = self
            .client
            .list_users(&ListQuery {
                exact_filter: filter.as_deref(),
                text_search: search.as_deref(),
                page_size: params.limit.min(self.max_page_size),
                continuation_url: next_link.as_deref(),
            })
            .await?;

        Ok(Page {
            items: resp.value.into_iter().map(User::from).collect(),
            next_cursor: resp.next_link.as_deref().map(encode_cursor),
        })
    }

    async fn get_user(&self, user_id: &str) -> Result<User, ProviderError> {
        Ok(self.client.get_user(user_id).await?.into())
    }

    async fn get_me(&self, bearer_token: &str) -> Result<User, ProviderError> {
        Ok(self.client.get_me(bearer_token).await?.into())
    }

    async fn list_groups(&self, params: &GroupQuery) -> Result<Page<Group>, ProviderError> {
        let next_link = params
            .cursor
            .as_deref()
            .map(|c| decode_cursor(c, &self.client.list_groups_url()))
            .transpose()?;

        let (prefix_filter, search) = build_display_name_query(params.display_name.as_deref());

        let resp = self
            .client
            .list_groups(&ListQuery {
                exact_filter: prefix_filter.as_deref(),
                text_search: search.as_deref(),
                page_size: params.limit.min(self.max_page_size),
                continuation_url: next_link.as_deref(),
            })
            .await?;

        Ok(Page {
            items: resp.value.into_iter().map(Group::from).collect(),
            next_cursor: resp.next_link.as_deref().map(encode_cursor),
        })
    }

    async fn get_group(&self, group_id: &str) -> Result<Group, ProviderError> {
        Ok(self.client.get_group(group_id).await?.into())
    }

    async fn list_group_members(
        &self,
        group_id: &str,
        params: &UserQuery,
    ) -> Result<Page<User>, ProviderError> {
        let next_link = params
            .cursor
            .as_deref()
            .map(|c| decode_cursor(c, &self.client.list_group_members_url(group_id)))
            .transpose()?;
        let (filter, search) = build_user_query(params);

        let resp = self
            .client
            .list_group_members(
                group_id,
                &ListQuery {
                    exact_filter: filter.as_deref(),
                    text_search: search.as_deref(),
                    page_size: params.limit.min(self.max_page_size),
                    continuation_url: next_link.as_deref(),
                },
            )
            .await?;

        Ok(Page {
            items: resp.value.into_iter().map(User::from).collect(),
            next_cursor: resp.next_link.as_deref().map(encode_cursor),
        })
    }

    async fn list_user_groups(
        &self,
        user_id: &str,
        params: &GroupQuery,
    ) -> Result<Page<Group>, ProviderError> {
        let next_link = params
            .cursor
            .as_deref()
            .map(|c| decode_cursor(c, &self.client.list_user_groups_url(user_id)))
            .transpose()?;

        let (prefix_filter, search) = build_display_name_query(params.display_name.as_deref());

        let resp = self
            .client
            .list_user_groups(
                user_id,
                &ListQuery {
                    exact_filter: prefix_filter.as_deref(),
                    text_search: search.as_deref(),
                    page_size: params.limit.min(self.max_page_size),
                    continuation_url: next_link.as_deref(),
                },
            )
            .await?;

        Ok(Page {
            items: resp.value.into_iter().map(Group::from).collect(),
            next_cursor: resp.next_link.as_deref().map(encode_cursor),
        })
    }

    async fn get_user_group(&self, user_id: &str, group_id: &str) -> Result<Group, ProviderError> {
        let filter_str = format!("id eq '{}'", odata_escape(group_id));
        let resp = self
            .client
            .list_user_groups(
                user_id,
                &ListQuery {
                    exact_filter: Some(&filter_str),
                    page_size: 1,
                    ..Default::default()
                },
            )
            .await?;

        resp.value
            .into_iter()
            .next()
            .map(Group::from)
            .ok_or_else(|| {
                ProviderError::NotFound(format!(
                    "user {user_id} is not a member of group {group_id}"
                ))
            })
    }

    async fn get_group_member(&self, group_id: &str, user_id: &str) -> Result<User, ProviderError> {
        let filter_str = format!("id eq '{}'", odata_escape(user_id));
        let resp = self
            .client
            .list_group_members(
                group_id,
                &ListQuery {
                    exact_filter: Some(&filter_str),
                    page_size: 1,
                    ..Default::default()
                },
            )
            .await?;

        resp.value
            .into_iter()
            .next()
            .map(User::from)
            .ok_or_else(|| {
                ProviderError::NotFound(format!(
                    "user {user_id} is not a member of group {group_id}"
                ))
            })
    }
}

pub struct EntraDeltaSync {
    client: Arc<EntraClient>,
    memberships_enabled: bool,
}

impl EntraDeltaSync {
    pub fn new(client: Arc<EntraClient>, memberships_enabled: bool) -> Self {
        Self {
            client,
            memberships_enabled,
        }
    }
}

#[async_trait]
impl DeltaSyncSource for EntraDeltaSync {
    async fn fetch_user_page(&self, url: Option<&str>) -> Result<UserPageResult, ProviderError> {
        let resp = self.client.list_users_delta(url).await?;

        let mut changes = Vec::with_capacity(resp.value.len());
        for item in resp.value {
            if item.removed.is_some() {
                changes.push(DeltaChange::Remove(item.inner.id));
            } else {
                changes.push(DeltaChange::Upsert(User::from(item.inner)));
            }
        }

        let outcome = PageOutcome::from_links(resp.next_link, resp.delta_link)?;
        Ok(UserPageResult { changes, outcome })
    }

    async fn fetch_group_page(&self, url: Option<&str>) -> Result<GroupPageResult, ProviderError> {
        let resp = self
            .client
            .list_groups_delta(url, self.memberships_enabled)
            .await?;

        let mut changes = Vec::new();
        let mut membership_changes: Vec<MembershipChange> = Vec::new();

        for item in resp.value {
            let group_id = item.inner.id.clone();

            if item.removed.is_some() {
                changes.push(DeltaChange::Remove(group_id));
            } else {
                if !item.members_delta.is_empty() {
                    let mut added = Vec::new();
                    let mut removed = Vec::new();
                    for member in item.members_delta {
                        if member.removed.is_some() {
                            removed.push(member.id);
                        } else {
                            added.push(member.id);
                        }
                    }
                    membership_changes.push(MembershipChange {
                        group_id: group_id.clone(),
                        added_user_ids: added,
                        removed_user_ids: removed,
                    });
                }
                changes.push(DeltaChange::Upsert(Group::from(item.inner)));
            }
        }

        let outcome = PageOutcome::from_links(resp.next_link, resp.delta_link)?;
        Ok(GroupPageResult {
            changes,
            membership_changes,
            outcome,
        })
    }
}

/// Graph `principalType` value marking a group assignment.
const PRINCIPAL_TYPE_GROUP: &str = "Group";
/// Graph `principalType` value marking a service principal assignment.
const PRINCIPAL_TYPE_SERVICE_PRINCIPAL: &str = "ServicePrincipal";

/// Reads an enterprise application's `appRoleAssignedTo` collection and resolves
/// it into the assigned groups (with their direct members) and the assigned
/// service principals surfaced as users.
pub struct EntraAppRoleAssignments {
    client: Arc<EntraClient>,
    /// Service principal object id to query. When `None`, the authenticating
    /// application's own service principal is resolved from `app_id`.
    service_principal_object_id: Option<String>,
    /// `AZURE_CLIENT_ID` of the authenticating application, used to resolve the
    /// own service principal when no object id is configured.
    app_id: String,
    include_service_principals: bool,
    max_page_size: u32,
}

impl EntraAppRoleAssignments {
    pub fn new(
        client: Arc<EntraClient>,
        service_principal_object_id: Option<String>,
        app_id: String,
        include_service_principals: bool,
        max_page_size: u32,
    ) -> Self {
        Self {
            client,
            service_principal_object_id,
            app_id,
            include_service_principals,
            max_page_size,
        }
    }

    /// Resolve the service principal object id to query, using the configured id
    /// when set or resolving the authenticating application's own service
    /// principal otherwise.
    async fn resolve_service_principal_id(&self) -> Result<String, ProviderError> {
        match &self.service_principal_object_id {
            Some(id) => Ok(id.clone()),
            None => Ok(self
                .client
                .get_service_principal_id_by_app_id(&self.app_id)
                .await?),
        }
    }

    /// Page through the assigned principals, partitioning them into assigned
    /// group ids and assigned service principals (as users). Assignments whose
    /// principal is a user are ignored — they are already real directory users.
    async fn fetch_assigned_principals(
        &self,
        service_principal_object_id: &str,
    ) -> Result<(Vec<String>, Vec<User>), ProviderError> {
        let mut group_ids: Vec<String> = Vec::new();
        let mut seen_groups: HashSet<String> = HashSet::new();
        let mut service_principals: Vec<User> = Vec::new();
        let mut seen_service_principals: HashSet<String> = HashSet::new();

        let mut continuation: Option<String> = None;
        loop {
            let resp = self
                .client
                .list_app_role_assigned_to(service_principal_object_id, continuation.as_deref())
                .await?;

            for assignment in resp.value {
                match assignment.principal_type.as_deref() {
                    Some(PRINCIPAL_TYPE_GROUP) => {
                        if seen_groups.insert(assignment.principal_id.clone()) {
                            group_ids.push(assignment.principal_id);
                        }
                    }
                    Some(PRINCIPAL_TYPE_SERVICE_PRINCIPAL) if self.include_service_principals => {
                        if seen_service_principals.insert(assignment.principal_id.clone()) {
                            service_principals.push(service_principal_as_user(
                                assignment.principal_id,
                                assignment.principal_display_name,
                            ));
                        }
                    }
                    _ => {}
                }
            }

            match resp.next_link {
                Some(next) => continuation = Some(next),
                None => break,
            }
        }

        Ok((group_ids, service_principals))
    }

    /// Fetch the direct user members of a single assigned group, following
    /// pagination.
    async fn fetch_group_member_ids(
        &self,
        group_id: &str,
    ) -> Result<HashSet<String>, ProviderError> {
        let mut ids: HashSet<String> = HashSet::new();
        let mut next_link: Option<String> = None;
        loop {
            let resp = self
                .client
                .list_group_members(
                    group_id,
                    &ListQuery {
                        page_size: self.max_page_size,
                        continuation_url: next_link.as_deref(),
                        ..Default::default()
                    },
                )
                .await?;
            for member in resp.value {
                ids.insert(member.id);
            }
            match resp.next_link {
                Some(next) => next_link = Some(next),
                None => break,
            }
        }
        Ok(ids)
    }
}

fn fmt_elapsed(d: Duration) -> String {
    humantime::format_duration(Duration::from_millis(d.as_millis() as u64)).to_string()
}

/// Map an assigned service principal to a directory user. Service principals
/// have no user principal name, so the object id doubles as the username.
fn service_principal_as_user(id: String, display_name: Option<String>) -> User {
    User {
        display_name: display_name.unwrap_or_default(),
        username: id.clone(),
        id,
        given_name: None,
        family_name: None,
        email: None,
        active: Some(true),
        job_title: None,
    }
}

#[async_trait]
impl AppRoleAssignmentsSource for EntraAppRoleAssignments {
    async fn fetch(&self) -> Result<ScopedDirectory, ProviderError> {
        let sp_id = self.resolve_service_principal_id().await?;
        let (group_ids, service_principals) = self.fetch_assigned_principals(&sp_id).await?;

        let total_groups = group_ids.len();
        info!(
            total_groups,
            service_principal_count = service_principals.len(),
            "fetching assigned groups and memberships"
        );
        let start = Instant::now();

        let mut groups: Vec<Group> = Vec::with_capacity(total_groups);
        let mut members_by_group: HashMap<String, HashSet<String>> = HashMap::new();
        let mut total_membership_items: usize = 0;
        for (index, group_id) in group_ids.into_iter().enumerate() {
            let group = self.client.get_group(&group_id).await?;
            let member_ids = self.fetch_group_member_ids(&group_id).await?;
            let member_count = member_ids.len();
            total_membership_items += member_count;
            members_by_group.insert(group_id.clone(), member_ids);
            groups.push(group.into());

            info!(
                group_id = %group_id,
                member_count,
                groups_fetched = index + 1,
                total_groups,
                total_membership_items,
                "assigned group members fetched"
            );
        }

        info!(
            total_groups,
            total_membership_items,
            service_principal_count = service_principals.len(),
            elapsed = fmt_elapsed(start.elapsed()),
            "assigned groups and memberships fetch complete"
        );

        Ok(ScopedDirectory {
            groups,
            members_by_group,
            service_principals,
        })
    }

    async fn probe(&self) -> Result<(), ProviderError> {
        let sp_id = self.resolve_service_principal_id().await?;
        self.client.list_app_role_assigned_to(&sp_id, None).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_400(body: &str) -> ProviderError {
        ProviderError::from(EntraError::Api {
            status: 400,
            body: body.to_string(),
        })
    }

    #[test]
    fn graph_invalid_object_identifier_maps_to_not_found() {
        let body = r#"{"error":{"code":"Request_BadRequest","message":"Invalid object identifier 'not-a-guid'."}}"#;
        assert!(matches!(graph_400(body), ProviderError::NotFound(_)));
    }

    #[test]
    fn graph_resource_not_found_400_maps_to_not_found() {
        let body = r#"{"error":{"code":"Request_ResourceNotFound","message":"Resource '00000000-0000-0000-0000-000000000000' does not exist."}}"#;
        assert!(matches!(graph_400(body), ProviderError::NotFound(_)));
    }

    #[test]
    fn graph_resource_not_found_for_segment_400_maps_to_not_found() {
        let body = r#"{"error":{"code":"Request_BadRequest","message":"Resource not found for the segment 'foo'."}}"#;
        assert!(matches!(graph_400(body), ProviderError::NotFound(_)));
    }

    #[test]
    fn graph_match_is_case_insensitive() {
        let body = r#"{"error":{"message":"INVALID OBJECT IDENTIFIER 'x'."}}"#;
        assert!(matches!(graph_400(body), ProviderError::NotFound(_)));
    }

    #[test]
    fn other_400s_remain_upstream() {
        let body = r#"{"error":{"code":"Request_BadRequest","message":"Invalid filter clause."}}"#;
        let err = graph_400(body);
        assert!(
            matches!(err, ProviderError::Upstream(_)),
            "expected Upstream, got {err:?}"
        );
    }

    #[test]
    fn graph_404_still_maps_to_not_found() {
        let err = ProviderError::from(EntraError::Api {
            status: 404,
            body: "anything".to_string(),
        });
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[test]
    fn search_clause_escapes_double_quote_in_display_name() {
        let (filter, search) = build_display_name_query(Some("admin\" OR \"mail:admin"));
        assert!(filter.is_none());
        let search = search.expect("search clause");
        assert_eq!(
            search, "\"displayName:admin\\\" OR \\\"mail:admin\"",
            "double quotes inside the clause must be backslash-escaped so the \
             attacker-supplied content cannot close the wrapping quotes and open a \
             second clause"
        );
    }

    #[test]
    fn search_clause_escapes_trailing_backslash_in_display_name() {
        let (_, search) = build_display_name_query(Some("foo\\"));
        let search = search.expect("search clause");
        assert_eq!(
            search, "\"displayName:foo\\\\\"",
            "trailing backslash must be escaped so it does not consume the closing \
             double quote of the wrapping clause"
        );
    }

    #[test]
    fn search_clause_preserves_normal_display_name() {
        let (_, search) = build_display_name_query(Some("Alice Example"));
        assert_eq!(search.as_deref(), Some("\"displayName:Alice Example\""));
    }

    #[test]
    fn user_query_search_clause_escapes_double_quote_in_display_name() {
        let params = UserQuery {
            display_name: Some("admin\" OR \"mail:admin".to_string()),
            email: None,
            limit: 20,
            cursor: None,
        };
        let (_, search) = build_user_query(&params);
        let search = search.expect("search clause");
        assert_eq!(search, "\"displayName:admin\\\" OR \\\"mail:admin\"");
    }

    #[test]
    fn user_query_search_clause_escapes_backslash_in_display_name() {
        let params = UserQuery {
            display_name: Some("foo\\bar".to_string()),
            email: None,
            limit: 20,
            cursor: None,
        };
        let (_, search) = build_user_query(&params);
        assert_eq!(
            search.as_deref(),
            Some("\"displayName:foo\\\\bar\""),
            "literal backslashes inside the clause must be doubled so the KQL \
             parser does not interpret them as escape characters"
        );
    }

    const USERS_ENDPOINT_URL: &str = "https://graph.microsoft.com/v1.0/users";
    const GROUP_MEMBERS_ENDPOINT_URL: &str =
        "https://graph.microsoft.com/v1.0/groups/group-a/members/microsoft.graph.user";

    #[test]
    fn cursor_matching_endpoint_is_accepted() {
        let next_link = "https://graph.microsoft.com/v1.0/users?$skiptoken=abc";
        let cursor = encode_cursor(next_link);
        assert_eq!(
            decode_cursor(&cursor, USERS_ENDPOINT_URL).unwrap(),
            next_link
        );
    }

    #[test]
    fn cursor_cross_origin_url_is_rejected() {
        let cursor = encode_cursor("https://attacker.example/steal");
        assert!(matches!(
            decode_cursor(&cursor, USERS_ENDPOINT_URL),
            Err(ProviderError::InvalidCursor(_))
        ));
    }

    #[test]
    fn cursor_scheme_downgrade_is_rejected() {
        let cursor = encode_cursor("http://graph.microsoft.com/v1.0/users?$skiptoken=abc");
        assert!(matches!(
            decode_cursor(&cursor, USERS_ENDPOINT_URL),
            Err(ProviderError::InvalidCursor(_))
        ));
    }

    #[test]
    fn cursor_non_http_scheme_is_rejected() {
        let cursor = encode_cursor("file:///etc/passwd");
        assert!(matches!(
            decode_cursor(&cursor, USERS_ENDPOINT_URL),
            Err(ProviderError::InvalidCursor(_))
        ));
    }

    #[test]
    fn cursor_for_different_collection_is_rejected() {
        let cursor = encode_cursor("https://graph.microsoft.com/v1.0/groups?$skiptoken=abc");
        assert!(matches!(
            decode_cursor(&cursor, USERS_ENDPOINT_URL),
            Err(ProviderError::InvalidCursor(_))
        ));
    }

    #[test]
    fn cursor_matching_route_identifier_is_accepted() {
        let next_link = "https://graph.microsoft.com/v1.0/groups/group-a/members/microsoft.graph.user?$skiptoken=abc";
        let cursor = encode_cursor(next_link);
        assert_eq!(
            decode_cursor(&cursor, GROUP_MEMBERS_ENDPOINT_URL).unwrap(),
            next_link
        );
    }

    #[test]
    fn cursor_for_different_route_identifier_is_rejected() {
        let cursor = encode_cursor(
            "https://graph.microsoft.com/v1.0/groups/group-b/members/microsoft.graph.user?$skiptoken=abc",
        );
        assert!(matches!(
            decode_cursor(&cursor, GROUP_MEMBERS_ENDPOINT_URL),
            Err(ProviderError::InvalidCursor(_))
        ));
    }
}
