// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const PASS: &str = "[PASS]";
const FAIL: &str = "[FAIL]";
const INFO: &str = "[INFO]";
const WARN: &str = "[WARN]";

fn redact_source_file_for_log(source_file: &str) -> &str {
    source_file.split('?').next().unwrap_or(source_file)
}

#[derive(Parser, Debug)]
#[command(version, about = "Smoke test for thumbnail-service HTTP API")]
struct Args {
    #[arg(long, env = "SERVICE_URL")]
    service_url: String,

    #[arg(long, env = "AUTH_TOKEN")]
    auth_token: Option<String>,

    #[arg(long, env = "AUTH_TOKEN_FILE")]
    auth_token_file: Option<String>,

    #[arg(long, env = "ENABLE_ACCESS_TOKEN_FILE", default_value_t = false)]
    enable_access_token_file: bool,

    #[arg(long, env = "ACCESS_TOKEN_FILE_PATH")]
    access_token_file_path: Option<String>,

    #[arg(
        long,
        env = "ACCESS_TOKEN_FILE_POLL_INTERVAL",
        default_value = "15s",
        value_parser = humantime::parse_duration
    )]
    access_token_file_poll_interval: Duration,

    #[arg(long, env = "SOURCE_FILE")]
    source_file: String,

    #[arg(long, env = "WAIT_RETRIES", default_value = "10")]
    wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value = "5")]
    wait_interval: u64,

    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "15")]
    request_timeout: u64,
}

#[derive(Debug)]
struct TestError {
    message: String,
}

impl TestError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

struct TestResult {
    name: String,
    kind: TestResultKind,
    duration_ms: u128,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestResultKind {
    Pass,
    Fail,
}

struct TestRunner {
    results: Vec<TestResult>,
}

impl TestRunner {
    fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    fn record_pass(&mut self, name: &str, duration_ms: u128) {
        println!("{} {} ({}ms)", PASS, name, duration_ms);
        self.results.push(TestResult {
            name: name.to_string(),
            kind: TestResultKind::Pass,
            duration_ms,
            error: None,
        });
    }

    fn record_fail(&mut self, name: &str, duration_ms: u128, error: &TestError) {
        println!("{} {} ({}ms)", FAIL, name, duration_ms);
        println!("       {}", error.message);
        self.results.push(TestResult {
            name: name.to_string(),
            kind: TestResultKind::Fail,
            duration_ms,
            error: Some(error.message.clone()),
        });
    }

    fn print_summary(&self) {
        let total = self.results.len();
        let passed = self
            .results
            .iter()
            .filter(|r| r.kind == TestResultKind::Pass)
            .count();
        let failed = self
            .results
            .iter()
            .filter(|r| r.kind == TestResultKind::Fail)
            .count();
        let total_ms: u128 = self.results.iter().map(|r| r.duration_ms).sum();

        println!();
        println!("----------------------------------------");
        for result in &self.results {
            match result.kind {
                TestResultKind::Pass => println!("{} {}", PASS, result.name),
                TestResultKind::Fail => {
                    if let Some(error) = &result.error {
                        println!("{} {} -> {}", FAIL, result.name, error);
                    } else {
                        println!("{} {}", FAIL, result.name);
                    }
                }
            }
        }
        println!("----------------------------------------");
        println!(
            "{} {} passed, {} failed ({} total, {}ms)",
            INFO, passed, failed, total, total_ms
        );
        println!();
    }

    fn all_passed(&self) -> bool {
        !self.results.iter().any(|r| r.kind == TestResultKind::Fail)
    }
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

#[derive(Debug, Deserialize)]
struct AssociateResponse {
    success: bool,
    #[serde(default)]
    error_message: Option<String>,
}

struct SmokeTest {
    client: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
    source_file: String,
    request_timeout: Duration,
}

impl SmokeTest {
    fn new(args: &Args) -> Result<Self, TestError> {
        let source_file = args.source_file.trim();
        if source_file.is_empty() {
            return Err(TestError::new(
                "SOURCE_FILE/--source-file must be set to a non-empty internal fixture path",
            ));
        }

        Ok(Self {
            client: reqwest::Client::new(),
            base_url: args.service_url.trim_end_matches('/').to_string(),
            auth_token: load_auth_token(args)?,
            source_file: source_file.to_string(),
            request_timeout: Duration::from_secs(args.request_timeout),
        })
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.auth_token.as_deref() {
            Some(token) => request.header("Authorization", format!("Bearer {}", token)),
            None => request,
        }
    }

