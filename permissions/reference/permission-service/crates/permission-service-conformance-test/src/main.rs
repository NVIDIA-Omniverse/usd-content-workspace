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

mod config;
mod grpc;
mod rest;

use clap::Parser;
use config::{acquire_auth_token, Args, TestSuite};
use std::process::ExitCode;
use std::time::Duration;
use tokio::time::sleep;

const PASS: &str = "\u{2705}";
const FAIL: &str = "\u{274C}";
const INFO: &str = "\u{2139}\u{FE0F}";
const ROCKET: &str = "\u{1F680}";
const CHECK: &str = "\u{2714}\u{FE0F}";
const HINT: &str = "\u{1F4A1}";
#[allow(dead_code)]
const WARN: &str = "\u{26A0}\u{FE0F}";
const HOURGLASS: &str = "\u{231B}";

pub const DENY_ACTION_NAME: &str = "deny-action";
pub const DENY_ACTION_SERVICE: &str = "conformance-test-nonexistent";

pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
}

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
            "connection refused or timed out"
        } else {
            details
        };
        Self::new(
            format!("Cannot connect to service at {}", url),
            format!(
                "Check that:\n\
                 {}   The service is running and healthy\n\
                 {}   Network connectivity and firewall rules allow the connection\n\
                 {}   Error: {}",
                HINT, HINT, HINT, details
            ),
        )
    }

    pub fn unexpected_decision(expected: &str, got: &str, args: &Args) -> Self {
        Self::new(
            format!("Expected '{}' but got '{}'", expected, got),
            format!(
                "Ensure the following data is prepared before running this test:\n\
                 {}   An authorization rule that permits principal \"{}\" to perform \
                 action \"{}:{}\" on resource {}::\"{}\"",
                HINT,
                args.principal,
                args.action_service,
                args.action_name,
                args.resource_type,
                args.resource_id
            ),
        )
    }

    pub fn from_reqwest_error(err: &reqwest::Error, url: &str) -> Self {
        if err.is_connect() {
            Self::connection_failed(url, &err.to_string())
        } else if err.is_timeout() {
            Self::new(
                "Request timed out",
                format!(
                    "{}   The service at {} may be overloaded or unreachable",
                    HINT, url
                ),
            )
        } else {
            Self::new(
                format!("Request failed: {}", err),
                format!("{}   Check that the service is running at {}", HINT, url),
            )
        }
    }

    pub fn from_status(status: reqwest::StatusCode, body: &str, url: &str) -> Self {
        match status.as_u16() {
            401 => Self::new(
                "Authentication failed (401 Unauthorized)",
                format!(
                    "{}   Verify credentials and token configuration for {}",
                    HINT, url
                ),
            ),
            403 => Self::new(
                "Access denied (403 Forbidden)",
                format!(
                    "{}   The authenticated identity lacks the required permissions",
                    HINT
                ),
            ),
            429 => Self::new(
                "Rate limited (429 Too Many Requests)",
                format!("{}   Retry after a short delay", HINT),
            ),
            500..=599 => Self::new(
                format!("Service error ({}): {}", status, body),
                format!("{}   Check the service logs for details", HINT),
            ),
            _ => Self::new(
                format!("HTTP {} - {}", status, body),
                format!("{}   Unexpected status code from {}", HINT, url),
            ),
        }
    }
}

