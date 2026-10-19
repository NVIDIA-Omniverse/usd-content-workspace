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

//! CLI / environment configuration and bearer-token acquisition for the
//! conformance test. The test only needs a valid bearer token; the acquisition
//! modes are modelled on the permission-service `serviceIdentity` chart
//! section so the same deployment conventions apply:
//!
//! 1. A direct `--bearer-token` (local development convenience).
//! 2. An `--access-token-file` JSON document populated by a sidecar /
//!    token-provisioner at deployment time (see `serviceIdentity.accessTokenFile`
//!    in the Helm chart); the conformance test is a one-shot Job, so the file is
//!    read once at startup.
//! 3. The OAuth2 client-credentials flow driven by OpenID Connect Discovery:
//!    the test fetches the OpenID provider metadata at
//!    `--oauth2-openid-configuration-uri`, extracts `token_endpoint`, and
//!    performs the token exchange against it. Matches
//!    `serviceIdentity.clientCredentials.openIdConfigurationUri` in the
//!    chart, removing the need to configure a separate token URL.

use crate::{HINT, TestError, style};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, clap::ValueEnum, PartialEq)]
pub enum TestSuite {
    #[default]
    All,
    Rest,
    Grpc,
}

/// Configuration for the conformance test runner.
///
/// The runner is data-agnostic: it samples all test fixtures (sample user,
/// sample group, membership pair, etc.) from the target service itself and
/// then asserts that responses satisfy the published REST and gRPC API
/// contract. There is no per-deployment configuration of identifiers, emails,
/// or display names.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Conformance test for any implementation of the User Info Service API",
    long_about = None,
)]
pub struct Args {
    #[arg(long, env = "REST_URL")]
    pub rest_url: Option<String>,

    /// gRPC endpoint, accepts `host:port` or `scheme://host:port`.
    #[arg(long, env = "GRPC_URL")]
    pub grpc_url: Option<String>,

    #[arg(long, env = "GRPC_TLS_ENABLED", default_value = "false")]
    pub grpc_tls_enabled: bool,

    #[arg(long, env = "TEST_SUITE", value_enum, default_value_t = TestSuite::All)]
    pub test_suite: TestSuite,

    /// Path on the REST URL used by the readiness wait loop. Set to a
    /// well-known probe such as `/readyz` if the implementation exposes one;
    /// otherwise set to a documented `/api/v1alpha/...` path that returns 2xx
    /// once the directory cache is populated.
    #[arg(long, env = "READY_PATH", default_value = "/readyz")]
    pub ready_path: String,

    /// Maximum number of items the conformance test will request per page.
    /// Pagination tests will additionally exercise `limit=1` and `page_size=1`.
    /// Fixture sampling scans up to this many users when looking for a user with an
    /// email and for a user that has at least one direct group membership.
    #[arg(long, env = "PAGE_LIMIT", default_value = "20")]
    pub page_limit: u32,

    /// Per-request HTTP / gRPC timeout in seconds.
    #[arg(long, env = "HTTP_TIMEOUT", default_value = "30")]
    pub http_timeout: u64,

