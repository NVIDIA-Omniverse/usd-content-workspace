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

use std::fmt::Debug;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, instrument, warn};

use crate::metrics;

use super::models::{
    GraphAppRoleAssignment, GraphDeltaResponse, GraphGroup, GraphGroupDeltaResponse,
    GraphListResponse, GraphServicePrincipalRef, GraphUser,
};
use crate::http::{self, HttpResponseError, Retryable};

const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(120);
const USER_SELECT: &str =
    "id,displayName,givenName,surname,mail,userPrincipalName,accountEnabled,jobTitle";
const GROUP_SELECT: &str = "id,displayName,description";
const APP_ROLE_ASSIGNED_TO_SELECT: &str = "principalId,principalDisplayName,principalType";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// OAuth2 credential used for token acquisition.
pub enum TokenCredential {
    ClientSecret(String),
    WorkloadIdentity { token_file: String },
}

/// Parameters for paginated Graph API collection queries.
#[derive(Debug, Default)]
pub struct ListQuery<'a> {
    /// Exact-match / comparison predicate passed as OData `$filter`
    /// (e.g. `"mail eq 'a@b.com'"`, `"id eq '...'"`).
    pub exact_filter: Option<&'a str>,
    /// Free-text / prefix search expression passed as OData `$search`
    /// (e.g. `"\"displayName:John\""`).
    pub text_search: Option<&'a str>,
    /// Maximum items per page (mapped to OData `$top`).
    pub page_size: u32,
    /// Opaque `@odata.nextLink` URL returned by a previous page; when set the
    /// other fields are ignored and this URL is called directly.
    pub continuation_url: Option<&'a str>,
}

/// Low-level HTTP client for the Microsoft Graph API. Handles OAuth2
/// client-credentials token acquisition with an in-memory cache and
/// provides typed helpers for user and group queries.
pub struct EntraClient {
    http: Client,
    client_id: String,
    credential: TokenCredential,
    token_cache: RwLock<Option<CachedToken>>,
    graph_base_url: String,
    token_url: String,
    max_retries: u32,
}

impl EntraClient {
    pub fn new(
        tenant_id: String,
        client_id: String,
        credential: TokenCredential,
        max_retries: u32,
    ) -> Self {
        let token_url = format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token");
        Self {
            http: Client::new(),
            graph_base_url: "https://graph.microsoft.com/v1.0".into(),
            token_url,
            client_id,
            credential,
            token_cache: RwLock::new(None),
            max_retries,
        }
    }

    #[cfg(test)]
    pub fn with_base_url(
        _tenant_id: String,
        client_id: String,
        client_secret: String,
        base_url: &str,
        max_retries: u32,
    ) -> Self {
        Self {
            http: Client::new(),
            graph_base_url: base_url.into(),
            token_url: format!("{base_url}/token"),
            client_id,
            credential: TokenCredential::ClientSecret(client_secret),
            token_cache: RwLock::new(None),
            max_retries,
        }
    }

    /// Return a valid app-level access token, using the cached value when still
    /// fresh or performing a client-credentials grant otherwise. Uses double-checked
    /// locking to avoid redundant token requests under concurrency.
    async fn acquire_token(&self) -> Result<String, EntraError> {
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache
                && Instant::now() < cached.expires_at - TOKEN_REFRESH_MARGIN
            {
                return Ok(cached.access_token.clone());
            }
        }

        let mut cache = self.token_cache.write().await;
        if let Some(ref cached) = *cache
            && Instant::now() < cached.expires_at - TOKEN_REFRESH_MARGIN
        {
            return Ok(cached.access_token.clone());
        }

        let resp = match &self.credential {
            TokenCredential::ClientSecret(secret) => {
                debug!("acquiring token via client secret");
                self.http
                    .post(&self.token_url)
                    .form(&[
                        ("grant_type", "client_credentials"),
                        ("client_id", &self.client_id),
                        ("client_secret", secret),
                        ("scope", "https://graph.microsoft.com/.default"),
                    ])
                    .send()
                    .await
                    .map_err(EntraError::Http)?
            }
            TokenCredential::WorkloadIdentity { token_file } => {
                debug!(
                    token_file,
                    "acquiring token via workload identity federation"
                );
                let assertion = tokio::fs::read_to_string(token_file)
                    .await
                    .map_err(|source| EntraError::TokenFile {
                        path: token_file.clone(),
                        source,
                    })?;
                self.http
                    .post(&self.token_url)
                    .form(&[
                        ("grant_type", "client_credentials"),
                        ("client_id", &self.client_id),
                        (
                            "client_assertion_type",
                            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                        ),
                        ("client_assertion", assertion.trim()),
                        ("scope", "https://graph.microsoft.com/.default"),
                    ])
                    .send()
                    .await
                    .map_err(EntraError::Http)?
            }
        };

