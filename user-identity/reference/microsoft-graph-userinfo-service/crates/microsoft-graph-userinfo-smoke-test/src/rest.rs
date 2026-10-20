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

//! REST transport smoke tests.
//!
//! Exercises the public REST API (`/healthz`, `/readyz`, `/api/v1alpha/users`,
//! `/api/v1alpha/groups`, `/api/v1alpha/users/{id}/groups`) through the public
//! ingress. When the deployment fronts the service with a Contour
//! `authExtension` (or any other auth gateway that authenticates every path),
//! the smoke test acquires a service bearer through `acquire_service_bearer`
//! — mirroring the conformance test's OAuth2 client-credentials + OpenID
//! Connect Discovery flow — and sends it on every call. When no service
//! bearer credentials are configured (`serviceIdentity.clientCredentials.enabled=false`,
//! or the equivalent CLI flags omitted), the smoke test runs without an
//! `Authorization` header and exercises the deployment as an anonymous client.
//! The optional Test 3 step uses `acquire_graph_bearer` to obtain a
//! Graph-audience token from Microsoft Entra ID and validates it against the
//! live Microsoft Graph API.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::time::sleep;

use crate::{Args, AuthMode, HINT, INFO, TestError, TestRunner, is_helm_mode};

/// Simple structured representations of the public REST API responses the
/// smoke test consumes. Fields other than `id` are intentionally ignored.
#[derive(Debug, Deserialize)]
struct IdOnly {
    id: String,
}

#[derive(Debug, Deserialize)]
struct UserInfoPage {
    items: Vec<IdOnly>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// Response shape for Microsoft Graph `/users` used to validate the acquired
/// token end-to-end against the real Graph API.
#[derive(Debug, Deserialize)]
struct GraphUsersResponse {
    #[serde(rename = "value")]
    value: Vec<serde_json::Value>,
}

/// OAuth2 token response from the Microsoft identity platform.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct AccessTokenFile {
    access_token: String,
    expires_on: DateTime<Utc>,
}

/// Thin helper around `reqwest::Client` that adds the service URL prefix and,
/// when configured, the bearer token to all calls to the user service.
struct SmokeClient {
    http: reqwest::Client,
    service_url: String,
    bearer: Option<String>,
}

impl SmokeClient {
    fn new(http: reqwest::Client, service_url: String, bearer: Option<String>) -> Self {
        Self {
            http,
            service_url,
            bearer,
        }
    }

    /// Builds a `GET` request and attaches the bearer token only when a service
    /// bearer was acquired in Test 1; deployments without an auth extension
    /// run unauthenticated.
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.http.get(url);
        match &self.bearer {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    async fn health(&self) -> Result<(), TestError> {
        let url = format!("{}/healthz", self.service_url);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &url))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TestError::unexpected_status("GET /healthz", status, &body));
        }
        Ok(())
    }

    async fn ready(&self) -> Result<(), TestError> {
        let url = format!("{}/readyz", self.service_url);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &url))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        if status.as_u16() == 503 {
            return Err(TestError::new(
                "GET /readyz returned 503 Service Unavailable",
                format!(
                    "GET /readyz on a reader returned 503, which is unexpected: readers report ready as soon as they start.\n\
                     {HINT}   * Check the reader started its HTTP server — inspect reader logs: `kubectl logs deploy/<release>-userinfo-service-reader`\n\
                     {HINT}   * The Service should select reader pods only; verify the `app.kubernetes.io/component: reader` selector resolves to running pods\n\
                     {HINT}   * Increase smoke-test `smokeTest.waitRetries` / `smokeTest.waitInterval` so the test waits long enough for the reader to start",
                ),
            ));
        }
        Err(TestError::unexpected_status("GET /readyz", status, &body))
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        step: &str,
        path: &str,
    ) -> Result<T, TestError> {
        let url = format!("{}{}", self.service_url, path);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &url))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TestError::unexpected_status(step, status, &body));
        }
        resp.json::<T>()
            .await
            .map_err(|e| TestError::invalid_json(step, &e))
    }
}

