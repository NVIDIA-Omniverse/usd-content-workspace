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

use clap::Parser;
use permission_auth::config::load_openid_configuration;
use permission_auth::service_identity::{self, AccessTokenFileArgs, ClientCredentialsArgs};
use permission_core::{infer_policy_scopes, Action, Principal, Resource};
use permission_db::config_file::PolicyConfig;
use permission_service::http::policies::{PolicyResponse, PutPolicyPayload};
use permission_service::service::authorization::{
    AuthorizationDecision, AuthorizationPayload, AuthorizationResponse,
};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::time::sleep;

const PASS: &str = "✅";
const FAIL: &str = "❌";
const INFO: &str = "ℹ️";
const ROCKET: &str = "🚀";
const CHECK: &str = "✔";
const HINT: &str = "💡";
const WARN: &str = "⚠️";

static HELM_MODE: AtomicBool = AtomicBool::new(false);

fn is_helm_mode() -> bool {
    HELM_MODE.load(Ordering::Relaxed)
}

#[derive(Clone, Debug, Default, clap::ValueEnum, PartialEq)]
enum DatabaseType {
    #[default]
    Postgres,
    ConfigFile,
}

#[derive(Parser, Debug)]
#[command(version, about = "Smoke test for permission-service", long_about = None)]
struct Args {
    #[arg(long, env = "SERVICE_URL")]
    service_url: String,

    #[arg(long, env = "DATABASE_TYPE", value_enum, default_value_t = DatabaseType::Postgres)]
    database_type: DatabaseType,

    #[arg(long, env = "INIT_POLICIES_FILE")]
    policy_file: Option<String>,

    #[arg(long, env = "WAIT_RETRIES", default_value = "10")]
    wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value = "5")]
    wait_interval: u64,

    #[arg(long, env = "HELM", default_value = "false")]
    helm: bool,

    #[arg(long, env = "OPENID_CONFIGURATION")]
    openid_configuration: Option<String>,

    #[command(flatten)]
    client_credentials: ClientCredentialsArgs,

    #[command(flatten)]
    access_token_file: AccessTokenFileArgs,
}

struct TestError {
    message: String,
    hint: String,
}

