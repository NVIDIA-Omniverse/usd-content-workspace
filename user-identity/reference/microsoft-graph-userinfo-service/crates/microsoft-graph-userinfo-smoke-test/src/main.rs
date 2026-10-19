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

//! Smoke test binary for the Microsoft Graph User Info Service.
//!
//! Verifies a live deployment end-to-end through the public ingress (or the
//! in-cluster Service when no ingress is in use). Test 1 acquires the
//! user-service bearer via the OAuth2 client-credentials flow driven by
//! OpenID Connect Discovery (matching the conformance test's
//! `serviceIdentity.clientCredentials` chart block), so the issued token has
//! the audience the Contour `authExtension` expects. The same bearer is sent
//! on every subsequent call, including the `/healthz` and `/readyz` probes,
//! because the auth extension authenticates every path. Test 3 additionally
//! acquires a Microsoft Graph-scoped Entra ID token (independent audience) to
//! confirm the smoke test pod's Entra credentials can call Graph.
//!
//! When the deployment does not enable an auth extension — or the HTTPProxy
//! is disabled entirely — service-bearer credentials may be omitted: Test 1
//! is skipped and the smoke test runs unauthenticated against the service.
//!
//! Tests are executed sequentially and stop after the first failure. Each
//! failure is reported with a detailed hint describing possible causes and
//! concrete Helm values / Kubernetes objects to inspect.
//!
//! Module layout:
//! - `main.rs` (this file): CLI arguments, shared primitives (`TestError`,
//!   `TestRunner`, log constants, Helm-mode flag), and the top-level launcher.
//! - `rest`: REST transport smoke tests, the OAuth2 client-credentials flow
//!   used for the user-service bearer, and the Microsoft Graph bootstrap.
//! - `grpc`: gRPC transport smoke tests that mirror the REST coverage.

mod grpc;
mod rest;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;

pub const PASS: &str = "PASS";
pub const FAIL: &str = "FAIL";
pub const INFO: &str = "INFO";
pub const HINT: &str = "HINT";

static HELM_MODE: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_helm_mode() -> bool {
    HELM_MODE.load(Ordering::Relaxed)
}

#[derive(clap::ValueEnum, Debug, Clone, Default)]
pub enum AuthMode {
    #[default]
    ClientSecret,
    WorkloadIdentity,
}

/// Command-line / environment configuration for the smoke test.
///
/// Mirrors the subset of the user service configuration that is required to
/// reach the service and acquire an Entra ID token in the same way the service
/// itself does.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Smoke test for microsoft-graph-userinfo-service",
    long_about = None,
)]
pub struct Args {
    #[arg(long, env = "REST_URL")]
    pub rest_url: Option<String>,

    /// Back-compat: previous name for `--rest-url`.
    #[arg(long, env = "SERVICE_URL", hide = true)]
    pub service_url: Option<String>,

    #[arg(long, env = "GRPC_ENABLED", default_value = "true")]
    pub grpc_enabled: bool,

    /// gRPC endpoint in the form `host:port` or `scheme://host:port`. When not set,
    /// defaults to the REST URL's host combined with `--grpc-port`.
    #[arg(long, env = "GRPC_ENDPOINT")]
    pub grpc_endpoint: Option<String>,

    #[arg(long, env = "GRPC_PORT", default_value = "50051")]
    pub grpc_port: u16,

    #[arg(long, env = "GRPC_TLS_ENABLED", default_value = "false")]
    pub grpc_tls_enabled: bool,

    /// Microsoft Entra ID tenant. Required only for the optional Microsoft
    /// Graph step (Test 3). Leave empty to skip Test 3.
    #[arg(long, env = "AZURE_TENANT_ID")]
    pub azure_tenant_id: Option<String>,

    /// Microsoft Entra ID app registration client ID used by Test 3.
    /// Required only for the Microsoft Graph step.
    #[arg(long, env = "AZURE_CLIENT_ID")]
    pub azure_client_id: Option<String>,