pub struct TestRunner {
    results: Vec<TestResult>,
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner {
    pub fn new() -> Self {
        Self { results: vec![] }
    }

    pub fn record_pass(&mut self, name: &str, duration_ms: u128) {
        println!("   {} {} ({}ms)", PASS, name, duration_ms);
        self.results.push(TestResult {
            name: name.to_string(),
            passed: true,
            duration_ms,
            error: None,
        });
    }

    pub fn record_fail(&mut self, name: &str, duration_ms: u128, error: &TestError) {
        println!("   {} {} ({}ms)", FAIL, name, duration_ms);
        println!();
        println!("      Error: {}", error.message);
        if let Some(ref request) = error.request {
            println!();
            println!("      Request sent:");
            for line in request.lines() {
                println!("        {}", line);
            }
        }
        if let Some(ref response) = error.response {
            println!();
            println!("      Response received:");
            for line in response.lines() {
                println!("        {}", line);
            }
        }
        if !error.hint.is_empty() {
            println!();
            for line in error.hint.lines() {
                println!("      {}", line);
            }
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
            println!(
                "{} All {} tests passed in {}ms",
                PASS, total, total_duration
            );
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

async fn wait_for_service(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    retries: u32,
    interval: u64,
) -> Result<(), TestError> {
    println!("{} Waiting for service to be ready...", HOURGLASS);

    for attempt in 1..=retries {
        print!("   Attempt {}/{}: ", attempt, retries);

        let result = rest::health::check(client, rest_url, auth_header).await;
        match result {
            Ok(()) => {
                println!("{} Service is ready!", CHECK);
                return Ok(());
            }
            Err(_) => {
                if attempt == retries {
                    println!("{}", FAIL);
                    return Err(TestError::connection_failed(rest_url, ""));
                }
                println!("not ready, waiting {}s...", interval);
                sleep(Duration::from_secs(interval)).await;
            }
        }
    }

    Err(TestError::connection_failed(rest_url, ""))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Permission Service Conformance Test");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("  Test suite:  {:?}", args.test_suite);
    if let Some(ref url) = args.rest_url {
        println!("  REST URL:    {}", url);
    }
    if let Some(ref url) = args.grpc_url {
        println!("  gRPC URL:    {}", url);
    }
    println!("  Principal:   {}", args.principal);
    println!(
        "  Action:      {}:{}",
        args.action_service, args.action_name
    );
    println!(
        "  Resource:    {}::\"{}\"",
        args.resource_type, args.resource_id
    );
    println!(
        "  Auth:        {}",
        if args.client_credentials.enabled {
            "client credentials"
        } else if args.access_token_file.enabled {
            "access token file"
        } else {
            "disabled"
        }
    );
    println!();

    if args.test_suite != TestSuite::Grpc && args.rest_url.is_none() {
        eprintln!("{} --rest-url is required for REST tests", FAIL);
        return ExitCode::FAILURE;
    }
    if args.test_suite != TestSuite::Rest && args.grpc_url.is_none() {
        eprintln!("{} --grpc-url is required for gRPC tests", FAIL);
        return ExitCode::FAILURE;
    }

    let access_token = match acquire_auth_token(&args).await {
        Ok(token) => token,
        Err(e) => {
            eprintln!("{} Setup failed: {}", FAIL, e.message);
            for line in e.hint.lines() {
                eprintln!("   {}", line);
            }
            return ExitCode::FAILURE;
        }
    };
    let auth_header = access_token.as_deref().map(|t| format!("Bearer {}", t));

    let client = reqwest::Client::new();
    let mut runner = TestRunner::new();

    if let Some(ref rest_url) = args.rest_url {
        if let Err(e) = wait_for_service(
            &client,
            rest_url,
            auth_header.as_deref(),
            args.wait_retries,
            args.wait_interval,
        )
        .await
        {
            eprintln!("{} {}", FAIL, e.message);
            for line in e.hint.lines() {
                eprintln!("   {}", line);
            }
            return ExitCode::FAILURE;
        }
    }
    println!();

    if args.test_suite == TestSuite::All || args.test_suite == TestSuite::Rest {
        let rest_url = args.rest_url.as_deref().unwrap();

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("{} REST Tests", ROCKET);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();

        rest::health::run_tests(&mut runner, &client, rest_url, auth_header.as_deref()).await;
        rest::authorization::run_tests(
            &mut runner,
            &client,
            rest_url,
            auth_header.as_deref(),
            &args,
        )
        .await;
        rest::authorization_batch::run_tests(
            &mut runner,
            &client,
            rest_url,
            auth_header.as_deref(),
            &args,
        )
        .await;
    }

    if args.test_suite == TestSuite::All || args.test_suite == TestSuite::Grpc {
        let grpc_url = args.grpc_url.as_deref().unwrap();

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("{} gRPC Tests", ROCKET);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();

        grpc::check_permission::run_tests(&mut runner, grpc_url, auth_header.as_deref(), &args)
            .await;
        grpc::check_permission_batch::run_tests(
            &mut runner,
            grpc_url,
            auth_header.as_deref(),
            &args,
        )
        .await;
    }

    runner.print_summary();

    if runner.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