impl TestError {
    fn connection_failed(url: &str, details: &str) -> Self {
        let details = if details.is_empty() {
            "connection refused or timed out"
        } else {
            details
        };
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Verify the pod is running: kubectl get pods -l app.kubernetes.io/name=permission-service\n\
                 {}   • Check pod logs: kubectl logs -l app.kubernetes.io/name=permission-service\n\
                 {}   • Helm values: service.ports.http (default: 3000), service.nameOverride",
                HINT, HINT, HINT
            )
        } else {
            format!(
                "{}   • SERVICE_URL points to a running service instance",
                HINT
            )
        };
        Self {
            message: format!("Cannot connect to service at {}", url),
            hint: format!(
                "Check that:\n\
                 {}   • The service is running and healthy\n\
                 {}   • Network connectivity and firewall rules allow the connection\n\
                 {}\n\
                 {}   Error: {}",
                HINT, HINT, config_hint, HINT, details
            ),
        }
    }

    fn unauthorized(url: &str) -> Self {
        let (client_hint, service_hint) = if is_helm_mode() {
            (
                format!(
                    "{}   • One of serviceIdentity.clientCredentials.enabled or serviceIdentity.accessTokenFile.enabled is true\n\
                     {}   • For client credentials: clientId and clientSecretRef are correct (serviceIdentity.clientCredentials.clientId, clientSecretRef.name/.key)\n\
                     {}   • For client credentials: OpenID configuration URI points to a valid IdP (serviceIdentity.clientCredentials.openIdConfigurationUri or openId.openIdConfigurationUri)\n\
                     {}   • For access token file: the provisioner sidecar (serviceIdentity.accessTokenFile.provisionerSidecar.container) writes a non-expired token",
                    HINT, HINT, HINT, HINT
                ),
                format!(
                    "{}   • Authentication is enabled on the service (openId.enabled=true)\n\
                     {}   • Token verification type matches your identity provider (openId.tokenVerificationType: jwt, opaque, or jwtuserinfo)\n\
                     {}   • The OpenID configuration URI is correct (openId.openIdConfigurationUri); jwks_uri and userinfo_endpoint come from there\n\
                     {}   • Audience matches token claims if used (openId.allowedJwtAudience)\n\
                     {}   • Token issuer is accepted (openId.allowedJwtIssuers), or left empty to accept any issuer",
                    HINT, HINT, HINT, HINT, HINT
                ),
            )
        } else {
            (
                format!(
                    "{}   • One of ENABLE_CLIENT_CREDENTIALS=true or ENABLE_ACCESS_TOKEN_FILE=true\n\
                     {}   • For client credentials: CLIENT_ID and CLIENT_SECRET/CLIENT_SECRET_FILE are correctly set\n\
                     {}   • For client credentials: CLIENT_CREDENTIALS_OPENID_CONFIGURATION (or OPENID_CONFIGURATION) points to a valid IdP discovery URI\n\
                     {}   • For access token file: ACCESS_TOKEN_FILE_PATH points to a JSON file with a non-expired token",
                    HINT, HINT, HINT, HINT
                ),
                format!(
                    "{}   • Authentication is enabled on the service (ENABLE_AUTH=true)\n\
                     {}   • Token verification type matches your identity provider (AUTH_TYPE: jwt, opaque, or jwtuserinfo)\n\
                     {}   • The OpenID configuration URI is correct (OPENID_CONFIGURATION); jwks_uri and userinfo_endpoint come from there\n\
                     {}   • Audience matches token claims if used (JWT_AUDIENCE)\n\
                     {}   • Token issuer is accepted (JWT_ISSUERS), or left empty to accept any issuer",
                    HINT, HINT, HINT, HINT, HINT
                ),
            )
        };
        Self {
            message: "Authentication failed (401 Unauthorized)".to_string(),
            hint: format!(
                "Verify the smoke test credentials:\n\
                 {}   • The client credentials have not expired\n\
                 {}\n\
                 Verify the service at {} is configured to accept tokens:\n\
                 {}",
                HINT, client_hint, url, service_hint
            ),
        }
    }

    fn forbidden() -> Self {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Init policies are correctly configured (database.init.policies)\n\
                 {}   • Client scope includes required permissions (serviceIdentity.clientCredentials.clientScope)",
                HINT, HINT
            )
        } else {
            format!(
                "{}   • Init policies are correctly configured (INIT_POLICIES_FILE)\n\
                 {}   • Client scope includes required permissions (CLIENT_SCOPE)",
                HINT, HINT
            )
        };
        Self {
            message: "Access denied (403 Forbidden)".to_string(),
            hint: format!(
                "Check that:\n\
                 {}   • The authenticated user/service has 'permissions:edit' permission\n\
                 {}",
                HINT, config_hint
            ),
        }
    }

    fn not_found(resource: &str) -> Self {
        Self {
            message: format!("{} not found (404)", resource),
            hint: format!(
                "Check that:\n\
                 {}   • The resource was created successfully in a previous step\n\
                 {}   • The database connection is working correctly\n\
                 {}   • There are no concurrent operations affecting the data",
                HINT, HINT, HINT
            ),
        }
    }

    fn bad_request(details: &str) -> Self {
        Self {
            message: format!("Invalid request (400 Bad Request): {}", details),
            hint: format!(
                "This may indicate:\n\
                 {}   • A bug in the smoke test or incompatible API version\n\
                 {}   • Policy validation is enabled and rejecting the test policy",
                HINT, HINT
            ),
        }
    }

    fn internal_error(details: &str) -> Self {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Database connection is correct (database.postgres.host, database.postgres.port, database.postgres.dbname)\n\
                 {}   • Database credentials are valid (database.postgres.user, the Secret named by database.postgres.secretRef.name)\n\
                 {}   • SSL mode matches your database setup (database.postgres.sslMode)\n\
                 {}   • Check logs: kubectl logs -l app.kubernetes.io/name=permission-service",
                HINT, HINT, HINT, HINT
            )
        } else {
            format!(
                "{}   • Database connection is correct (POSTGRES_HOST, POSTGRES_PORT, POSTGRES_DBNAME)\n\
                 {}   • Database credentials are valid (POSTGRES_USER, POSTGRES_PASSWORD)\n\
                 {}   • SSL mode matches your database setup (POSTGRES_SSL_MODE)",
                HINT, HINT, HINT
            )
        };
        Self {
            message: format!("Service error (500): {}", details),
            hint: format!(
                "Check that:\n\
                 {}   • The database (PostgreSQL) is running and accessible\n\
                 {}",
                HINT, config_hint
            ),
        }
    }

    fn service_unavailable() -> Self {
        Self {
            message: "Service unavailable (503)".to_string(),
            hint: format!(
                "The service is not ready. Check that:\n\
                 {}   • The service has finished starting up\n\
                 {}   • Database migrations have completed\n\
                 {}   • Health checks are passing: GET /health",
                HINT, HINT, HINT
            ),
        }
    }

    fn unexpected_decision(expected: &str, got: &str, reason: Option<&str>) -> Self {
        let reason_text = reason
            .map(|r| format!(" (reason: {})", r))
            .unwrap_or_default();
        let cache_hint = if is_helm_mode() {
            "config.evaluationCacheTtl"
        } else {
            "EVALUATION_CACHE_TTL"
        };
        Self {
            message: format!(
                "Expected '{}' decision but got '{}'{}",
                expected, got, reason_text
            ),
            hint: format!(
                "Check that:\n\
                 {}   • The policy was correctly added to the database\n\
                 {}   • The authorization request matches the policy constraints\n\
                 {}   • No conflicting policies exist that may override the test policy\n\
                 {}   • Cache settings ({}) may cause stale results",
                HINT, HINT, HINT, HINT, cache_hint
            ),
        }
    }

    fn from_reqwest_error(err: &reqwest::Error, url: &str) -> Self {
        if err.is_connect() {
            Self::connection_failed(url, &err.to_string())
        } else if err.is_timeout() {
            Self {
                message: "Request timed out".to_string(),
                hint: format!(
                    "Check that:\n\
                     {}   • The service is responsive and not overloaded\n\
                     {}   • Network latency is acceptable\n\
                     {}   • The service URL {} is reachable",
                    HINT, HINT, HINT, url
                ),
            }
        } else {
            Self {
                message: format!("Request failed: {}", err),
                hint: format!(
                    "Check that:\n\
                     {}   • The service is running at {}\n\
                     {}   • Network connectivity is available",
                    HINT, url, HINT
                ),
            }
        }
    }

    fn from_status(status: reqwest::StatusCode, body: &str, url: &str) -> Self {
        match status.as_u16() {
            401 => Self::unauthorized(url),
            403 => Self::forbidden(),
            404 => Self::not_found("Resource"),
            400 => Self::bad_request(body),
            500..=599 if status.as_u16() == 503 => Self::service_unavailable(),
            500..=599 => Self::internal_error(body),
            _ => Self {
                message: format!("HTTP {} - {}", status, body),
                hint: format!(
                    "{}   Unexpected status code. Check service logs for details.",
                    HINT
                ),
            },
        }
    }
}