/// Polls `/healthz` until the service accepts requests or the retry budget is
/// exhausted. When a service bearer is provided it is sent on every probe so
/// deployments fronted by an auth extension that authenticates every path
/// (Contour `authExtension`, permission middleware) accept the call; handlers
/// downstream ignore the token on health endpoints. When the bearer is `None`,
/// the probe is sent unauthenticated, which matches deployments without an
/// auth extension (or with the HTTPProxy disabled entirely).
pub async fn wait_for_service(
    client: &reqwest::Client,
    service_url: &str,
    bearer: Option<&str>,
    retries: u32,
    interval_s: u64,
) -> Result<(), TestError> {
    let url = format!("{service_url}/healthz");
    println!(
        "   {INFO} waiting for service at {url} (up to {} s)",
        retries as u64 * interval_s
    );
    for attempt in 1..=retries {
        let req = match bearer {
            Some(token) => client.get(&url).bearer_auth(token),
            None => client.get(&url),
        };
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("   {INFO} service is reachable (attempt {attempt}/{retries})");
                return Ok(());
            }
            Ok(resp) => {
                println!(
                    "   {INFO} not ready yet (HTTP {}), retrying in {interval_s} s...",
                    resp.status().as_u16()
                );
            }
            Err(e) => {
                println!("   {INFO} not reachable ({e}), retrying in {interval_s} s...");
            }
        }
        if attempt < retries {
            sleep(Duration::from_secs(interval_s)).await;
        }
    }
    Err(TestError::connection_failed(
        service_url,
        "service did not become reachable within the retry budget",
    ))
}

/// Subset of the OpenID Connect Discovery metadata document the smoke test
/// consumes — only `token_endpoint` is required to drive the client-credentials
/// flow against the configured identity provider.
#[derive(Debug, Deserialize)]
struct OpenIdProviderMetadata {
    token_endpoint: String,
}

/// Test 1: acquires the bearer used for every call into the user service.
///
/// Runs before the pre-flight `wait_for_service` so the bearer is available
/// for `/healthz` and `/readyz` probes when an `authExtension` (Contour) sits
/// in front of the pod and authenticates every path. Mirrors the conformance
/// test's `serviceIdentity.clientCredentials` flow: the smoke test fetches the
/// OpenID Connect Discovery document, reads `token_endpoint`, and exchanges
/// `OAUTH2_CLIENT_ID` + `OAUTH2_CLIENT_SECRET` for a token whose audience is
/// the user service. A Microsoft Graph-scoped Entra token would not be
/// accepted by an `authExtension` configured for the service's own audience.
///
/// When no service-bearer credentials are configured the test is skipped (it
/// is not recorded as a pass or a failure) and `Ok(None)` is returned, which
/// signals downstream calls to run unauthenticated. This supports deployments
/// without an auth extension (or without an HTTPProxy at all).
pub async fn run_acquire_service_bearer_test(
    runner: &mut TestRunner,
    args: &Args,
    http: &reqwest::Client,
) -> Result<Option<String>, ()> {
    if !service_bearer_configured(args) {
        println!();
        println!(
            "Test 1 skipped: no service bearer credentials configured, the smoke test will run unauthenticated"
        );
        println!(
            "   {INFO} expected when the service is deployed without an auth extension (Contour `authExtension`) \
             or without an HTTPProxy at all. Configure `serviceIdentity.clientCredentials` (Helm) or \
             `--bearer-token` / `--access-token-file` / OAuth2 flags (CLI) to enable Test 1."
        );
        return Ok(None);
    }
    println!();
    println!("Test 1: Acquire user service bearer token");
    println!(
        "   {INFO} runs the OAuth2 client-credentials flow against the configured identity provider \
         (OpenID Connect Discovery URI, client id, client secret, client scope) and uses the resulting \
         access token for every subsequent call into the user service. Mirrors the deployment-time \
         conventions of `serviceIdentity.clientCredentials` in the conformance test chart."
    );
    let start = Instant::now();
    match acquire_service_bearer(http, args).await {
        Ok(t) => {
            runner.record_pass("Acquire user service bearer", start.elapsed().as_millis());
            Ok(Some(t))
        }
        Err(e) => {
            runner.record_fail(
                "Acquire user service bearer",
                start.elapsed().as_millis(),
                &e,
            );
            Err(())
        }
    }
}