    async fn parse_json<T: DeserializeOwned>(
        &self,
        response: reqwest::Response,
        context: &str,
    ) -> Result<T, TestError> {
        response
            .json::<T>()
            .await
            .map_err(|e| TestError::new(format!("{}: failed to parse JSON: {}", context, e)))
    }

    fn thumbnail_address(&self) -> String {
        format!("{}.smoke-test-thumbnail.png", self.source_file)
    }

    async fn check_health(&self) -> Result<(), TestError> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .with_auth(self.client.get(&url))
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|e| TestError::new(format!("GET /health failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TestError::new(format!(
                "GET /health returned {}: {}",
                status, body
            )));
        }

        let body: HealthResponse = self.parse_json(response, "GET /health").await?;
        let normalized_status = body.status.trim().to_ascii_uppercase();
        if normalized_status != "OK" && normalized_status != "UP" {
            return Err(TestError::new(format!(
                "GET /health returned non-healthy status {:?} (expected OK or UP)",
                body.status
            )));
        }
        Ok(())
    }

    async fn associate(&self) -> Result<(), TestError> {
        let url = format!("{}/v1alpha/associations", self.base_url);
        let payload = json!({
            "target_type": "FILE",
            "thumbnail": {
                "resource_address": self.thumbnail_address(),
                "resource_identity": self.thumbnail_address()
            },
            "association_target": {
                "resource_address": self.source_file,
                "resource_identity": self.source_file
            },
            "specification": {
                "type": "IMAGE_TO_IMAGE",
                "width": 128,
                "height": 128,
                "encoding_format": "PNG",
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER",
                "quality": 85,
                "preserve_aspect_ratio": true
            }
        });

        let response = self
            .with_auth(self.client.post(&url))
            .json(&payload)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|e| TestError::new(format!("POST /v1alpha/associations failed: {}", e)))?;

        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(());
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(TestError::new(format!(
                "POST /v1alpha/associations returned {}: {}",
                status, body
            )));
        }

        let body: AssociateResponse = self
            .parse_json(response, "POST /v1alpha/associations")
            .await?;
        if !body.success {
            return Err(TestError::new(format!(
                "POST /v1alpha/associations returned success=false: {:?}",
                body.error_message
            )));
        }
        Ok(())
    }

    async fn get_thumbnails(&self) -> Result<(), TestError> {
        let url = format!("{}/v1alpha/thumbnails", self.base_url);
        let response = self
            .with_auth(self.client.get(&url))
            .query(&[("association_target_resource_address", &self.source_file)])
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|e| TestError::new(format!("GET /v1alpha/thumbnails failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TestError::new(format!(
                "GET /v1alpha/thumbnails returned {}: {}",
                status, body
            )));
        }

        let body: serde_json::Value = self.parse_json(response, "GET /v1alpha/thumbnails").await?;
        let thumbnails = body
            .get("thumbnails")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                TestError::new("GET /v1alpha/thumbnails response missing 'thumbnails' array")
            })?;

        let thumbnail_address = self.thumbnail_address();
        let found = thumbnails.iter().any(|t| {
            t.get("thumbnail")
                .and_then(|th| th.get("resource_address"))
                .and_then(|v| v.as_str())
                == Some(&thumbnail_address)
        });

        if !found {
            return Err(TestError::new(format!(
                "GET /v1alpha/thumbnails did not return the associated thumbnail {} (got {} entries)",
                thumbnail_address,
                thumbnails.len()
            )));
        }

        Ok(())
    }

    async fn disassociate(&self) -> Result<(), TestError> {
        let url = format!("{}/v1alpha/associations", self.base_url);
        let response = self
            .disassociate_request(&url)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|e| TestError::new(format!("DELETE /v1alpha/associations failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TestError::new(format!(
                "DELETE /v1alpha/associations returned {}: {}",
                status, body
            )));
        }

        Ok(())
    }

    fn disassociate_request(&self, url: &str) -> reqwest::RequestBuilder {
        let thumbnail_address = self.thumbnail_address();
        self.with_auth(self.client.delete(url)).query(&[
            ("target_type", "FILE"),
            (
                "association_target_resource_address",
                self.source_file.as_str(),
            ),
            ("thumbnail_resource_address", thumbnail_address.as_str()),
        ])
    }
}