#[allow(dead_code)]
struct TestResult {
    name: String,
    passed: bool,
    duration_ms: u128,
    error: Option<String>,
}

struct TestRunner {
    results: Vec<TestResult>,
}

impl TestRunner {
    fn new() -> Self {
        Self { results: vec![] }
    }

    fn record_pass(&mut self, name: &str, duration_ms: u128) {
        println!("   {} {} ({}ms)", PASS, name, duration_ms);
        self.results.push(TestResult {
            name: name.to_string(),
            passed: true,
            duration_ms,
            error: None,
        });
    }

    fn record_fail(&mut self, name: &str, duration_ms: u128, error: &TestError) {
        println!("   {} {} ({}ms)", FAIL, name, duration_ms);
        println!();
        println!("      Error: {}", error.message);
        println!();
        for line in error.hint.lines() {
            println!("      {}", line);
        }
        println!();
        self.results.push(TestResult {
            name: name.to_string(),
            passed: false,
            duration_ms,
            error: Some(error.message.clone()),
        });
    }

    fn print_summary(&self) {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let total_duration: u128 = self.results.iter().map(|r| r.duration_ms).sum();

        println!();
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        if failed == 0 {
            println!("🎉 All {} tests passed in {}ms", total, total_duration);
        } else {
            println!(
                "{} {} passed, {} failed ({} total in {}ms)",
                FAIL, passed, failed, total, total_duration
            );
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();
    }

    fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }
}

struct SmokeTest {
    client: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
}