        let resp: TokenResponse = http::classify_response(resp)
            .await?
            .json()
            .await
            .map_err(EntraError::Http)?;

        let token = resp.access_token.clone();
        *cache = Some(CachedToken {
            access_token: resp.access_token,
            expires_at: Instant::now() + Duration::from_secs(resp.expires_in),
        });
        Ok(token)
    }

    fn graph_url(&self, path: &str) -> String {
        format!("{}{path}", self.graph_base_url)
    }

    /// The configured Microsoft Graph base URL. Pagination cursors are pinned
    /// to this origin so a caller-supplied cursor cannot redirect an
    /// app-token-bearing request to an arbitrary host.
    pub fn graph_base_url(&self) -> &str {
        &self.graph_base_url
    }

    /// Build a Graph URL by appending `segments` to the configured base URL.
    /// Each segment is percent-encoded individually, so caller-supplied
    /// identifiers cannot escape into the query string or select a
    /// different resource by injecting `/`, `?`, `#`, or other reserved
    /// characters. `graph_base_url` is controlled by the service config
    /// (never user input) and is validated here to be a well-formed URL.
    fn graph_url_with_segments(&self, segments: &[&str]) -> String {
        let mut url =
            reqwest::Url::parse(&self.graph_base_url).expect("graph_base_url must be a valid URL");
        {
            let mut path = url
                .path_segments_mut()
                .expect("graph_base_url must support path segments");
            path.pop_if_empty();
            path.extend(segments);
        }
        url.into()
    }

    /// Execute a pre-built request, mapping non-2xx responses to `EntraError`
    /// and deserializing successful bodies into `T`.
    async fn send_request<T: serde::de::DeserializeOwned + Debug>(
        request: reqwest::RequestBuilder,
    ) -> Result<T, EntraError> {
        let resp = request.send().await.map_err(EntraError::Http)?;
        let resp = http::classify_response(resp).await?;
        resp.json().await.map_err(EntraError::Http)
    }

    /// Perform an authenticated GET against the Graph API using the app token.
    /// Adds `ConsistencyLevel: eventual` for `$search` / `$count` compatibility.
    async fn graph_get<T: serde::de::DeserializeOwned + Debug>(
        &self,
        url: &str,
        query: &[(String, String)],
        operation: &str,
    ) -> Result<T, EntraError> {
        let start = std::time::Instant::now();
        let mut attempt = 0u32;
        loop {
            let token = self.acquire_token().await?;
            let mut trace_headers = reqwest::header::HeaderMap::new();
            crate::telemetry::inject_trace_context(&mut trace_headers);
            let request_id = crate::telemetry::client_request_id();
            match Self::send_request(
                self.http
                    .get(url)
                    .bearer_auth(&token)
                    .header("ConsistencyLevel", "eventual")
                    .header("client-request-id", &request_id)
                    .headers(trace_headers)
                    .query(query),
            )
            .await
            {
                Ok(v) => {
                    metrics::record_graph_request(operation, "ok", start.elapsed().as_secs_f64());
                    return Ok(v);
                }
                Err(e) if attempt < self.max_retries && e.is_retryable() => {
                    let reason = retry_reason(&e);
                    metrics::record_graph_retry(operation, reason);
                    let wait = http::retry_delay(attempt, e.retry_after());
                    warn!(attempt, ?wait, %e, "retryable Graph API error, backing off");
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(e) => {
                    metrics::record_graph_request(
                        operation,
                        &error_status_label(&e),
                        start.elapsed().as_secs_f64(),
                    );
                    return Err(e);
                }
            }
        }
    }

    /// Assemble the OData query parameters shared by all collection endpoints:
    /// `$top`, `$select`, `$count`, and optional `$filter` / `$search`.
    fn build_list_query(select: &str, q: &ListQuery<'_>) -> Vec<(String, String)> {
        let mut query = vec![
            ("$top".into(), q.page_size.to_string()),
            ("$select".into(), select.into()),
            ("$count".into(), "true".into()),
        ];
        if let Some(f) = q.exact_filter {
            query.push(("$filter".into(), f.into()));
        }
        if let Some(s) = q.text_search {
            query.push(("$search".into(), s.into()));
        }
        query
    }

    /// Fetch a single page from a Graph API collection. If `next_link` is present
    /// the opaque `@odata.nextLink` URL is called directly; otherwise the supplied
    /// `url` is called with the shared `$top`/`$select`/`$count`/`$filter`/`$search`
    /// query parameters. Callers are responsible for constructing `url` with proper
    /// percent-encoding of any caller-supplied path segments.
    async fn graph_list<T: serde::de::DeserializeOwned + Debug>(
        &self,
        url: &str,
        select: &str,
        q: &ListQuery<'_>,
        operation: &str,
    ) -> Result<GraphListResponse<T>, EntraError> {
        if let Some(link) = q.continuation_url {
            return self.graph_get(link, &[], operation).await;
        }
        let query = Self::build_list_query(select, q);
        self.graph_get(url, &query, operation).await
    }

    /// Endpoint URL a paginated list operation targets for its first page. The
    /// value doubles as the reference a decoded pagination cursor is pinned to:
    /// a cursor must resolve to the same origin and path as the operation that
    /// issued it, so these mirror the `list_*` URLs exactly.
    pub(crate) fn list_users_url(&self) -> String {
        self.graph_url("/users")
    }

    pub(crate) fn list_groups_url(&self) -> String {
        self.graph_url("/groups")
    }

    pub(crate) fn list_group_members_url(&self, group_id: &str) -> String {
        self.graph_url_with_segments(&["groups", group_id, "members", "microsoft.graph.user"])
    }

    pub(crate) fn list_user_groups_url(&self, user_id: &str) -> String {
        self.graph_url_with_segments(&["users", user_id, "memberOf", "microsoft.graph.group"])
    }

    /// List directory users with optional email/display-name filtering and cursor pagination.
    #[instrument(skip(self, q))]
    pub async fn list_users(
        &self,
        q: &ListQuery<'_>,
    ) -> Result<GraphListResponse<GraphUser>, EntraError> {
        let url = self.list_users_url();
        self.graph_list(&url, USER_SELECT, q, "list_users").await
    }

    /// Fetch a single user by ID.
    #[instrument(skip(self))]
    pub async fn get_user(&self, user_id: &str) -> Result<GraphUser, EntraError> {
        let url = self.graph_url_with_segments(&["users", user_id]);
        self.graph_get(&url, &[("$select".into(), USER_SELECT.into())], "get_user")
            .await
    }

    /// Resolve the caller's identity by extracting the `oid` claim from the
    /// bearer JWT and looking the user up via `/users/{oid}` with the
    /// app-level token.
    #[instrument(skip(self, bearer_token))]
    pub async fn get_me(&self, bearer_token: &str) -> Result<GraphUser, EntraError> {
        let oid = http::decode_token_oid(bearer_token)
            .ok_or_else(|| EntraError::InvalidToken("missing or invalid oid claim".into()))?;
        self.get_user(&oid).await
    }

    /// List directory groups with optional display-name filtering and cursor pagination.
    #[instrument(skip(self, q))]
    pub async fn list_groups(
        &self,
        q: &ListQuery<'_>,
    ) -> Result<GraphListResponse<GraphGroup>, EntraError> {
        let url = self.list_groups_url();
        self.graph_list(&url, GROUP_SELECT, q, "list_groups").await
    }

    /// Fetch a single group by ID.
    #[instrument(skip(self))]
    pub async fn get_group(&self, group_id: &str) -> Result<GraphGroup, EntraError> {
        let url = self.graph_url_with_segments(&["groups", group_id]);
        self.graph_get(
            &url,
            &[("$select".into(), GROUP_SELECT.into())],
            "get_group",
        )
        .await
    }

    /// List the user members of a group, with optional filtering and pagination.
    #[instrument(skip(self, q))]
    pub async fn list_group_members(
        &self,
        group_id: &str,
        q: &ListQuery<'_>,
    ) -> Result<GraphListResponse<GraphUser>, EntraError> {
        let url = self.list_group_members_url(group_id);
        self.graph_list(&url, USER_SELECT, q, "list_group_members")
            .await
    }

    /// List the groups a user belongs to (via `memberOf`), with optional filtering and pagination.
    #[instrument(skip(self, q))]
    pub async fn list_user_groups(
        &self,
        user_id: &str,
        q: &ListQuery<'_>,
    ) -> Result<GraphListResponse<GraphGroup>, EntraError> {
        let url = self.list_user_groups_url(user_id);
        self.graph_list(&url, GROUP_SELECT, q, "list_user_groups")
            .await
    }

    /// List the principals assigned to an enterprise application via
    /// `GET /servicePrincipals/{id}/appRoleAssignedTo`, following
    /// `@odata.nextLink` pagination. Pass `None` to fetch the first page or the
    /// opaque continuation URL from a previous page.
    ///
    /// When the service authenticates as the enterprise application itself and
    /// queries that application's own service principal, Graph returns the
    /// assignments without the `Application.Read.All` permission. Querying a
    /// different application's service principal requires that permission and
    /// otherwise fails with `403`.
    #[instrument(skip(self))]
    pub async fn list_app_role_assigned_to(
        &self,
        service_principal_object_id: &str,
        continuation_url: Option<&str>,
    ) -> Result<GraphListResponse<GraphAppRoleAssignment>, EntraError> {
        if let Some(link) = continuation_url {
            return self.graph_get(link, &[], "list_app_role_assigned_to").await;
        }
        let url = self.graph_url_with_segments(&[
            "servicePrincipals",
            service_principal_object_id,
            "appRoleAssignedTo",
        ]);
        let query = vec![("$select".into(), APP_ROLE_ASSIGNED_TO_SELECT.into())];
        self.graph_get(&url, &query, "list_app_role_assigned_to")
            .await
    }

    /// Resolve the object id of the service principal that backs `app_id`
    /// (`AZURE_CLIENT_ID`) via `GET /servicePrincipals(appId='{app_id}')`.
    /// Reading the authenticating application's own service principal does not
    /// require any application permission.
    #[instrument(skip(self))]
    pub async fn get_service_principal_id_by_app_id(
        &self,
        app_id: &str,
    ) -> Result<String, EntraError> {
        let escaped = app_id.replace('\'', "''");
        let url = format!(
            "{}/servicePrincipals(appId='{escaped}')",
            self.graph_base_url
        );
        let sp: GraphServicePrincipalRef = self
            .graph_get(
                &url,
                &[("$select".into(), "id".into())],
                "get_service_principal_by_app_id",
            )
            .await?;
        Ok(sp.id)
    }

    /// Fetch a single page of user deltas. Pass `None` for the initial full
    /// delta sync, a `@odata.nextLink` to continue pagination, or a
    /// `@odata.deltaLink` to start an incremental sync.
    ///
    /// `$top` is not supported by the Graph user/group delta endpoints and is
    /// therefore not sent; page size is determined by Graph.
    #[instrument(skip(self))]
    pub async fn list_users_delta(
        &self,
        url: Option<&str>,
    ) -> Result<GraphDeltaResponse<GraphUser>, EntraError> {
        if let Some(url) = url {
            return self.graph_get(url, &[], "list_users_delta").await;
        }
        let url = self.graph_url("/users/delta");
        let query = vec![("$select".into(), USER_SELECT.into())];
        self.graph_get(&url, &query, "list_users_delta").await
    }

    /// Fetch a single page of group deltas. Pass `None` for the initial full delta
    /// sync, a `@odata.nextLink` to continue pagination, or a `@odata.deltaLink` to
    /// start an incremental sync.
    ///
    /// When `include_members` is `true`, `members` is added to `$select` so that
    /// Graph returns a `members@delta` array alongside each group object. For groups
    /// with many members the same group may appear across several `@odata.nextLink`
    /// pages, each carrying a different subset of `members@delta`; callers must
    /// accumulate membership changes from all pages.
    ///
    /// `include_members` is only consulted for the initial request (`url = None`);
    /// subsequent pages follow `@odata.nextLink` URLs that already have `$select`
    /// baked in.
    ///
    /// `$top` is not supported by the Graph user/group delta endpoints and is
    /// therefore not sent; page size is determined by Graph.
    #[instrument(skip(self))]
    pub async fn list_groups_delta(
        &self,
        url: Option<&str>,
        include_members: bool,
    ) -> Result<GraphGroupDeltaResponse, EntraError> {
        if let Some(url) = url {
            return self.graph_get(url, &[], "list_groups_delta").await;
        }
        let url = self.graph_url("/groups/delta");
        let select = if include_members {
            format!("{GROUP_SELECT},members")
        } else {
            GROUP_SELECT.to_string()
        };
        let query = vec![("$select".into(), select)];
        self.graph_get(&url, &query, "list_groups_delta").await
    }
}