/// Returns true when at least one service-bearer credential source is
/// configured, in priority order: literal token, access-token file, or the
/// OAuth2 client-credentials flow (OpenID Connect Discovery URI + client ID).
pub fn service_bearer_configured(args: &Args) -> bool {
    args.bearer_token.as_deref().is_some_and(|t| !t.is_empty())
        || args
            .access_token_file
            .as_deref()
            .is_some_and(|p| !p.is_empty())
        || (args
            .oauth2_openid_configuration_uri
            .as_deref()
            .is_some_and(|s| !s.is_empty())
            && args
                .oauth2_client_id
                .as_deref()
                .is_some_and(|s| !s.is_empty()))
}

/// Resolves the bearer the smoke test uses to authenticate against the user
/// service. Caller must guarantee that at least one credential source is
/// configured (see `service_bearer_configured`). Honors, in order:
///
/// 1. `--bearer-token` / `BEARER_TOKEN` literal (local development).
/// 2. `--access-token-file` / `ACCESS_TOKEN_FILE` populated by a sidecar.
/// 3. OAuth2 client-credentials with `OAUTH2_OPENID_CONFIGURATION_URI` and
///    `OAUTH2_CLIENT_ID` (+ secret material and an optional scope), matching
///    the conformance test's `serviceIdentity.clientCredentials` chart block.
async fn acquire_service_bearer(http: &reqwest::Client, args: &Args) -> Result<String, TestError> {
    if let Some(token) = args.bearer_token.as_ref().filter(|t| !t.is_empty()) {
        println!("   {INFO} bearer token loaded from --bearer-token");
        return Ok(token.clone());
    }
    if let Some(path) = args.access_token_file.as_ref().filter(|p| !p.is_empty()) {
        let raw = tokio::fs::read_to_string(path).await.map_err(|e| {
            TestError::new(
                format!("Failed to read access token file '{path}': {e}"),
                format!(
                    "{HINT}   * The file must exist and be readable by the smoke test pod\n\
                         {HINT}   * In Helm, populate it via a token-provisioner sidecar; see\n\
                         {HINT}     `serviceIdentity.accessTokenFile.provisionerSidecar`",
                ),
            )
        })?;
        let token = parse_access_token_file(path, &raw)?;
        if token.is_empty() {
            return Err(TestError::new(
                format!("Access token file '{path}' is empty"),
                format!(
                    "{HINT}   * The token-provisioner sidecar has not written the token yet, or\n\
                     {HINT}     wrote an empty file. Inspect the sidecar logs."
                ),
            ));
        }
        println!("   {INFO} bearer token loaded from {path}");
        return Ok(token);
    }
    oauth2_client_credentials(http, args).await
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

/// Performs the OAuth2 client-credentials flow against the identity provider's
/// `token_endpoint`, advertised through the supplied OpenID Connect Discovery
/// URI. Returns the issued access token.
async fn oauth2_client_credentials(
    http: &reqwest::Client,
    args: &Args,
) -> Result<String, TestError> {
    let oidc_uri = args.oauth2_openid_configuration_uri.as_ref().unwrap();
    let client_id = args.oauth2_client_id.as_ref().unwrap();

    let client_secret = resolve_oauth2_client_secret(args).await?;

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

    println!("   {INFO} requesting OAuth2 token from {token_url}");
    let mut req = http.post(&token_url).form(&form);
    for (k, v) in &extra_headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_connect() {
            TestError::connection_failed(&token_url, &e.to_string())
        } else {
            TestError::new(
                format!("OAuth2 token request to {token_url} failed: {e}"),
                format!(
                    "{HINT}   * Confirm cluster egress to the identity provider's token endpoint\n\
                     {HINT}   * Verify TLS interception or proxies are not stripping the request",
                ),
            )
        }
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let body_preview: String = body.chars().take(500).collect();
        return Err(TestError::new(
            format!("OAuth2 token endpoint returned HTTP {}", status.as_u16()),
            format!(
                "{HINT}   * Verify --oauth2-client-id / --oauth2-client-secret / --oauth2-client-scope\n\
                 {HINT}   * Helm equivalents: serviceIdentity.clientCredentials.{{clientId, clientScope, clientSecretRef}}\n\
                 {HINT}   * Response body: {body_preview}",
            ),
        ));
    }

    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| TestError::invalid_json("OAuth2 token endpoint", &e))?;

    if token.access_token.is_empty() {
        return Err(TestError::new(
            "OAuth2 token endpoint returned an empty access_token",
            format!(
                "{HINT}   * Inspect the identity provider configuration; the response had no access_token"
            ),
        ));
    }
    if token.expires_in > 0 {
        println!(
            "   {INFO} access_token acquired (expires in {} s)",
            token.expires_in
        );
    }
    Ok(token.access_token)
}