impl SmokeTest {
    fn new(base_url: String, auth_token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            auth_token,
        }
    }

    fn auth_header(&self) -> Option<String> {
        self.auth_token
            .as_ref()
            .map(|token| format!("Bearer {}", token))
    }

    async fn check_health(&self) -> Result<(), TestError> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }

    async fn add_policy(&self, payload: &PutPolicyPayload) -> Result<PolicyResponse, TestError> {
        let url = format!("{}/v1beta/policies/", self.base_url);
        let mut request = self.client.put(&url).json(payload);

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        let status = response.status();
        if status.is_success() {
            let policy: PolicyResponse = response.json().await.map_err(|e| TestError {
                message: format!("Failed to parse response: {}", e),
                hint: format!("{}   The service returned invalid JSON", HINT),
            })?;
            Ok(policy)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }

    async fn get_policy(&self, id: i64) -> Result<PolicyResponse, TestError> {
        let url = format!("{}/v1beta/policies/{}", self.base_url, id);
        let mut request = self.client.get(&url);

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        let status = response.status();
        if status.is_success() {
            let policy: PolicyResponse = response.json().await.map_err(|e| TestError {
                message: format!("Failed to parse response: {}", e),
                hint: format!("{}   The service returned invalid JSON", HINT),
            })?;
            Ok(policy)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }

    async fn authorize(
        &self,
        payload: &AuthorizationPayload,
    ) -> Result<AuthorizationResponse, TestError> {
        let url = format!("{}/v1beta/authorization/", self.base_url);
        let mut request = self.client.post(&url).json(payload);

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        let status = response.status();
        if status.is_success() {
            let auth_response: AuthorizationResponse =
                response.json().await.map_err(|e| TestError {
                    message: format!("Failed to parse response: {}", e),
                    hint: format!("{}   The service returned invalid JSON", HINT),
                })?;
            Ok(auth_response)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }

    async fn get_policies_list(
        &self,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, TestError> {
        let url = format!("{}/v1beta/policies/", self.base_url);
        let mut request = self.client.get(&url).query(query);

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        let status = response.status();
        if status.is_success() {
            response.json().await.map_err(|e| TestError {
                message: format!("Failed to parse response: {}", e),
                hint: format!("{}   The service returned invalid JSON", HINT),
            })
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }

    async fn delete_policy(&self, id: i64) -> Result<(), TestError> {
        let url = format!("{}/v1beta/policies/{}", self.base_url, id);
        let mut request = self.client.delete(&url);

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TestError::from_reqwest_error(&e, &self.base_url))?;

        let status = response.status();
        if status.is_success() || status.as_u16() == 204 {
            Ok(())
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(TestError::from_status(status, &body, &self.base_url))
        }
    }
}

/// Acquires a Bearer access token via client credentials, an access-token file,
/// or returns `None` when outbound authentication is disabled.
async fn acquire_auth_token(args: &Args) -> Result<Option<String>, TestError> {
    if args.client_credentials.enabled {
        acquire_via_client_credentials(args).await
    } else if args.access_token_file.enabled {
        acquire_via_access_token_file(args).await
    } else {
        println!(
            "{} Authentication disabled, running without credentials",
            INFO
        );
        Ok(None)
    }
}

async fn acquire_via_client_credentials(args: &Args) -> Result<Option<String>, TestError> {
    let cc = &args.client_credentials;

    // Prefer CLIENT_CREDENTIALS_OPENID_CONFIGURATION (serviceIdentity.clientCredentials.openIdConfigurationUri)
    // and fall back to OPENID_CONFIGURATION (openId.openIdConfigurationUri) when it is left empty.
    let openid_configuration = if !cc.openid_configuration.is_empty() {
        cc.openid_configuration.clone()
    } else {
        args.openid_configuration.clone().unwrap_or_default()
    };
    if openid_configuration.is_empty() {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   Set serviceIdentity.clientCredentials.openIdConfigurationUri or openId.openIdConfigurationUri in Helm values",
                HINT
            )
        } else {
            format!(
                "{}   Set CLIENT_CREDENTIALS_OPENID_CONFIGURATION or OPENID_CONFIGURATION environment variable",
                HINT
            )
        };
        return Err(TestError {
            message: "OpenID configuration URI is required when client credentials are enabled"
                .to_string(),
            hint: format!(
                "{}\n\
                 {}   Example: https://auth.example.com/.well-known/openid-configuration",
                config_hint, HINT
            ),
        });
    }

    let client_secret = cc.get_client_secret().map_err(|e| {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Client secret reference is valid (serviceIdentity.clientCredentials.clientSecretRef.name and .key)\n\
                 {}   • The referenced Kubernetes Secret exists and contains the expected key",
                HINT, HINT
            )
        } else {
            format!(
                "{}   • CLIENT_SECRET environment variable is set, or\n\
                 {}   • CLIENT_SECRET_FILE points to a readable file",
                HINT, HINT
            )
        };
        TestError {
            message: format!("Failed to get client secret: {}", e),
            hint: format!("Check that:\n{}", config_hint),
        }
    })?;

    println!("{} Loading OpenID configuration...", INFO);

    let client = reqwest::Client::new();
    let openid_response = load_openid_configuration(&openid_configuration, &client).await.map_err(|e| {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • OpenID configuration endpoint is correct and reachable (serviceIdentity.clientCredentials.openIdConfigurationUri or openId.openIdConfigurationUri = {})\n\
                     {}   • OpenID configuration endpoint returns valid OpenID configuration metadata (jwks_uri, userinfo_endpoint, issuer, token_endpoint)",
                HINT, openid_configuration, HINT,
            )
        } else {
            format!(
                "{}   • OpenID configuration URI ({}) is correct and reachable\n\
                     {}   • The endpoint returns valid OpenID configuration metadata (jwks_uri, userinfo_endpoint, issuer, token_endpoint)",
                HINT, openid_configuration, HINT
            )
        };
        TestError {
            message: format!("Failed to load OpenID configuration: {}", e),
            hint: format!("Check that:\n{}", config_hint),
        }
    })?;

    let extra_headers = cc.get_extra_headers().map_err(|e| {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Verify serviceIdentity.clientCredentials.extraHeaders is a valid YAML map in Helm values\n\
                 {}   • Example: extraHeaders: {{\"X-Custom-Header\": \"value\"}}",
                HINT, HINT
            )
        } else {
            format!(
                "{}   • CLIENT_CREDENTIALS_EXTRA_HEADERS must be a valid JSON object\n\
                 {}   • Example: '{{\"X-Custom-Header\": \"value\"}}'",
                HINT, HINT
            )
        };
        TestError {
            message: format!("Failed to parse extra headers: {}", e),
            hint: format!("Check that:\n{}", config_hint),
        }
    })?;
    let extra_body_params = cc.get_extra_body_params().map_err(|e| {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   • Verify serviceIdentity.clientCredentials.extraBodyParams is a valid YAML map in Helm values\n\
                 {}   • Example: extraBodyParams: {{\"audience\": \"https://api.example.com\"}}",
                HINT, HINT
            )
        } else {
            format!(
                "{}   • CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS must be a valid JSON object\n\
                 {}   • Example: '{{\"audience\": \"https://api.example.com\"}}'",
                HINT, HINT
            )
        };
        TestError {
            message: format!("Failed to parse extra body params: {}", e),
            hint: format!("Check that:\n{}", config_hint),
        }
    })?;

    println!("{} Obtaining access token...", INFO);

    let response = service_identity::exchange_client_credentials_token(
        &client,
        &openid_response.token_endpoint,
        &cc.client_id,
        &client_secret,
        &cc.client_scope,
        &extra_headers,
        &extra_body_params,
    )
    .await
    .map_err(|e| {
        let err_str = e.to_string();
        if err_str.contains("connect") || err_str.contains("Connection") {
            TestError::connection_failed(&openid_response.token_endpoint, &err_str)
        } else {
            let config_hint = if is_helm_mode() {
                format!(
                    "{}   • Token endpoint ({}) returned from OpenID configuration is correct and reachable\n\
                     {}   • Client ID and secret are valid (serviceIdentity.clientCredentials.clientId, clientSecretRef)\n\
                     {}   • Client scope is accepted by the identity provider (serviceIdentity.clientCredentials.clientScope)",
                    HINT, openid_response.token_endpoint, HINT, HINT
                )
            } else {
                format!(
                    "{}   • Token endpoint ({}) returned from the OpenID configuration is correct and reachable\n\
                     {}   • CLIENT_ID and CLIENT_SECRET are valid\n\
                     {}   • CLIENT_SCOPE is accepted by the identity provider",
                    HINT, openid_response.token_endpoint, HINT, HINT
                )
            };
            TestError {
                message: format!("Token exchange failed: {}", e),
                hint: format!("Check that:\n{}", config_hint),
            }
        }
    })?;

    println!("{} Access token obtained", CHECK);
    Ok(Some(response.access_token))
}

