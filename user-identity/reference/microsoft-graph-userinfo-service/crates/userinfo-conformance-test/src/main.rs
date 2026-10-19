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

//! Conformance test for the Microsoft Graph User Info Service public API.
//!
//! Verifies that an arbitrary implementation of the User Info Service satisfies
//! the published REST and gRPC API contract: correct paths, status codes,
//! response shapes (required / optional fields),
//! error handling, pagination and filtering semantics. The test does not
//! exercise any implementation-specific configuration (Microsoft Graph,
//! caching, etc.) — it treats the service as a black box behind the public
//! REST and gRPC interfaces.
//!
//! Module layout:
//! - `main.rs`: shared `TestError` / `TestRunner` primitives, top-level CLI
//!   orchestration, and the readiness wait loop.
//! - `config`: CLI / environment configuration and bearer-token acquisition.
//! - `fixtures`: samples the small set of test fixtures (sample user,
//!   sample group, membership pair, etc.) that the per-endpoint contract
//!   tests need from the live service.
//! - `rest`: REST conformance tests (one module per endpoint group).
//! - `grpc`: gRPC conformance tests (one module per RPC group).
//! - `style`: console styling helpers (color, banner, section separators).

mod config;
mod fixtures;
mod grpc;
mod rest;
mod style;

use clap::Parser;
use config::{Args, TestSuite, acquire_bearer_token};
use std::process::ExitCode;
use std::time::Duration;
use tokio::time::sleep;

/// The literal `HINT` token embedded inside multi-line hint strings. Kept
/// as a plain word (not styled) because hints are rendered as a single
/// dimmed block by `TestRunner::record_fail`; embedding ANSI escapes here
/// would break that grouping. See `style::pass_marker`,
/// `style::fail_marker`, and `style::info_marker` for the runtime-styled
/// markers used at the start of result / progress lines.
pub const HINT: &str = "HINT";

#[derive(Clone, Copy, PartialEq)]
pub enum TestStatus {
    Passed,
    Failed,
}

/// Result of a single named test case.
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub duration_ms: u128,
    pub error: Option<String>,
}

/// Structured failure with a remediation hint and optional request / response
/// transcripts. Hints are deliberately verbose because conformance failures
/// are typically diagnosed by people who did not write the implementation.
pub struct TestError {
    pub message: String,
    pub hint: String,
    pub request: Option<String>,
    pub response: Option<String>,
}

impl TestError {
    pub fn new(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: hint.into(),
            request: None,
            response: None,
        }
    }

    pub fn with_request(mut self, request: impl Into<String>) -> Self {
        self.request = Some(request.into());
        self
    }

    pub fn with_response(mut self, response: impl Into<String>) -> Self {
        self.response = Some(response.into());
        self
    }

    pub fn connection_failed(url: &str, details: &str) -> Self {
        let details = if details.is_empty() {
            "connection refused or timed out".to_string()
        } else {
            details.to_string()
        };
        Self::new(
            format!("Cannot connect to service at {url}"),
            format!(
                "Check that:\n\
                 {HINT}   * The service is running and listening on the expected port\n\
                 {HINT}   * Network / firewall allows the connection from where the test runs\n\
                 {HINT}   * Raw error: {details}",
            ),
        )
    }

    pub fn from_reqwest_error(err: &reqwest::Error, url: &str) -> Self {
        if err.is_connect() {
            Self::connection_failed(url, &err.to_string())
        } else if err.is_timeout() {
            Self::new(
                format!("Request to {url} timed out"),
                format!(
                    "{HINT}   * The service may be overloaded or its initial directory sync may not have completed\n\
                     {HINT}   * Increase --http-timeout or --wait-retries / --wait-interval",
                ),
            )
        } else {
            Self::new(
                format!("HTTP request failed: {err}"),
                format!("{HINT}   * Check that the service at {url} is healthy"),
            )
        }
    }

    pub fn unexpected_status(step: &str, status: reqwest::StatusCode, body: &str) -> Self {
        let body_preview: String = body.chars().take(500).collect();
        let code = status.as_u16();
        let hint = match code {
            401 => format!(
                "HTTP 401 Unauthorized. The service rejected the bearer token.\n\
                 {HINT}   * Confirm a token is configured (--bearer-token / --bearer-token-file / --oauth2-*)\n\
                 {HINT}   * The token must be accepted by whatever auth layer fronts the service",
            ),
            403 => format!(
                "HTTP 403 Forbidden. The caller is authenticated but lacks permission.\n\
                 {HINT}   * If permission integration is enabled on the service, ensure the caller principal is allowed to read the target user / group\n\
                 {HINT}   * Required actions: list-users, get-user, list-groups, get-group, list-user-groups, get-user-group, list-group-members, get-group-member",
            ),
            404 => format!(
                "HTTP 404 Not Found. The target resource does not exist or has been hidden by post-filter authorization.\n\
                 {HINT}   * The conformance test sampled the target identifier from a prior list call, so a 404 here\n\
                 {HINT}     usually means the entity was deleted between sampling and this call, or post-filter authorization\n\
                 {HINT}     is hiding an entity the caller was allowed to list",
            ),
            429 => format!(
                "HTTP 429 Too Many Requests. The service is rate-limited.\n\
                 {HINT}   * Retry after a short delay; reduce parallelism if running against a shared environment",
            ),
            500..=599 => format!(
                "HTTP {code} server error.\n\
                 {HINT}   * Inspect the service logs and upstream dependencies (identity provider, datastore)\n\
                 {HINT}   * Response body (truncated): {body_preview}",
            ),
            _ => format!("{HINT}   * Response body (truncated): {body_preview}"),
        };
        Self::new(format!("{step} returned HTTP {code}"), hint)
    }

    pub fn invalid_json(step: &str, err: &reqwest::Error) -> Self {
        Self::new(
            format!("{step}: failed to parse JSON response ({err})"),
            format!(
                "{HINT}   * The endpoint must return a JSON body matching the documented schema\n\
                 {HINT}   * An ingress / proxy may be returning an HTML error page",
            ),
        )
    }
}