/// Resolves the OAuth2 client secret from the literal CLI value or, failing
/// that, from a mounted file (the Helm chart's typical delivery path).
async fn resolve_oauth2_client_secret(args: &Args) -> Result<String, TestError> {
    if let Some(secret) = args.oauth2_client_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok(secret.clone());
    }
    if let Some(path) = args
        .oauth2_client_secret_file
        .as_ref()
        .filter(|s| !s.is_empty())
    {
        return tokio::fs::read_to_string(path)
            .await
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                TestError::new(
                    format!("Failed to read OAuth2 client secret file '{path}': {e}"),
                    format!(
                        "{HINT}   * The file must exist and be readable by the smoke test pod\n\
                         {HINT}   * In Helm, the secret is mounted from `serviceIdentity.clientCredentials.clientSecretRef`",
                    ),
                )
            });
    }
    Err(TestError::new(
        "OAuth2 client-credentials flow needs a client secret",
        format!(
            "{HINT}   * Provide --oauth2-client-secret or --oauth2-client-secret-file\n\
             {HINT}   * Helm: serviceIdentity.clientCredentials.clientSecretRef.{{name,key}}",
        ),
    ))
}

/// Fetches the OpenID Connect Discovery metadata document and returns the
/// advertised `token_endpoint`.
async fn discover_token_endpoint(http: &reqwest::Client, uri: &str) -> Result<String, TestError> {
    println!("   {INFO} fetching OpenID configuration from {uri}");
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

/// Parses a JSON object of string values, used by the OAuth2 extra headers /
/// extra body parameter knobs. Empty input is treated as an empty map.
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

/// Acquires a Microsoft Entra ID access token whose audience is the Microsoft
/// Graph API. Used solely for the optional Test 3 step that proves the smoke
/// test pod's Entra credentials are accepted by Graph and that the configured
/// app registration has the required application permissions. The audience of
/// this token does not match the user service's `authExtension` audience and
/// is therefore not interchangeable with the service bearer.
async fn acquire_graph_bearer(http: &reqwest::Client, args: &Args) -> Result<String, TestError> {
    let tenant_id = args
        .azure_tenant_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            TestError::new(
                "AZURE_TENANT_ID is required to acquire a Microsoft Graph token",
                format!(
                    "{HINT}   * Set --azure-tenant-id / AZURE_TENANT_ID or disable Test 3 by\n\
                     {HINT}     leaving the AZURE_* variables empty in Helm values"
                ),
            )
        })?;
    let client_id = args
        .azure_client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            TestError::new(
                "AZURE_CLIENT_ID is required to acquire a Microsoft Graph token",
                format!("{HINT}   * Set --azure-client-id / AZURE_CLIENT_ID"),
            )
        })?;

    let token_url = format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token");

    let form: Vec<(String, String)> = match args.azure_auth_mode {
        AuthMode::ClientSecret => {
            let secret = resolve_azure_client_secret(args).await?;
            vec![
                ("grant_type".into(), "client_credentials".into()),
                ("client_id".into(), client_id.to_string()),
                ("client_secret".into(), secret),
                ("scope".into(), args.graph_scope.clone()),
            ]
        }
        AuthMode::WorkloadIdentity => {
            let assertion = tokio::fs::read_to_string(&args.azure_federated_token_file)
                .await
                .map_err(|e| {
                    let config_hint = if is_helm_mode() {
                        format!(
                            "{HINT}   * Helm value `providers.entra.authMode` is 'workload-identity' but the projected token is missing\n\
                             {HINT}   * Confirm the service account is annotated for Azure Workload Identity and the mutating webhook ran\n\
                             {HINT}   * `kubectl describe pod <smoke-test-pod>` should show the `azure-identity-token` projected volume at {path}",
                            path = args.azure_federated_token_file,
                        )
                    } else {
                        format!(
                            "{HINT}   * `AZURE_FEDERATED_TOKEN_FILE` ({path}) must exist and be readable\n\
                             {HINT}   * When running outside Kubernetes, workload identity is usually not available",
                            path = args.azure_federated_token_file,
                        )
                    };
                    TestError::new(
                        format!(
                            "Failed to read federated token file {}: {e}",
                            args.azure_federated_token_file
                        ),
                        config_hint,
                    )
                })?;
            vec![
                ("grant_type".into(), "client_credentials".into()),
                ("client_id".into(), client_id.to_string()),
                (
                    "client_assertion_type".into(),
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".into(),
                ),
                ("client_assertion".into(), assertion.trim().to_string()),
                ("scope".into(), args.graph_scope.clone()),
            ]
        }
    };

    let resp = http
        .post(&token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                TestError::connection_failed(&token_url, &e.to_string())
            } else {
                TestError::new(
                    format!("Request to {token_url} failed: {e}"),
                    format!(
                        "{HINT}   * Cluster egress may block `login.microsoftonline.com`\n\
                         {HINT}   * Check egress proxies / network policies / firewall rules",
                    ),
                )
            }
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let body_preview: String = body.chars().take(500).collect();
        let hint = match status.as_u16() {
            400 | 401 => format!(
                "Entra ID rejected the credentials.\n\
                 {HINT}   * Helm value `providers.entra.tenantId` matches the app registration's tenant\n\
                 {HINT}   * Helm value `providers.entra.clientId` matches the app registration's Application (client) ID\n\
                 {HINT}   * For `client-secret` auth mode: the Kubernetes Secret referenced by `providers.entra.clientSecret.secretRef` contains a valid, non-expired client secret\n\
                 {HINT}   * For `workload-identity` auth mode: the federated credential in Entra ID matches this namespace / service account\n\
                 {HINT}   * Response body: {body_preview}",
            ),
            403 => format!(
                "Entra ID returned 403 Forbidden when requesting a token.\n\
                 {HINT}   * The app registration may not have admin-consented Microsoft Graph application permissions (`User.Read.All`, `Group.Read.All`)\n\
                 {HINT}   * Response body: {body_preview}",
            ),
            _ => format!(
                "{HINT}   * Unexpected response from Entra ID. Response body: {body_preview}",
            ),
        };
        return Err(TestError::new(
            format!("Entra ID token endpoint returned HTTP {}", status.as_u16()),
            hint,
        ));
    }

    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| TestError::invalid_json("Entra ID token endpoint", &e))?;
    if token.access_token.is_empty() {
        return Err(TestError::new(
            "Entra ID returned an empty access_token",
            format!(
                "{HINT}   * Token endpoint responded with a body missing `access_token`; check Azure AD app configuration"
            ),
        ));
    }
    if token.expires_in > 0 {
        println!(
            "   {INFO} Microsoft Graph access token acquired (expires in {} s)",
            token.expires_in
        );
    }
    Ok(token.access_token)
}