/// Errors returned by Entra ID operations — transport failures, non-2xx API
/// responses, rate-limiting, or inability to read a federated token file.
#[derive(Debug)]
pub enum EntraError {
    Http(reqwest::Error),
    Api {
        status: u16,
        body: String,
    },
    RateLimited {
        retry_after: u64,
    },
    TokenFile {
        path: String,
        source: std::io::Error,
    },
    InvalidToken(String),
}

impl From<HttpResponseError> for EntraError {
    fn from(e: HttpResponseError) -> Self {
        match e {
            HttpResponseError::RateLimited { retry_after } => Self::RateLimited { retry_after },
            HttpResponseError::Api { status, body } => Self::Api { status, body },
        }
    }
}

impl Retryable for EntraError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } => true,
            Self::Api { status, .. } => matches!(status, 503 | 504),
            _ => false,
        }
    }

    fn retry_after(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after } => Some(*retry_after),
            _ => None,
        }
    }
}

impl std::fmt::Display for EntraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Api { status, body } => write!(f, "Graph API {status}: {body}"),
            Self::RateLimited { retry_after } => {
                write!(f, "rate limited (retry after {retry_after}s)")
            }
            Self::TokenFile { path, source } => {
                write!(f, "failed to read federated token from {path}: {source}")
            }
            Self::InvalidToken(reason) => write!(f, "invalid token: {reason}"),
        }
    }
}