    #[arg(long, env = "WAIT_RETRIES", default_value = "10")]
    pub wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value = "5")]
    pub wait_interval: u64,

    /// Direct bearer token, passed on the command line or in the
    /// `BEARER_TOKEN` environment variable. Intended for local development;
    /// in-cluster deployments should use `--access-token-file` or the
    /// OAuth2 client-credentials flags below instead.
    #[arg(long, env = "BEARER_TOKEN")]
    pub bearer_token: Option<String>,

    /// Path to a JSON document with `access_token` and `expires_on` fields.
    /// Typically populated by a token-provisioner sidecar — see
    /// `serviceIdentity.accessTokenFile.provisionerSidecar` in the Helm chart.
    /// The file is read once at startup.
    #[arg(long, env = "ACCESS_TOKEN_FILE")]
    pub access_token_file: Option<String>,

    /// OAuth2 client-credentials flow: OpenID Connect Discovery URI.
    ///
    /// The test issues `GET <uri>` to retrieve the identity provider's
    /// metadata document (see
    /// <https://openid.net/specs/openid-connect-discovery-1_0.html>) and
    /// uses the `token_endpoint` field from the response for the token
    /// exchange. Pass the full metadata URL, e.g.
    /// `https://login.microsoftonline.com/<tenant>/v2.0/.well-known/openid-configuration`.
    #[arg(long, env = "OAUTH2_OPENID_CONFIGURATION_URI")]
    pub oauth2_openid_configuration_uri: Option<String>,

    /// OAuth2 client-credentials flow: client identifier.
    #[arg(long, env = "OAUTH2_CLIENT_ID")]
    pub oauth2_client_id: Option<String>,

    /// OAuth2 client-credentials flow: client secret value.
    #[arg(long, env = "OAUTH2_CLIENT_SECRET")]
    pub oauth2_client_secret: Option<String>,

    /// OAuth2 client-credentials flow: file containing the client secret.
    /// Mutually exclusive with `--oauth2-client-secret`.
    #[arg(long, env = "OAUTH2_CLIENT_SECRET_FILE")]
    pub oauth2_client_secret_file: Option<String>,

    /// OAuth2 client-credentials flow: requested scope value
    /// (e.g. `<app-id>/.default`).
    #[arg(long, env = "OAUTH2_CLIENT_SCOPE")]
    pub oauth2_client_scope: Option<String>,

    /// OAuth2 client-credentials flow: additional HTTP headers to include
    /// in the token exchange request, as a JSON object
    /// (e.g. `{"X-Custom-Header": "value"}`).
    #[arg(long, env = "OAUTH2_EXTRA_HEADERS", default_value = "{}")]
    pub oauth2_extra_headers: String,

    /// OAuth2 client-credentials flow: additional body parameters to merge
    /// into the token exchange request, as a JSON object
    /// (e.g. `{"audience": "https://api.example.com"}`).
    #[arg(long, env = "OAUTH2_EXTRA_BODY_PARAMS", default_value = "{}")]
    pub oauth2_extra_body_params: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct AccessTokenFile {
    access_token: String,
    expires_on: DateTime<Utc>,
}

/// Subset of the OpenID Connect Discovery metadata document used by the
/// conformance test. See the OIDC Discovery 1.0 spec, section "OpenID
/// Provider Metadata".
#[derive(Debug, Deserialize)]
struct OpenIdProviderMetadata {
    token_endpoint: String,
}

/// Acquires a bearer token from the configured source. Returns `Ok(None)` when
/// no credentials are configured, in which case the test runs unauthenticated.
pub async fn acquire_bearer_token(
    http: &reqwest::Client,
    args: &Args,
) -> Result<Option<String>, TestError> {
    let info = style::info_marker();
    if let Some(token) = args.bearer_token.as_ref().filter(|t| !t.is_empty()) {
        println!("   {info} bearer token loaded from --bearer-token");
        return Ok(Some(token.clone()));
    }
    if let Some(path) = args.access_token_file.as_ref().filter(|p| !p.is_empty()) {
        let raw = tokio::fs::read_to_string(Path::new(path))
            .await
            .map_err(|e| {
                TestError::new(
                    format!("Failed to read --access-token-file '{path}': {e}"),
                    format!(
                        "{HINT}   * The path must exist and be readable by the test runner\n\
                     {HINT}   * In Helm, the file is populated by the sidecar configured under\n\
                     {HINT}     `serviceIdentity.accessTokenFile.provisionerSidecar`",
                    ),
                )
            })?;
        let token = parse_access_token_file(path, &raw)?;
        if token.is_empty() {
            return Err(TestError::new(
                format!("Access token file '{path}' contains an empty access_token"),
                format!(
                    "{HINT}   * The token-provisioner sidecar has not written the token yet, or\n\
                     {HINT}     wrote an empty file. Inspect the sidecar logs.",
                ),
            ));
        }
        println!("   {info} bearer token loaded from {path}");
        return Ok(Some(token));
    }

    if args
        .oauth2_openid_configuration_uri
        .as_deref()
        .is_some_and(|s| !s.is_empty())
        && args
            .oauth2_client_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    {
        let token = oauth2_client_credentials(http, args).await?;
        return Ok(Some(token));
    }

    println!("   {info} no bearer credentials configured; running unauthenticated");
    Ok(None)
}