/// Reads the Entra ID client secret from the CLI value or from the file the
/// chart mounts at `providers.entra.clientSecret.secretRef`.
async fn resolve_azure_client_secret(args: &Args) -> Result<String, TestError> {
    if let Some(secret) = args.azure_client_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok(secret.clone());
    }
    if let Some(path) = args
        .azure_client_secret_file
        .as_ref()
        .filter(|s| !s.is_empty())
    {
        return tokio::fs::read_to_string(path)
            .await
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                TestError::new(
                    format!("Failed to read client secret file {path}: {e}"),
                    format!(
                        "{HINT}   * Verify the Secret referenced by `providers.entra.clientSecret.secretRef` exists in the Release namespace\n\
                         {HINT}       kubectl get secret <name> -n <namespace>\n\
                         {HINT}   * Verify the Secret contains the key `<key>` (Helm value `providers.entra.clientSecret.secretRef.key`)",
                    ),
                )
            });
    }
    Err(TestError::new(
        "No Entra ID client secret provided",
        format!(
            "{HINT}   * Set `providers.entra.clientSecret.secretRef` (when authMode=client-secret) in Helm values\n\
             {HINT}   * Or switch `providers.entra.authMode` to 'workload-identity'\n\
             {HINT}   * When running standalone, set `AZURE_CLIENT_SECRET` or `AZURE_CLIENT_SECRET_FILE`",
        ),
    ))
}