async fn wait_for_service(
    smoke_test: &SmokeTest,
    retries: u32,
    interval_secs: u64,
) -> Result<(), TestError> {
    if retries == 0 {
        return Err(TestError::new(format!(
            "Invalid configuration: WAIT_RETRIES must be >= 1, got {}",
            retries
        )));
    }

    println!(
        "{} Waiting for service readiness ({} retries, {}s interval)",
        INFO, retries, interval_secs
    );

    for attempt in 1..=retries {
        match smoke_test.check_health().await {
            Ok(()) => {
                println!("{} Service is ready on attempt {}", PASS, attempt);
                return Ok(());
            }
            Err(error) => {
                if attempt == retries {
                    return Err(TestError::new(format!(
                        "Service did not become ready: {}",
                        error.message
                    )));
                }
                println!(
                    "{} Attempt {}/{} failed: {}",
                    WARN, attempt, retries, error.message
                );
                sleep(Duration::from_secs(interval_secs)).await;
            }
        }
    }

    Err(TestError::new("Service did not become ready"))
}

async fn run_smoke_tests(smoke_test: &SmokeTest, _retries: u32, _interval_secs: u64) -> TestRunner {
    let mut runner = TestRunner::new();

    let start = Instant::now();
    match smoke_test.check_health().await {
        Ok(()) => runner.record_pass("Health Check", start.elapsed().as_millis()),
        Err(error) => {
            runner.record_fail("Health Check", start.elapsed().as_millis(), &error);
            return runner;
        }
    }

    let start = Instant::now();
    match smoke_test.associate().await {
        Ok(()) => runner.record_pass("Associate Thumbnail", start.elapsed().as_millis()),
        Err(error) => {
            runner.record_fail("Associate Thumbnail", start.elapsed().as_millis(), &error);
            return runner;
        }
    }

    let start = Instant::now();
    match smoke_test.get_thumbnails().await {
        Ok(()) => runner.record_pass("Get Thumbnails", start.elapsed().as_millis()),
        Err(error) => runner.record_fail("Get Thumbnails", start.elapsed().as_millis(), &error),
    }

    let start = Instant::now();
    match smoke_test.disassociate().await {
        Ok(()) => runner.record_pass("Disassociate Thumbnails", start.elapsed().as_millis()),
        Err(error) => runner.record_fail(
            "Disassociate Thumbnails",
            start.elapsed().as_millis(),
            &error,
        ),
    }

    runner
}

fn load_auth_token(args: &Args) -> Result<Option<String>, TestError> {
    if args.enable_access_token_file
        && args
            .access_token_file_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .is_none()
    {
        return Err(TestError::new(
            "ENABLE_ACCESS_TOKEN_FILE=true but ACCESS_TOKEN_FILE_PATH is not set",
        ));
    }

    if let Some(token_file) = configured_token_file(args) {
        let wait_timeout = Duration::from_secs(args.wait_retries as u64 * args.wait_interval);
        let poll_interval = args
            .access_token_file_poll_interval
            .max(Duration::from_millis(500));

        let started = Instant::now();
        let mut last_error = None;

        while started.elapsed() <= wait_timeout {
            match std::fs::read_to_string(token_file) {
                Ok(contents) => match parse_token_file_contents(token_file, &contents) {
                    Ok(token) if !token.is_empty() => return Ok(Some(token)),
                    Ok(_) => {
                        last_error = Some("token file is empty".to_string());
                    }
                    Err(error) => {
                        last_error = Some(error.message);
                    }
                },
                Err(error) => {
                    last_error = Some(error.to_string());
                }
            }

            std::thread::sleep(poll_interval);
        }

        return Err(TestError::new(format!(
            "Token file at {} was not usable after {}s: {}",
            token_file,
            wait_timeout.as_secs(),
            last_error.unwrap_or_else(|| "unknown error".to_string())
        )));
    }

    Ok(args
        .auth_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string))
}