fn parse_access_token_file(path: &str, raw: &str) -> Result<String, TestError> {
    let token: AccessTokenFile = serde_json::from_str(raw).map_err(|e| {
        TestError::new(
            format!("Access token file '{path}' is not valid JSON: {e}"),
            format!(
                "{HINT}   * The file must contain `access_token` and `expires_on` fields\n\
                 {HINT}   * `expires_on` must be an RFC3339 timestamp"
            ),
        )
    })?;
    if token.expires_on <= Utc::now() {
        return Err(TestError::new(
            format!("Access token file '{path}' expired at {}", token.expires_on),
            format!("{HINT}   * The token-provisioner sidecar must refresh the file before expiry"),
        ));
    }
    Ok(token.access_token.trim().to_string())
}

/// Fetches the OpenID Connect Discovery metadata document at `uri` and
/// returns the `token_endpoint` advertised by the identity provider.
async fn discover_token_endpoint(http: &reqwest::Client, uri: &str) -> Result<String, TestError> {
    let info = style::info_marker();
    println!("   {info} fetching OpenID configuration from {uri}");
    let resp = http.get(uri).send().await.map_err(|e| {
        TestError::new(
            format!("OpenID configuration request to {uri} failed: {e}"),
            format!(
                "{HINT}   * Confirm OAUTH2_OPENID_CONFIGURATION_URI points at the provider's\n\
                 {HINT}     OpenID metadata endpoint (typically ending in\n\
                 {HINT}     /.well-known/openid-configuration) and is reachable from this pod",
            ),
        )
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body_preview: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        return Err(TestError::new(
            format!(
                "OpenID configuration endpoint {uri} returned HTTP {}",
                status.as_u16()
            ),
            format!("{HINT}   * Response body: {body_preview}"),
        ));
    }
    let metadata: OpenIdProviderMetadata = resp.json().await.map_err(|e| {
        TestError::new(
            format!(
                "OpenID configuration endpoint {uri} returned an invalid metadata document: {e}"
            ),
            format!(
                "{HINT}   * The document must be a JSON object with at least a\n\
                 {HINT}     `token_endpoint` field (OIDC Discovery 1.0, Provider Metadata)",
            ),
        )
    })?;
    if metadata.token_endpoint.is_empty() {
        return Err(TestError::new(
            format!("OpenID configuration document at {uri} has an empty `token_endpoint`"),
            format!(
                "{HINT}   * The identity provider metadata must advertise a non-empty\n\
                 {HINT}     token_endpoint for the client-credentials flow",
            ),
        ));
    }
    Ok(metadata.token_endpoint)
}

/// Performs the OAuth2 client-credentials flow and returns the access token.
///
/// The token endpoint URL is resolved dynamically via OpenID Connect
/// Discovery: the caller supplies the provider's openid-configuration URI
/// and the test reads `token_endpoint` from the returned metadata.
async fn oauth2_client_credentials(
    http: &reqwest::Client,
    args: &Args,
) -> Result<String, TestError> {
    let oidc_uri = args.oauth2_openid_configuration_uri.as_ref().unwrap();
    let client_id = args.oauth2_client_id.as_ref().unwrap();

    let client_secret = if let Some(s) =
        args.oauth2_client_secret.as_ref().filter(|s| !s.is_empty())
    {
        s.clone()
    } else if let Some(p) = args
        .oauth2_client_secret_file
        .as_ref()
        .filter(|s| !s.is_empty())
    {
        tokio::fs::read_to_string(p)
            .await
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                TestError::new(
                    format!("Failed to read --oauth2-client-secret-file '{p}': {e}"),
                    format!("{HINT}   * The path must exist and be readable by the test runner",),
                )
            })?
    } else {
        return Err(TestError::new(
            "OAuth2 client-credentials flow needs --oauth2-client-secret or --oauth2-client-secret-file",
            format!(
                "{HINT}   * Either provide the secret directly via OAUTH2_CLIENT_SECRET\n\
                 {HINT}   * Or mount it as a file and set OAUTH2_CLIENT_SECRET_FILE",
            ),
        ));
    };

    let extra_headers = parse_json_string_map(&args.oauth2_extra_headers, "OAUTH2_EXTRA_HEADERS")?;
    let extra_body_params =
        parse_json_string_map(&args.oauth2_extra_body_params, "OAUTH2_EXTRA_BODY_PARAMS")?;

    let token_url = discover_token_endpoint(http, oidc_uri).await?;

    let mut form: Vec<(String, String)> = vec![
        ("grant_type".into(), "client_credentials".into()),
        ("client_id".into(), client_id.clone()),
        ("client_secret".into(), client_secret),
    ];
    if let Some(scope) = args.oauth2_client_scope.as_ref().filter(|s| !s.is_empty()) {
        form.push(("scope".into(), scope.clone()));
    }
    for (k, v) in extra_body_params {
        form.push((k, v));
    }

    let info = style::info_marker();
    println!("   {info} requesting OAuth2 token from {token_url}");
    let mut req = http.post(&token_url).form(&form);
    for (k, v) in &extra_headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| {
        TestError::new(
            format!("OAuth2 token request to {token_url} failed: {e}"),
            format!(
                "{HINT}   * Confirm the token endpoint URL and network reachability\n\
                 {HINT}   * Verify the credentials have not been rotated or revoked",
            ),
        )
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let body_preview: String = body.chars().take(400).collect();
        return Err(TestError::new(
            format!("OAuth2 token endpoint returned HTTP {}", status.as_u16()),
            format!(
                "{HINT}   * Verify --oauth2-client-id / --oauth2-client-secret / --oauth2-client-scope\n\
                 {HINT}   * Response body: {body_preview}",
            ),
        ));
    }
    let token: TokenResponse = resp.json().await.map_err(|e| {
        TestError::new(
            format!("OAuth2 token endpoint returned a non-JSON body: {e}"),
            format!("{HINT}   * The token endpoint must return a JSON object with `access_token`"),
        )
    })?;
    if token.access_token.is_empty() {
        return Err(TestError::new(
            "OAuth2 token endpoint returned an empty access_token",
            format!("{HINT}   * Inspect the identity provider configuration"),
        ));
    }
    println!("   {info} OAuth2 access token acquired");
    Ok(token.access_token)
}