async fn acquire_via_access_token_file(args: &Args) -> Result<Option<String>, TestError> {
    let cfg = &args.access_token_file;

    if cfg.file_path.is_empty() {
        let config_hint = if is_helm_mode() {
            format!(
                "{}   The chart sets ACCESS_TOKEN_FILE_PATH automatically when serviceIdentity.accessTokenFile.enabled=true; verify the smoke-test pod has the token-exchange volume mounted",
                HINT
            )
        } else {
            format!(
                "{}   Set --access-token-file-path or ACCESS_TOKEN_FILE_PATH",
                HINT
            )
        };
        return Err(TestError {
            message: "Access token file path is required when access token file mode is enabled"
                .to_string(),
            hint: config_hint,
        });
    }

    println!("{} Waiting for access token at {}...", INFO, cfg.file_path);

    let path = Path::new(&cfg.file_path);
    let mut last_error: Option<String> = None;
    for attempt in 1..=args.wait_retries {
        match service_identity::read_token_file(path) {
            Ok(token) => {
                println!("{} Access token loaded from file", CHECK);
                return Ok(Some(token));
            }
            Err(e) => {
                last_error = Some(e.to_string());
                if attempt < args.wait_retries {
                    sleep(Duration::from_secs(args.wait_interval)).await;
                }
            }
        }
    }

    let config_hint = if is_helm_mode() {
        format!(
            "{}   • The token provisioner sidecar (serviceIdentity.accessTokenFile.provisionerSidecar.container) is running and writing a valid token\n\
             {}   • The token file contains JSON with access_token and an RFC3339 expires_on in the future\n\
             {}   • Increase smokeTest.waitRetries / smokeTest.waitInterval if the sidecar needs more time to initialize",
            HINT, HINT, HINT
        )
    } else {
        format!(
            "{}   • ACCESS_TOKEN_FILE_PATH ({}) points to a readable JSON file\n\
             {}   • The file contains access_token and an RFC3339 expires_on in the future",
            HINT, cfg.file_path, HINT
        )
    };
    Err(TestError {
        message: format!(
            "Failed to read access token file: {}",
            last_error.unwrap_or_else(|| "no token available".to_string())
        ),
        hint: format!("Check that:\n{}", config_hint),
    })
}

const HOURGLASS: &str = "⏳";