    #[arg(
        long,
        env = "AZURE_AUTH_MODE",
        default_value = "client-secret",
        value_enum
    )]
    pub azure_auth_mode: AuthMode,

    #[arg(long, env = "AZURE_CLIENT_SECRET")]
    pub azure_client_secret: Option<String>,

    #[arg(long, env = "AZURE_CLIENT_SECRET_FILE")]
    pub azure_client_secret_file: Option<String>,

    #[arg(
        long,
        env = "AZURE_FEDERATED_TOKEN_FILE",
        default_value = "/var/run/secrets/azure/tokens/azure-identity-token"
    )]
    pub azure_federated_token_file: String,

    #[arg(
        long,
        env = "GRAPH_BASE_URL",
        default_value = "https://graph.microsoft.com/v1.0"
    )]
    pub graph_base_url: String,

    #[arg(
        long,
        env = "GRAPH_SCOPE",
        default_value = "https://graph.microsoft.com/.default"
    )]
    pub graph_scope: String,

    /// Direct service bearer token. Intended for local development; in-cluster
    /// deployments should use the OAuth2 client-credentials flags or an
    /// access-token file populated by a sidecar instead.
    #[arg(long, env = "BEARER_TOKEN")]
    pub bearer_token: Option<String>,

    /// Path to a JSON document with `access_token` and `expires_on` fields.
    #[arg(long, env = "ACCESS_TOKEN_FILE")]
    pub access_token_file: Option<String>,

    /// OAuth2 client-credentials flow: OpenID Connect Discovery URI.
    ///
    /// The smoke test issues `GET <uri>` to retrieve the identity provider's
    /// metadata document and uses the `token_endpoint` field for the token
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
    /// (e.g. `<service-app-id>/.default`). The token's `aud` claim is what the
    /// Contour `authExtension` validates, so this scope must match the
    /// audience the auth extension expects for the user service.
    #[arg(long, env = "OAUTH2_CLIENT_SCOPE")]
    pub oauth2_client_scope: Option<String>,

    /// OAuth2 client-credentials flow: additional HTTP headers to include in
    /// the token-exchange request, as a JSON object
    /// (e.g. `{"X-Custom-Header": "value"}`).
    #[arg(long, env = "OAUTH2_EXTRA_HEADERS", default_value = "{}")]
    pub oauth2_extra_headers: String,

    /// OAuth2 client-credentials flow: additional body parameters to merge
    /// into the token-exchange request, as a JSON object
    /// (e.g. `{"audience": "https://api.example.com"}`).
    #[arg(long, env = "OAUTH2_EXTRA_BODY_PARAMS", default_value = "{}")]
    pub oauth2_extra_body_params: String,

    #[arg(long, env = "WAIT_RETRIES", default_value = "10")]
    pub wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value = "5")]
    pub wait_interval: u64,

    #[arg(long, env = "PAGE_LIMIT", default_value = "10")]
    pub page_limit: u32,

    #[arg(long, env = "HTTP_TIMEOUT", default_value = "30")]
    pub http_timeout: u64,

    #[arg(long, env = "HELM", default_value = "false")]
    pub helm: bool,
}

/// A structured test failure with a user-facing message and a remediation hint.
pub struct TestError {
    pub message: String,
    pub hint: String,
}