pub struct TestRunner {
    pub results: Vec<TestResult>,
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
        }
    }

    pub fn record_pass(&mut self, name: &str, duration_ms: u128) {
        println!(
            "   {} {} {}",
            style::pass_marker(),
            name,
            style::dim(&format!("({duration_ms} ms)")),
        );
        self.results.push(TestResult {
            name: name.to_string(),
            status: TestStatus::Passed,
            duration_ms,
            error: None,
        });
    }

    pub fn record_fail(&mut self, name: &str, duration_ms: u128, error: &TestError) {
        println!(
            "   {} {} {}",
            style::fail_marker(),
            style::bold(name),
            style::dim(&format!("({duration_ms} ms)")),
        );
        println!();
        println!("      {} {}", style::bold_red("Error:"), error.message);
        if let Some(ref request) = error.request {
            println!();
            println!("      {}", style::bold("Request:"));
            for line in request.lines() {
                println!("        {}", style::dim(line));
            }
        }
        if let Some(ref response) = error.response {
            println!();
            println!("      {}", style::bold("Response:"));
            for line in response.lines() {
                println!("        {}", style::dim(line));
            }
        }
        if !error.hint.is_empty() {
            println!();
            for line in error.hint.lines() {
                println!("      {}", style::dim(line));
            }
        }
        println!();
        self.results.push(TestResult {
            name: name.to_string(),
            status: TestStatus::Failed,
            duration_ms,
            error: Some(error.message.clone()),
        });
    }

    pub fn no_failures(&self) -> bool {
        self.results.iter().all(|r| r.status != TestStatus::Failed)
    }

    fn print_summary(&self) {
        let total = self.results.len();
        let passed = self
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Passed)
            .count();
        let failed = self
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .count();
        let total_ms: u128 = self.results.iter().map(|r| r.duration_ms).sum();

        style::print_section("Summary");
        let counts = format!("({total} total in {total_ms} ms)");
        if failed == 0 {
            println!(
                "  {} {} {}",
                style::pass_marker(),
                style::bold_green(&format!("{passed} passed")),
                style::dim(&counts),
            );
        } else {
            println!(
                "  {} {}, {} {}",
                style::fail_marker(),
                style::bold_green(&format!("{passed} passed")),
                style::bold_red(&format!("{failed} failed")),
                style::dim(&counts),
            );
        }
        println!();
    }
}