/// Returns true when the smoke test is configured to acquire a Microsoft Graph
/// token for Test 3. When false, the Microsoft Graph step is skipped because
/// the operator opted out by leaving the AZURE_* variables empty.
fn graph_test_enabled(args: &Args) -> bool {
    args.azure_tenant_id
        .as_deref()
        .is_some_and(|s| !s.is_empty())
        && args
            .azure_client_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
}

/// Issues a minimal `/users?$top=1` request to Microsoft Graph to confirm the
/// acquired token is accepted and the app has the required scopes.
async fn call_graph_users(
    http: &reqwest::Client,
    base_url: &str,
    bearer: &str,
) -> Result<usize, TestError> {
    let url = format!("{base_url}/users?$top=1&$select=id");
    let resp = http
        .get(&url)
        .bearer_auth(bearer)
        .header("ConsistencyLevel", "eventual")
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                TestError::connection_failed(base_url, &e.to_string())
            } else {
                TestError::new(
                    format!("Microsoft Graph request failed: {e}"),
                    format!(
                        "{HINT}   * Cluster egress to graph.microsoft.com is blocked\n\
                         {HINT}   * TLS interception / proxy is stripping or rejecting the request\n\
                         {HINT}   * The user service would fail similarly; verify from a debug pod",
                    ),
                )
            }
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let body_preview: String = body.chars().take(500).collect();
        let hint = match status.as_u16() {
            401 => format!(
                "HTTP 401: Microsoft Graph rejected the token.\n\
                 {HINT}   * Token was issued for the wrong audience. Check that `GRAPH_SCOPE` is set to `https://graph.microsoft.com/.default`\n\
                 {HINT}   * The app registration does not exist in the specified tenant\n\
                 {HINT}   * Response body: {body_preview}",
            ),
            403 => format!(
                "HTTP 403: Microsoft Graph accepted the token but the app lacks permissions.\n\
                 {HINT}   * Grant the app registration admin-consented Microsoft Graph application permissions: `User.Read.All`, `Group.Read.All` (and `Directory.Read.All` for delta queries)\n\
                 {HINT}   * Response body: {body_preview}",
            ),
            _ => format!(
                "Microsoft Graph returned HTTP {}. Response body: {body_preview}",
                status.as_u16()
            ),
        };
        return Err(TestError::new(
            format!("Microsoft Graph /users returned HTTP {}", status.as_u16()),
            hint,
        ));
    }

    let parsed: GraphUsersResponse = resp
        .json()
        .await
        .map_err(|e| TestError::invalid_json("Microsoft Graph /users", &e))?;
    Ok(parsed.value.len())
}