impl TestError {
    pub fn new(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: hint.into(),
        }
    }

    pub(crate) fn connection_failed(url: &str, details: &str) -> Self {
        let details = if details.is_empty() {
            "connection refused or timed out".to_string()
        } else {
            details.to_string()
        };
        let config_hint = if is_helm_mode() {
            format!(
                "{HINT}   * Pod is running and Ready:\n\
                 {HINT}       kubectl get pods -l app.kubernetes.io/name=microsoft-graph-userinfo-service\n\
                 {HINT}   * Inspect pod logs for crashes or startup errors:\n\
                 {HINT}       kubectl logs -l app.kubernetes.io/name=microsoft-graph-userinfo-service\n\
                 {HINT}   * Service object points to the correct pod port:\n\
                 {HINT}       kubectl get svc,endpoints -l app.kubernetes.io/name=microsoft-graph-userinfo-service\n\
                 {HINT}   * Helm values: `service.ports.rest` (default 8080) matches the container port\n\
                 {HINT}   * Helm values: `service.nameOverride` / `fullnameOverride` did not rename the service to something the smoke test cannot resolve\n\
                 {HINT}   * If ingress is used: `httpProxy.enabled`, `httpProxy.fqdn.host`, `httpProxy.fqdn.domain`, `httpProxy.tls.enabled`",
            )
        } else {
            format!(
                "{HINT}   * `SERVICE_URL` is reachable from where the smoke test runs\n\
                 {HINT}   * The service process is up and listening on the expected port\n\
                 {HINT}   * Network / firewall allows the outbound connection",
            )
        };
        Self::new(
            format!("Cannot connect to service at {url}"),
            format!(
                "Check that:\n\
                 {config_hint}\n\
                 {HINT}   Raw error: {details}",
            ),
        )
    }

    pub(crate) fn from_reqwest_error(err: &reqwest::Error, url: &str) -> Self {
        if err.is_connect() {
            Self::connection_failed(url, &err.to_string())
        } else if err.is_timeout() {
            Self::new(
                format!("Request to {url} timed out"),
                format!(
                    "Check that:\n\
                     {HINT}   * The service is responsive and not overloaded\n\
                     {HINT}   * Initial directory sync has completed; until readiness is reported, list endpoints can block or time out\n\
                     {HINT}   * Network latency between the test runner and the service is acceptable",
                ),
            )
        } else {
            Self::new(
                format!("HTTP request failed: {err}"),
                format!(
                    "{HINT}   * The service at {url} may be misbehaving\n\
                     {HINT}   * TLS / certificate issues are also possible when connecting through ingress",
                ),
            )
        }
    }

    pub(crate) fn unexpected_status(step: &str, status: reqwest::StatusCode, body: &str) -> Self {
        let body_preview: String = body.chars().take(500).collect();
        let code = status.as_u16();
        let hint = match code {
            401 => format!(
                "HTTP 401 Unauthorized. The service refused the bearer token.\n\
                 {HINT}   * Confirm permission middleware expectations:\n\
                 {HINT}       - Helm value `permission.enabled` (default false): when true, the service requires an Authorization header accepted by the Permission Service\n\
                 {HINT}       - Helm value `permission.url` points to a reachable Permission Service\n\
                 {HINT}   * If Permission Service is on, the token issued to the smoke test must be accepted by it (issuer, audience, signature)\n\
                 {HINT}   * Verify the service received the token: `kubectl logs -l app.kubernetes.io/name=microsoft-graph-userinfo-service` and search for 'missing or invalid Authorization header'",
            ),
            403 => format!(
                "HTTP 403 Forbidden. Authorization was denied by the Permission Service.\n\
                 {HINT}   * Review Permission Service policies for the calling principal and the `userinfo` service\n\
                 {HINT}   * Helm values on the user service side: `permission.serviceName` (default 'userinfo')\n\
                 {HINT}   * If this is unexpected for a smoke test, temporarily set `permission.enabled=false` to isolate the problem",
            ),
            404 => format!(
                "HTTP 404 Not Found. The target resource does not exist or is hidden by permission filtering.\n\
                 {HINT}   * For list endpoints the directory may be empty; verify Entra ID has users / groups and the app registration has `User.Read.All` and `Group.Read.All` (or `Directory.Read.All`)\n\
                 {HINT}   * The ID used for the detailed lookup may have been filtered out by the Permission Service post-filter; inspect permission rules\n\
                 {HINT}   * For the group-membership test, the user may genuinely have no groups; pick another user for the test",
            ),
            429 => format!(
                "HTTP 429 Too Many Requests. The service or an upstream dependency is rate-limiting.\n\
                 {HINT}   * Microsoft Graph rate limiting: increase `providers.entra.maxRetries` (values.yaml)\n\
                 {HINT}   * Reduce the frequency of smoke tests or retry after a delay",
            ),
            503 => format!(
                "HTTP 503 Service Unavailable. The service is not ready yet.\n\
                 {HINT}   * Initial directory sync may still be in progress. For large directories, increase `readinessProbe.failureThreshold` and `readinessProbe.periodSeconds` (values.yaml)\n\
                 {HINT}   * Also confirm PostgreSQL is reachable so the cache can load quickly: `providers.cache.postgres.*`\n\
                 {HINT}   * `kubectl describe pod ...` will show readiness probe failures",
            ),
            500..=599 => format!(
                "HTTP {code} server error from the user service.\n\
                 {HINT}   * Check service logs: `kubectl logs -l app.kubernetes.io/name=microsoft-graph-userinfo-service`\n\
                 {HINT}   * Verify upstream dependencies: PostgreSQL (`providers.cache.postgres.*`), Microsoft Graph reachability, Entra ID token acquisition\n\
                 {HINT}   * Response body (truncated): {body_preview}",
            ),
            _ => format!(
                "{HINT}   * Unexpected status code. Response body (truncated): {body_preview}",
            ),
        };
        Self::new(format!("{step} returned HTTP {code}"), hint)
    }

    pub(crate) fn invalid_json(step: &str, err: &reqwest::Error) -> Self {
        Self::new(
            format!("{step}: failed to parse JSON response ({err})"),
            format!(
                "{HINT}   * The server returned a non-JSON or malformed body.\n\
                 {HINT}   * Inspect service logs: `kubectl logs -l app.kubernetes.io/name=microsoft-graph-userinfo-service`\n\
                 {HINT}   * An HTTP proxy / ingress may be returning an error page instead of the API response",
            ),
        )
    }
}