/// Polls the configured readiness path on the REST URL until it responds
/// with 2xx or the retry budget is exhausted.
async fn wait_for_service(
    client: &reqwest::Client,
    rest_url: &str,
    ready_path: &str,
    auth_header: Option<&str>,
    retries: u32,
    interval_s: u64,
) -> Result<(), TestError> {
    let url = format!("{}{}", rest_url.trim_end_matches('/'), ready_path);
    let info = style::info_marker();
    println!(
        "   {info} waiting for service at {url} (up to {} s)",
        retries as u64 * interval_s
    );
    for attempt in 1..=retries {
        let mut req = client.get(&url);
        if let Some(auth) = auth_header {
            req = req.header("Authorization", auth);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("   {info} service is ready (attempt {attempt}/{retries})");
                return Ok(());
            }
            Ok(resp) => {
                println!(
                    "   {info} not ready yet (HTTP {}), retrying in {interval_s} s...",
                    resp.status().as_u16()
                );
            }
            Err(e) => {
                println!("   {info} not reachable ({e}), retrying in {interval_s} s...");
            }
        }
        if attempt < retries {
            sleep(Duration::from_secs(interval_s)).await;
        }
    }
    Err(TestError::connection_failed(
        &url,
        "service did not become ready within the retry budget",
    ))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let fail = style::fail_marker();

    style::print_banner("User Info Service API conformance test");
    println!(
        "  {} Test suite:        {}",
        style::dim("\u{2022}"),
        style::bold(&format!("{:?}", args.test_suite)),
    );
    if let Some(ref u) = args.rest_url {
        println!("  {} REST URL:          {u}", style::dim("\u{2022}"));
    }
    if let Some(ref u) = args.grpc_url {
        let tls = if args.grpc_tls_enabled { " (TLS)" } else { "" };
        println!("  {} gRPC URL:          {u}{tls}", style::dim("\u{2022}"));
    }
    println!(
        "  {} Page limit:        {}",
        style::dim("\u{2022}"),
        args.page_limit,
    );

    if args.test_suite != TestSuite::Grpc && args.rest_url.is_none() {
        eprintln!("{fail} --rest-url is required for the REST suite");
        return ExitCode::FAILURE;
    }
    if args.test_suite != TestSuite::Rest && args.grpc_url.is_none() {
        eprintln!("{fail} --grpc-url is required for the gRPC suite");
        return ExitCode::FAILURE;
    }

    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(args.http_timeout))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{fail} failed to build HTTP client: {e}");
            return ExitCode::FAILURE;
        }
    };

    style::print_section("Authentication");
    let bearer = match acquire_bearer_token(&http, &args).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{fail} {}", e.message);
            for line in e.hint.lines() {
                eprintln!("   {}", style::dim(line));
            }
            return ExitCode::FAILURE;
        }
    };
    let auth_header = bearer.as_deref().map(|t| format!("Bearer {t}"));

    let mut runner = TestRunner::new();

    if let Some(ref rest_url) = args.rest_url {
        style::print_section("Pre-flight: waiting for the service to become ready");
        if let Err(e) = wait_for_service(
            &http,
            rest_url,
            &args.ready_path,
            auth_header.as_deref(),
            args.wait_retries,
            args.wait_interval,
        )
        .await
        {
            eprintln!("{fail} {}", e.message);
            for line in e.hint.lines() {
                eprintln!("   {}", style::dim(line));
            }
            return ExitCode::FAILURE;
        }
    }

    if matches!(args.test_suite, TestSuite::All | TestSuite::Rest)
        && let Some(rest_url) = args.rest_url.as_deref()
    {
        let rest_url = rest_url.trim_end_matches('/');
        style::print_section("Fixture sampling (REST)");
        match fixtures::sample_via_rest(&http, rest_url, auth_header.as_deref(), args.page_limit)
            .await
        {
            Ok(data) => {
                style::print_section("REST conformance");
                rest::run(&mut runner, &http, rest_url, auth_header.as_deref(), &data).await;
            }
            Err(e) => {
                runner.record_fail("REST fixture sampling", 0, &e);
            }
        }
    }

    if matches!(args.test_suite, TestSuite::All | TestSuite::Grpc)
        && let Some(grpc_url) = args.grpc_url.as_deref()
    {
        style::print_section("gRPC connection");
        match grpc::build_client(grpc_url, args.grpc_tls_enabled, args.http_timeout).await {
            Ok(mut client) => {
                println!("   {} connected to {grpc_url}", style::info_marker(),);
                style::print_section("Fixture sampling (gRPC)");
                match fixtures::sample_via_grpc(&mut client, bearer.as_deref(), args.page_limit)
                    .await
                {
                    Ok(data) => {
                        style::print_section("gRPC conformance");
                        grpc::run(&mut runner, &mut client, bearer.as_deref(), &args, &data).await;
                    }
                    Err(e) => {
                        runner.record_fail("gRPC fixture sampling", 0, &e);
                    }
                }
            }
            Err(e) => {
                runner.record_fail("Connect to gRPC endpoint", 0, &e);
            }
        }
    }

    runner.print_summary();

    if runner.no_failures() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