async fn wait_for_service(
    smoke_test: &SmokeTest,
    base_url: &str,
    retries: u32,
    interval: u64,
) -> Result<(), TestError> {
    println!("{} Waiting for service to be ready...", HOURGLASS);

    for attempt in 1..=retries {
        print!("   Attempt {}/{}: ", attempt, retries);

        match smoke_test.check_health().await {
            Ok(()) => {
                println!("{} Service is ready!", CHECK);
                return Ok(());
            }
            Err(_) => {
                if attempt == retries {
                    println!("{}", FAIL);
                    let err = TestError::connection_failed(base_url, "");
                    return Err(TestError {
                        message: format!("Service not ready after {} attempts", retries),
                        hint: err.hint,
                    });
                }
                println!("not ready, waiting {}s...", interval);
                sleep(Duration::from_secs(interval)).await;
            }
        }
    }

    let err = TestError::connection_failed(base_url, "");
    Err(TestError {
        message: format!("Service not ready after {} attempts", retries),
        hint: err.hint,
    })
}

async fn cleanup_stale_policies(smoke_test: &SmokeTest) {
    let query: Vec<(&str, String)> = vec![
        ("principal", "smoke-test-principal".to_string()),
        ("action", "smoke-test:read".to_string()),
        (
            "resource",
            "SmokeTestResource::smoke-test-resource".to_string(),
        ),
    ];

    let mut stale_ids: Vec<i64> = Vec::new();
    let mut page = 1i64;

    loop {
        let mut page_query = query.clone();
        page_query.push(("page", page.to_string()));

        let result = match smoke_test.get_policies_list(&page_query).await {
            Ok(v) => v,
            Err(e) => {
                println!(
                    "{} Failed to check for stale smoke test policies: {}",
                    WARN, e.message
                );
                for line in e.hint.lines() {
                    println!("   {}", line);
                }
                println!(
                    "   {} Stale policies from previous runs may still exist. \
                    Query the policies API with principal=smoke-test-principal to check.",
                    WARN
                );
                return;
            }
        };

        let items = match result["items"].as_array() {
            Some(items) => items,
            None => {
                println!(
                    "{} Unexpected response format while checking for stale policies: \
                    missing 'items' array",
                    WARN
                );
                return;
            }
        };

        stale_ids.extend(items.iter().filter_map(|item| item["id"].as_i64()));

        let page_count = result["page_count"].as_i64().unwrap_or(1);
        if page >= page_count {
            break;
        }
        page += 1;
    }

    if stale_ids.is_empty() {
        return;
    }

    println!(
        "{} Found {} stale smoke test policies from a previous run",
        WARN,
        stale_ids.len()
    );
    cleanup_policies(smoke_test, &stale_ids).await;
    println!();
}

async fn cleanup_policies(smoke_test: &SmokeTest, policy_ids: &[i64]) {
    if policy_ids.is_empty() {
        return;
    }

    println!();
    println!("{} Cleaning up {} test policies...", INFO, policy_ids.len());

    for &id in policy_ids {
        match smoke_test.delete_policy(id).await {
            Ok(()) => println!("   {} Deleted policy {}", CHECK, id),
            Err(e) => {
                println!("   {} Failed to delete policy {}: {}", WARN, id, e.message);
                println!(
                    "   {} Manual cleanup may be required (DELETE /v1beta/policies/{})",
                    WARN, id
                );
            }
        }
    }
}