/// Parses a JSON object of string values into a map. Accepts `""`, `"{}"`
/// or a full JSON object literal. Rejects arrays, nulls, and non-string
/// values so the caller can forward the entries verbatim as HTTP headers
/// or as `application/x-www-form-urlencoded` body pairs.
fn parse_json_string_map(
    raw: &str,
    source_label: &str,
) -> Result<HashMap<String, String>, TestError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str::<HashMap<String, String>>(trimmed).map_err(|e| {
        TestError::new(
            format!("{source_label} must be a JSON object of string values: {e}"),
            format!("{HINT}   * Example: {{\"audience\": \"https://api.example.com\"}}",),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> HashMap<String, String> {
        match parse_json_string_map(raw, "X") {
            Ok(m) => m,
            Err(e) => panic!("parse_json_string_map({raw:?}) failed: {}", e.message),
        }
    }

    fn future_rfc3339() -> String {
        (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()
    }

    #[test]
    fn parse_empty_json_map() {
        assert!(ok("").is_empty());
        assert!(ok("{}").is_empty());
        assert!(ok("   ").is_empty());
    }

    #[test]
    fn parse_json_map_with_entries() {
        let m = ok(r#"{"a":"1","b":"two"}"#);
        assert_eq!(m.get("a").map(String::as_str), Some("1"));
        assert_eq!(m.get("b").map(String::as_str), Some("two"));
    }

    #[test]
    fn parse_json_map_rejects_non_string_values() {
        assert!(parse_json_string_map(r#"{"a":1}"#, "X").is_err());
        assert!(parse_json_string_map(r#"{"a":null}"#, "X").is_err());
        assert!(parse_json_string_map(r#"["a","b"]"#, "X").is_err());
    }

    #[test]
    fn parse_access_token_file_reads_json_token() {
        let raw = serde_json::json!({
            "access_token": "conformance-token",
            "expires_on": future_rfc3339(),
        })
        .to_string();

        let token = match parse_access_token_file("token.json", &raw) {
            Ok(token) => token,
            Err(e) => panic!("parse_access_token_file failed: {}", e.message),
        };

        assert_eq!(token, "conformance-token");
    }

    #[test]
    fn parse_access_token_file_rejects_plain_text_token() {
        assert!(parse_access_token_file("token.json", "conformance-token").is_err());
    }

    #[test]
    fn parse_access_token_file_rejects_expired_token() {
        let raw = serde_json::json!({
            "access_token": "conformance-token",
            "expires_on": (Utc::now() - chrono::Duration::minutes(5)).to_rfc3339(),
        })
        .to_string();

        assert!(parse_access_token_file("token.json", &raw).is_err());
    }
}