/// Runs the REST smoke-test sequence after Test 1 has already acquired (or
/// skipped) the service bearer. Stops and returns `Err(())` on the first
/// failure. When `bearer` is `None`, requests are sent without an
/// `Authorization` header — matching deployments without an auth extension.
pub async fn run_rest_tests(
    runner: &mut TestRunner,
    args: &Args,
    http: reqwest::Client,
    rest_url: &str,
    bearer: Option<&str>,
) -> Result<(), ()> {
    let service_url = rest_url.trim_end_matches('/').to_string();
    let client = SmokeClient::new(
        http.clone(),
        service_url.clone(),
        bearer.map(|b| b.to_string()),
    );

    // 2. Health check ---------------------------------------------------------
    println!();
    println!("Test 2: Service health");
    println!(
        "   {INFO} verifies that /healthz returns 200 and /readyz reports the service is ready to serve requests"
    );

    let start = Instant::now();
    if let Err(e) = client.health().await {
        runner.record_fail("Health check (/healthz)", start.elapsed().as_millis(), &e);
        return Err(());
    }
    runner.record_pass("Health check (/healthz)", start.elapsed().as_millis());

    let start = Instant::now();
    if let Err(e) = client.ready().await {
        runner.record_fail("Readiness check (/readyz)", start.elapsed().as_millis(), &e);
        return Err(());
    }
    runner.record_pass("Readiness check (/readyz)", start.elapsed().as_millis());

    // 3. Microsoft Graph API --------------------------------------------------
    if graph_test_enabled(args) {
        println!();
        println!("Test 3: Call Microsoft Graph API with the configured Entra credentials");
        let mode = match args.azure_auth_mode {
            AuthMode::ClientSecret => "client-secret",
            AuthMode::WorkloadIdentity => "workload-identity",
        };
        println!(
            "   {INFO} acquires a Microsoft Graph-scoped access token via `{mode}` auth mode and issues \
             GET {base}/users?$top=1 to confirm the smoke test pod's Entra credentials are accepted by Graph \
             and the app registration has the required application permissions",
            base = args.graph_base_url,
        );
        let start = Instant::now();
        let graph_bearer = match acquire_graph_bearer(&http, args).await {
            Ok(t) => t,
            Err(e) => {
                runner.record_fail("Microsoft Graph /users", start.elapsed().as_millis(), &e);
                return Err(());
            }
        };
        match call_graph_users(&http, &args.graph_base_url, &graph_bearer).await {
            Ok(count) => {
                println!("   {INFO} Microsoft Graph returned {count} user(s)");
                runner.record_pass("Microsoft Graph /users", start.elapsed().as_millis());
            }
            Err(e) => {
                runner.record_fail("Microsoft Graph /users", start.elapsed().as_millis(), &e);
                return Err(());
            }
        }
    } else {
        println!();
        println!(
            "   {INFO} Test 3 skipped: AZURE_TENANT_ID / AZURE_CLIENT_ID not configured, Microsoft Graph step disabled"
        );
    }

    // 4a. List users ----------------------------------------------------------
    println!();
    println!("Test 4a: GET /api/v1alpha/users (list one page)");
    println!(
        "   {INFO} verifies the users list endpoint returns a page with items and pagination fields"
    );
    let start = Instant::now();
    let users_page: UserInfoPage = match client
        .get_json(
            "GET /api/v1alpha/users",
            &format!("/api/v1alpha/users?limit={}", args.page_limit),
        )
        .await
    {
        Ok(page) => page,
        Err(e) => {
            runner.record_fail(
                "List users (/api/v1alpha/users)",
                start.elapsed().as_millis(),
                &e,
            );
            return Err(());
        }
    };
    if users_page.items.is_empty() {
        runner.record_fail(
            "List users (/api/v1alpha/users)",
            start.elapsed().as_millis(),
            &TestError::new(
                "GET /api/v1alpha/users returned 0 items",
                format!(
                    "{HINT}   * The directory appears to be empty from the service's point of view\n\
                     {HINT}   * Verify the Entra app registration has admin-consented `User.Read.All` permission\n\
                     {HINT}   * If caching is enabled (`providers.cache.enabled=true`, `providers.cache.users.enabled=true`), wait for the initial sync to complete; check the sync logs\n\
                     {HINT}   * If the Permission Service filters all users out, review policies or temporarily disable `permission.enabled`",
                ),
            ),
        );
        return Err(());
    }
    println!(
        "   {INFO} received {} user(s), next_cursor={}",
        users_page.items.len(),
        users_page
            .next_cursor
            .as_deref()
            .map(|_| "<present>")
            .unwrap_or("<none>"),
    );
    runner.record_pass(
        "List users (/api/v1alpha/users)",
        start.elapsed().as_millis(),
    );
    let sample_user_id = users_page.items[0].id.clone();

    // 4b. Get one user --------------------------------------------------------
    println!();
    println!("Test 4b: GET /api/v1alpha/users/{{userId}} (single user detail)");
    println!("   {INFO} fetches user '{sample_user_id}' — the first entry from the users list");
    let start = Instant::now();
    let path = format!("/api/v1alpha/users/{sample_user_id}");
    if let Err(e) = client
        .get_json::<serde_json::Value>("GET /api/v1alpha/users/{id}", &path)
        .await
    {
        runner.record_fail(
            "Get user (/api/v1alpha/users/{id})",
            start.elapsed().as_millis(),
            &e,
        );
        return Err(());
    }
    runner.record_pass(
        "Get user (/api/v1alpha/users/{id})",
        start.elapsed().as_millis(),
    );

    // 4c. List groups ---------------------------------------------------------
    println!();
    println!("Test 4c: GET /api/v1alpha/groups (list one page)");
    println!(
        "   {INFO} verifies the groups list endpoint returns a page with items and pagination fields"
    );
    let start = Instant::now();
    let groups_page: UserInfoPage = match client
        .get_json(
            "GET /api/v1alpha/groups",
            &format!("/api/v1alpha/groups?limit={}", args.page_limit),
        )
        .await
    {
        Ok(page) => page,
        Err(e) => {
            runner.record_fail(
                "List groups (/api/v1alpha/groups)",
                start.elapsed().as_millis(),
                &e,
            );
            return Err(());
        }
    };
    if groups_page.items.is_empty() {
        runner.record_fail(
            "List groups (/api/v1alpha/groups)",
            start.elapsed().as_millis(),
            &TestError::new(
                "GET /api/v1alpha/groups returned 0 items",
                format!(
                    "{HINT}   * The directory appears to have no groups\n\
                     {HINT}   * Verify the Entra app registration has admin-consented `Group.Read.All` permission\n\
                     {HINT}   * If caching is enabled (`providers.cache.groups.enabled=true`), wait for the initial sync\n\
                     {HINT}   * If the Permission Service filters all groups out, review policies or temporarily disable `permission.enabled`",
                ),
            ),
        );
        return Err(());
    }
    println!(
        "   {INFO} received {} group(s), next_cursor={}",
        groups_page.items.len(),
        groups_page
            .next_cursor
            .as_deref()
            .map(|_| "<present>")
            .unwrap_or("<none>"),
    );
    runner.record_pass(
        "List groups (/api/v1alpha/groups)",
        start.elapsed().as_millis(),
    );
    let sample_group_id = groups_page.items[0].id.clone();

    // 4d. Get one group -------------------------------------------------------
    println!();
    println!("Test 4d: GET /api/v1alpha/groups/{{groupId}} (single group detail)");
    println!("   {INFO} fetches group '{sample_group_id}' — the first entry from the groups list");
    let start = Instant::now();
    let path = format!("/api/v1alpha/groups/{sample_group_id}");
    if let Err(e) = client
        .get_json::<serde_json::Value>("GET /api/v1alpha/groups/{id}", &path)
        .await
    {
        runner.record_fail(
            "Get group (/api/v1alpha/groups/{id})",
            start.elapsed().as_millis(),
            &e,
        );
        return Err(());
    }
    runner.record_pass(
        "Get group (/api/v1alpha/groups/{id})",
        start.elapsed().as_millis(),
    );

    // 4e. Group membership for one user ---------------------------------------
    println!();
    println!("Test 4e: GET /api/v1alpha/users/{{userId}}/groups (user's groups)");
    println!(
        "   {INFO} fetches group membership of user '{sample_user_id}' to validate the user-to-group resolution path"
    );
    let start = Instant::now();
    let path = format!(
        "/api/v1alpha/users/{sample_user_id}/groups?limit={}",
        args.page_limit
    );
    if let Err(e) = client
        .get_json::<UserInfoPage>("GET /api/v1alpha/users/{id}/groups", &path)
        .await
    {
        runner.record_fail(
            "User group membership (/api/v1alpha/users/{id}/groups)",
            start.elapsed().as_millis(),
            &e,
        );
        return Err(());
    }
    runner.record_pass(
        "User group membership (/api/v1alpha/users/{id}/groups)",
        start.elapsed().as_millis(),
    );

    Ok(())
}