async fn run_smoke_tests(smoke_test: &SmokeTest) -> TestRunner {
    let mut runner = TestRunner::new();
    let mut policies_to_cleanup: Vec<i64> = Vec::new();

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("{} Running Smoke Tests", ROCKET);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let test_principal = "smoke-test-principal";
    let test_action = Action {
        name: "read".to_string(),
        service: "smoke-test".to_string(),
    };
    let test_resource = Resource {
        id: "smoke-test-resource".to_string(),
        kind: "SmokeTestResource".to_string(),
        data: None,
    };

    let start = Instant::now();
    match smoke_test.check_health().await {
        Ok(()) => runner.record_pass("Health Check", start.elapsed().as_millis()),
        Err(e) => {
            runner.record_fail("Health Check", start.elapsed().as_millis(), &e);
            return runner;
        }
    }

    cleanup_stale_policies(smoke_test).await;

    let policy_payload = PutPolicyPayload {
        policy: format!(
            r#"permit(principal == Principal::"{}", action == Action::"{}:{}", resource == {}::"{}");"#,
            test_principal,
            test_action.service,
            test_action.name,
            test_resource.kind,
            test_resource.id
        ),
        order: None,
        comment: None,
    };

    let start = Instant::now();
    let added_policy = match smoke_test.add_policy(&policy_payload).await {
        Ok(policy) => {
            runner.record_pass("Add Policy", start.elapsed().as_millis());
            policies_to_cleanup.push(policy.id);
            policy
        }
        Err(e) => {
            runner.record_fail("Add Policy", start.elapsed().as_millis(), &e);
            return runner;
        }
    };

    let start = Instant::now();
    match smoke_test.get_policy(added_policy.id).await {
        Ok(retrieved) => {
            if retrieved.policy == added_policy.policy {
                runner.record_pass("Retrieve Policy", start.elapsed().as_millis());
            } else {
                runner.record_fail(
                    "Retrieve Policy",
                    start.elapsed().as_millis(),
                    &TestError {
                        message: "Policy content mismatch".to_string(),
                        hint: format!(
                            "{}   The retrieved policy differs from the one that was added.\n\
                             {}   This may indicate a database or caching issue.",
                            HINT, HINT
                        ),
                    },
                );
                cleanup_policies(smoke_test, &policies_to_cleanup).await;
                return runner;
            }
        }
        Err(e) => {
            runner.record_fail("Retrieve Policy", start.elapsed().as_millis(), &e);
            cleanup_policies(smoke_test, &policies_to_cleanup).await;
            return runner;
        }
    }

    let auth_payload = AuthorizationPayload {
        principal: Some(Principal {
            id: test_principal.to_string(),
            info: None,
        }),
        action: test_action.clone(),
        resource: Some(test_resource.clone()),
        context: None,
    };

    let start = Instant::now();
    match smoke_test.authorize(&auth_payload).await {
        Ok(response) => {
            if response.decision == AuthorizationDecision::Allow {
                runner.record_pass("Authorization (Allow)", start.elapsed().as_millis());
            } else {
                runner.record_fail(
                    "Authorization (Allow)",
                    start.elapsed().as_millis(),
                    &TestError::unexpected_decision(
                        "Allow",
                        &format!("{:?}", response.decision),
                        response.reason.as_deref(),
                    ),
                );
                cleanup_policies(smoke_test, &policies_to_cleanup).await;
                return runner;
            }
        }
        Err(e) => {
            runner.record_fail("Authorization (Allow)", start.elapsed().as_millis(), &e);
            cleanup_policies(smoke_test, &policies_to_cleanup).await;
            return runner;
        }
    }

    let start = Instant::now();
    match smoke_test.delete_policy(added_policy.id).await {
        Ok(()) => {
            runner.record_pass("Delete Policy", start.elapsed().as_millis());
            policies_to_cleanup.retain(|&id| id != added_policy.id);
        }
        Err(e) => {
            runner.record_fail("Delete Policy", start.elapsed().as_millis(), &e);
            cleanup_policies(smoke_test, &policies_to_cleanup).await;
            return runner;
        }
    }

    let start = Instant::now();
    match smoke_test.authorize(&auth_payload).await {
        Ok(response) => {
            if response.decision == AuthorizationDecision::Deny {
                runner.record_pass(
                    "Authorization (Deny after delete)",
                    start.elapsed().as_millis(),
                );
            } else {
                runner.record_fail(
                    "Authorization (Deny after delete)",
                    start.elapsed().as_millis(),
                    &TestError::unexpected_decision(
                        "Deny",
                        &format!("{:?}", response.decision),
                        response.reason.as_deref(),
                    ),
                );
            }
        }
        Err(e) => {
            runner.record_fail(
                "Authorization (Deny after delete)",
                start.elapsed().as_millis(),
                &e,
            );
        }
    }

    cleanup_policies(smoke_test, &policies_to_cleanup).await;
    runner
}