pub struct TestResult {
    #[allow(dead_code)]
    pub name: String,
    pub passed: bool,
    pub duration_ms: u128,
}

pub struct TestRunner {
    pub results: Vec<TestResult>,
    pub stopped_early: bool,
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            stopped_early: false,
        }
    }

    pub fn record_pass(&mut self, name: &str, duration_ms: u128) {
        println!("   {PASS} {name} ({duration_ms} ms)");
        self.results.push(TestResult {
            name: name.to_string(),
            passed: true,
            duration_ms,
        });
    }

    pub fn record_fail(&mut self, name: &str, duration_ms: u128, error: &TestError) {
        println!("   {FAIL} {name} ({duration_ms} ms)");
        println!();
        println!("      Error: {}", error.message);
        println!();
        for line in error.hint.lines() {
            println!("      {line}");
        }
        println!();
        self.results.push(TestResult {
            name: name.to_string(),
            passed: false,
            duration_ms,
        });
        self.stopped_early = true;
    }

    fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    fn print_summary(&self) {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let total_ms: u128 = self.results.iter().map(|r| r.duration_ms).sum();

        println!();
        println!("---------------------------------------------------");
        if failed == 0 {
            println!("All {total} tests passed in {total_ms} ms");
        } else {
            println!(
                "{FAIL} {passed} passed, {failed} failed ({total} total in {total_ms} ms){}",
                if self.stopped_early {
                    ", stopped after first failure"
                } else {
                    ""
                }
            );
        }
        println!("---------------------------------------------------");
        println!();
    }
}

/// Resolves the REST URL from the explicit flag, falling back to the back-compat
/// `SERVICE_URL` env var. Exits with a clear error when neither is set.
fn resolve_rest_url(args: &Args) -> Result<String, String> {
    match (&args.rest_url, &args.service_url) {
        (Some(u), _) if !u.is_empty() => Ok(u.clone()),
        (_, Some(u)) if !u.is_empty() => Ok(u.clone()),
        _ => Err(
            "REST URL is required. Set `--rest-url` or the `REST_URL` environment variable.".into(),
        ),
    }
}