impl std::error::Error for EntraError {}

fn retry_reason(e: &EntraError) -> &'static str {
    match e {
        EntraError::RateLimited { .. } => "429",
        EntraError::Api { status: 503, .. } => "503",
        EntraError::Api { status: 504, .. } => "504",
        _ => "unknown",
    }
}

fn error_status_label(e: &EntraError) -> String {
    match e {
        EntraError::Api { status, .. } => status.to_string(),
        EntraError::RateLimited { .. } => "429".into(),
        EntraError::Http(_) => "network_error".into(),
        EntraError::TokenFile { .. } => "token_file_error".into(),
        EntraError::InvalidToken(_) => "invalid_token".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> EntraClient {
        EntraClient::with_base_url(
            "tenant".into(),
            "client".into(),
            "secret".into(),
            "https://graph.microsoft.com/v1.0",
            0,
        )
    }

    #[test]
    fn graph_url_with_segments_preserves_base_path() {
        let url = test_client()
            .graph_url_with_segments(&["users", "00000000-0000-0000-0000-000000000001"]);
        assert_eq!(
            url,
            "https://graph.microsoft.com/v1.0/users/00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn graph_url_with_segments_percent_encodes_reserved_characters() {
        // Slashes, `?` and `#` in a caller-supplied ID must be escaped so the
        // segment cannot reach a different Graph endpoint or introduce query /
        // fragment components.
        let url = test_client().graph_url_with_segments(&[
            "users",
            "me/memberOf/microsoft.graph.group?$select=displayName#frag",
        ]);
        assert_eq!(
            url,
            "https://graph.microsoft.com/v1.0/users/me%2FmemberOf%2Fmicrosoft.graph.group%3F$select=displayName%23frag"
        );
    }

    #[test]
    fn graph_url_with_segments_handles_multiple_dynamic_segments() {
        let url = test_client().graph_url_with_segments(&[
            "groups",
            "g/../x",
            "members",
            "microsoft.graph.user",
        ]);
        assert_eq!(
            url,
            "https://graph.microsoft.com/v1.0/groups/g%2F..%2Fx/members/microsoft.graph.user"
        );
    }
}