async fn run_config_file_smoke_tests(
    smoke_test: &SmokeTest,
    policy_file: &Option<String>,
) -> TestRunner {
    let mut runner = TestRunner::new();

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("{} Running Smoke Tests", ROCKET);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let policy_config = match policy_file {
        Some(path) => {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_norway::from_str::<PolicyConfig>(&content) {
                    Ok(config) => {
                        println!(
                            "{} Loaded policy config: {} policies from {}",
                            INFO,
                            config.policies.len(),
                            path
                        );
                        Some(config)
                    }
                    Err(e) => {
                        runner.record_fail("Load Policy Config", 0, &TestError {
                        message: format!("Failed to parse policy config: {}", e),
                        hint: format!("{}   Check that {} contains valid YAML in PolicyConfig format.", HINT, path),
                    });
                        return runner;
                    }
                },
                Err(e) => {
                    runner.record_fail(
                        "Load Policy Config",
                        0,
                        &TestError {
                            message: format!("Failed to read policy file: {}", e),
                            hint: format!("{}   Check that {} exists and is readable.", HINT, path),
                        },
                    );
                    return runner;
                }
            }
        }
        None => {
            println!(
                "{} No policy file specified, skipping policy content verification",
                INFO
            );
            None
        }
    };
    println!();

    let start = Instant::now();
    match smoke_test.check_health().await {
        Ok(()) => runner.record_pass("Health Check", start.elapsed().as_millis()),
        Err(e) => {
            runner.record_fail("Health Check", start.elapsed().as_millis(), &e);
            return runner;
        }
    }

    let start = Instant::now();
    match smoke_test.get_policies_list(&[]).await {
        Ok(result) => {
            if let Some(items) = result["items"].as_array() {
                if let Some(ref config) = policy_config {
                    if items.len() == config.policies.len() {
                        runner.record_pass(
                            "Get Policies (count matches config)",
                            start.elapsed().as_millis(),
                        );
                    } else {
                        runner.record_fail(
                            "Get Policies (count matches config)",
                            start.elapsed().as_millis(),
                            &TestError {
                                message: format!(
                                    "Policy count mismatch: service returned {} policies, config file has {}",
                                    items.len(),
                                    config.policies.len()
                                ),
                                hint: format!("{}   The service should return all policies from the configuration file.", HINT),
                            },
                        );
                        return runner;
                    }
                } else if items.is_empty() {
                    runner.record_fail(
                        "Get Policies",
                        start.elapsed().as_millis(),
                        &TestError {
                            message: "No policies found".to_string(),
                            hint: format!("{}   The policy configuration file must contain at least one policy.", HINT),
                        },
                    );
                    return runner;
                } else {
                    runner.record_pass("Get Policies", start.elapsed().as_millis());
                }
            } else {
                runner.record_fail(
                    "Get Policies",
                    start.elapsed().as_millis(),
                    &TestError {
                        message: "Unexpected response format: missing 'items' array".to_string(),
                        hint: format!(
                            "{}   The service returned an unexpected JSON structure.",
                            HINT
                        ),
                    },
                );
                return runner;
            }
        }
        Err(e) => {
            runner.record_fail("Get Policies", start.elapsed().as_millis(), &e);
            return runner;
        }
    }

    if let Some(ref config) = policy_config {
        if let Some((action, principal, resource)) = config.policies.iter().find_map(|entry| {
            let scopes = infer_policy_scopes(&entry.policy).ok()?;
            let action = scopes.action?;
            Some((action, scopes.principal, scopes.resource))
        }) {
            let auth_payload = AuthorizationPayload {
                principal: principal.map(|p| Principal {
                    id: p.id.clone(),
                    info: None,
                }),
                action,
                resource,
                context: None,
            };

            let start = Instant::now();
            match smoke_test.authorize(&auth_payload).await {
                Ok(response) => {
                    if response.decision == AuthorizationDecision::Allow {
                        runner.record_pass("Authorization", start.elapsed().as_millis());
                    } else {
                        runner.record_fail(
                            "Authorization",
                            start.elapsed().as_millis(),
                            &TestError::unexpected_decision(
                                "Allow",
                                &format!("{:?}", response.decision),
                                response.reason.as_deref(),
                            ),
                        );
                    }
                }
                Err(e) => {
                    runner.record_fail("Authorization", start.elapsed().as_millis(), &e);
                }
            }
        }
    }

    runner
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    HELM_MODE.store(args.helm, Ordering::Relaxed);

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Permission Service Smoke Test");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("  Service URL: {}", args.service_url);
    println!("  Wait retries: {}", args.wait_retries);
    println!("  Wait interval: {}s", args.wait_interval);
    println!(
        "  Environment: {}",
        if args.helm {
            "Kubernetes (Helm)"
        } else {
            "Standalone"
        }
    );
    println!(
        "  Database type: {}",
        match args.database_type {
            DatabaseType::Postgres => "postgres",
            DatabaseType::ConfigFile => "config-file",
        }
    );
    println!(
        "  Authentication: {}",
        if args.client_credentials.enabled {
            "enabled (client credentials)"
        } else if args.access_token_file.enabled {
            "enabled (access token file)"
        } else {
            "disabled"
        }
    );
    println!();

    let smoke_test = SmokeTest::new(args.service_url.clone(), None);
    if let Err(e) = wait_for_service(
        &smoke_test,
        &args.service_url,
        args.wait_retries,
        args.wait_interval,
    )
    .await
    {
        eprintln!("{} {}", FAIL, e.message);
        eprintln!();
        for line in e.hint.lines() {
            eprintln!("   {}", line);
        }
        eprintln!();
        return ExitCode::FAILURE;
    }
    println!();

    let auth_token = match acquire_auth_token(&args).await {
        Ok(token) => token,
        Err(e) => {
            eprintln!("{} Setup failed: {}", FAIL, e.message);
            eprintln!();
            for line in e.hint.lines() {
                eprintln!("   {}", line);
            }
            eprintln!();
            return ExitCode::FAILURE;
        }
    };

    let smoke_test = SmokeTest::new(args.service_url, auth_token);
    let runner = match args.database_type {
        DatabaseType::Postgres => run_smoke_tests(&smoke_test).await,
        DatabaseType::ConfigFile => {
            run_config_file_smoke_tests(&smoke_test, &args.policy_file).await
        }
    };

    runner.print_summary();

    if runner.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