/// Derives a default gRPC endpoint (`host:grpc_port`) from the REST URL when
/// `--grpc-endpoint` was not provided.
fn default_grpc_endpoint(rest_url: &str, grpc_port: u16) -> String {
    let without_scheme = rest_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(rest_url);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    format!("{host}:{grpc_port}")
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    HELM_MODE.store(args.helm, Ordering::Relaxed);

    let rest_url = match resolve_rest_url(&args) {
        Ok(u) => u,
        Err(msg) => {
            eprintln!("{FAIL} {msg}");
            return ExitCode::FAILURE;
        }
    };
    let grpc_endpoint = args
        .grpc_endpoint
        .clone()
        .unwrap_or_else(|| default_grpc_endpoint(&rest_url, args.grpc_port));

    println!();
    println!("===================================================");
    println!("  microsoft-graph-userinfo-service smoke test");
    println!("===================================================");
    println!("  REST URL:        {rest_url}");
    if args.grpc_enabled {
        println!(
            "  gRPC endpoint:   {grpc_endpoint}{tls}",
            tls = if args.grpc_tls_enabled { " (TLS)" } else { "" },
        );
    } else {
        println!("  gRPC endpoint:   (disabled)");
    }
    println!("  Service auth:    {}", describe_service_auth(&args));
    println!("  Graph base URL:  {}", args.graph_base_url);
    println!(
        "  Tenant ID:       {}",
        args.azure_tenant_id.as_deref().unwrap_or("(unset)")
    );
    println!(
        "  Client ID:       {}",
        args.azure_client_id.as_deref().unwrap_or("(unset)")
    );
    println!(
        "  Auth mode:       {}",
        match args.azure_auth_mode {
            AuthMode::ClientSecret => "client-secret",
            AuthMode::WorkloadIdentity => "workload-identity",
        }
    );
    println!(
        "  Environment:     {}",
        if args.helm {
            "Kubernetes (Helm)"
        } else {
            "Standalone"
        }
    );
    println!();

    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(args.http_timeout))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{FAIL} failed to build HTTP client: {e}");
            return ExitCode::FAILURE;
        }
    };

    let service_url = rest_url.trim_end_matches('/').to_string();
    let mut runner = TestRunner::new();

    let bearer = match rest::run_acquire_service_bearer_test(&mut runner, &args, &http).await {
        Ok(t) => t,
        Err(()) => {
            runner.print_summary();
            return ExitCode::FAILURE;
        }
    };

    println!();
    println!("Pre-flight: waiting for service to be reachable");
    if let Err(e) = rest::wait_for_service(
        &http,
        &service_url,
        bearer.as_deref(),
        args.wait_retries,
        args.wait_interval,
    )
    .await
    {
        eprintln!("{FAIL} {}", e.message);
        eprintln!();
        for line in e.hint.lines() {
            eprintln!("   {line}");
        }
        eprintln!();
        runner.print_summary();
        return ExitCode::FAILURE;
    }

    let rest_result =
        rest::run_rest_tests(&mut runner, &args, http, &service_url, bearer.as_deref()).await;

    if rest_result.is_ok() && args.grpc_enabled {
        let grpc_settings = grpc::GrpcSettings {
            endpoint: grpc_endpoint.clone(),
            tls_enabled: args.grpc_tls_enabled,
            timeout: Duration::from_secs(args.http_timeout),
            page_limit: args.page_limit,
        };
        let _ = grpc::run_grpc_tests(&mut runner, &grpc_settings, bearer.as_deref()).await;
    }

    runner.print_summary();

    if runner.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Renders the configured service-bearer source for the banner so an operator
/// can see at a glance which credential path Test 1 will exercise.
fn describe_service_auth(args: &Args) -> &'static str {
    if args.bearer_token.as_deref().is_some_and(|t| !t.is_empty()) {
        "literal bearer token"
    } else if args
        .access_token_file
        .as_deref()
        .is_some_and(|t| !t.is_empty())
    {
        "access-token file"
    } else if args
        .oauth2_openid_configuration_uri
        .as_deref()
        .is_some_and(|s| !s.is_empty())
        && args
            .oauth2_client_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    {
        "OAuth2 client-credentials (OpenID Connect Discovery)"
    } else {
        "(none — running unauthenticated)"
    }
}