fn configured_token_file(args: &Args) -> Option<&str> {
    args.auth_token_file
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            if args.enable_access_token_file {
                args.access_token_file_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
            } else {
                None
            }
        })
}

fn parse_token_file_contents(token_file: &str, contents: &str) -> Result<String, TestError> {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    // Sidecar token providers may write JSON envelopes like:
    // {"access_token":"<jwt>","expires_on":"..."}.
    // Fall back to treating file contents as the raw token when JSON is not used.
    if trimmed.starts_with('{') {
        let parsed: serde_json::Value = serde_json::from_str(trimmed).map_err(|error| {
            TestError::new(format!(
                "AUTH_TOKEN_FILE at {} contains invalid JSON: {}",
                token_file, error
            ))
        })?;

        let token = parsed
            .get("access_token")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                TestError::new(format!(
                    "AUTH_TOKEN_FILE at {} is JSON but missing non-empty access_token",
                    token_file
                ))
            })?;

        return Ok(token.to_string());
    }

    Ok(trimmed.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let smoke_test = match SmokeTest::new(&args) {
        Ok(smoke_test) => smoke_test,
        Err(error) => {
            eprintln!("{} {}", FAIL, error.message);
            return ExitCode::FAILURE;
        }
    };

    println!("----------------------------------------");
    println!("Thumbnail Service Smoke Test");
    println!("----------------------------------------");
    println!("Service URL: {}", args.service_url);
    println!(
        "Auth source: {}",
        if configured_token_file(&args).is_some() {
            "token file"
        } else if args
            .auth_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .is_some()
        {
            "inline token"
        } else {
            "none"
        }
    );
    println!(
        "Source file: {}",
        redact_source_file_for_log(&args.source_file)
    );
    println!("Wait retries: {}", args.wait_retries);
    println!("Wait interval: {}s", args.wait_interval);
    println!("Request timeout: {}s", args.request_timeout);
    println!("----------------------------------------");
    println!();

    if let Err(error) = wait_for_service(&smoke_test, args.wait_retries, args.wait_interval).await {
        eprintln!("{} {}", FAIL, error.message);
        return ExitCode::FAILURE;
    }

    let runner = run_smoke_tests(&smoke_test, args.wait_retries, args.wait_interval).await;
    runner.print_summary();

    if runner.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn smoke_test() -> SmokeTest {
        SmokeTest {
            client: reqwest::Client::new(),
            base_url: "http://127.0.0.1:3000".to_string(),
            auth_token: Some("test-token".to_string()),
            source_file: "https://storage.example.test/source.usd".to_string(),
            request_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn disassociate_request_uses_query_parameters_without_a_body() {
        let smoke_test = smoke_test();
        let request = smoke_test
            .disassociate_request("http://127.0.0.1:3000/v1alpha/associations")
            .build()
            .expect("disassociate request must build");
        let query = request
            .url()
            .query_pairs()
            .into_owned()
            .collect::<HashMap<_, _>>();

        assert_eq!(query.get("target_type").map(String::as_str), Some("FILE"));
        assert_eq!(
            query
                .get("association_target_resource_address")
                .map(String::as_str),
            Some(smoke_test.source_file.as_str())
        );
        assert_eq!(
            query.get("thumbnail_resource_address").map(String::as_str),
            Some(smoke_test.thumbnail_address().as_str())
        );
        assert!(request.body().is_none());
    }
}
