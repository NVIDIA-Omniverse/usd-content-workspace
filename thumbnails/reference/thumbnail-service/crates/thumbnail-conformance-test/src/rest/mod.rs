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

//! REST suite: exercises `/v1alpha/associations`, `/v1alpha/thumbnails`
//! (both the list-read and CreateThumbnails paths), and `/health*`.

pub(crate) mod associations;
pub(crate) mod thumbnails;

use crate::contract::{VerifiedBound, OPENAPI_PATH};
#[cfg(test)]
pub(crate) use associations::ASSOCIATION_REJECTIONS;
use std::time::Duration;
#[cfg(test)]
pub(crate) use thumbnails::CREATE_CASES;

use serde::Deserialize;
use tokio::time::sleep;

use crate::style::{ConfigurationFault, TestError, TestRunner, INFO, PASS, WARN};

/// Shared REST client and small response-shape helpers used by both
/// submodules.
pub struct RestClient {
    pub client: reqwest::Client,
    pub base_url: String,
    pub token: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

impl RestClient {
    pub fn new(base_url: &str, token: Option<String>, timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            timeout,
        }
    }

    pub fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.token.as_deref() {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }

    /// GET a health path and confirm both the status code and the documented
    /// `{"status": "..."}` body shape.
    pub async fn check_health(&self, path: &str) -> Result<(), TestError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .with_auth(self.client.get(&url))
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| classify_request_error(&e, &format!("GET {path} failed")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| classify_request_error(&error, "read response body"))?;
            return Err(classify_unexpected_status(
                status,
                &format!("GET {path}"),
                &body,
            ));
        }

        let body: HealthResponse = response.json().await.map_err(|e| {
            classify_request_error(&e, &format!("GET {path}: failed to parse JSON"))
        })?;
        if body.status.trim().is_empty() {
            return Err(TestError::new(format!(
                "GET {path} returned an empty 'status' field"
            )));
        }
        Ok(())
    }
}

/// Poll `ready_path` until it succeeds or the retry budget is exhausted.
pub async fn wait_for_ready(
    rest: &RestClient,
    ready_path: &str,
    retries: u32,
    interval: Duration,
) -> Result<(), TestError> {
    if retries == 0 {
        return Err(TestError::new(format!(
            "invalid configuration: WAIT_RETRIES must be >= 1, got {retries}"
        )));
    }

    println!(
        "{INFO} Waiting for {ready_path} readiness ({retries} retries, {} interval)",
        humantime::format_duration(interval)
    );

    for attempt in 1..=retries {
        match rest.check_health(ready_path).await {
            Ok(()) => {
                println!("{PASS} Service is ready on attempt {attempt}");
                return Ok(());
            }
            Err(mut error) => {
                if attempt == retries {
                    error.message = format!(
                        "service did not become ready; run incomplete: {}",
                        error.message
                    );
                    return Err(error);
                }
                println!("{WARN} Attempt {attempt}/{retries} failed: {error}");
                sleep(interval).await;
            }
        }
    }

    Err(TestError::new("service did not become ready"))
}

/// Classify a status the case did not expect, so the run's own faults are not
/// reported as the deployment's.
///
/// `401` identifies missing/rejected credentials; `403` identifies denied access.
/// Neither certifies the contract. Preserve the distinction so source permissions
/// are not reported as an invalid token.
///
/// Everything else is the deployment's answer and stays a contract failure.
/// Raised by Mitri on !221 as finding 10.
pub(crate) fn classify_unexpected_status(
    status: reqwest::StatusCode,
    context: &str,
    body: &str,
) -> TestError {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => TestError::configuration(
            ConfigurationFault::RejectedCredentials,
            format!("{context} returned {status}, so the run never reached the contract: {body}"),
        ),
        reqwest::StatusCode::FORBIDDEN => TestError::configuration(
            ConfigurationFault::AccessDenied,
            format!("{context} returned {status}: {body}"),
        ),
        reqwest::StatusCode::NOT_FOUND => TestError::configuration(
            ConfigurationFault::EndpointMismatch,
            format!("{context} returned 404; check --rest-url and the API route: {body}"),
        ),
        _ => TestError::new(format!("{context} returned {status}: {body}")),
    }
}

/// Tell a source that does not resolve apart from an endpoint that is not
/// serving this API, both of which answer `404`.
///
/// The discriminator is the documented body shape, not its prose: this API's
/// `404` carries an `ErrorResponse`, and a router with no such route answers
/// with something else (or nothing). Attributing every `404` to `--source-file`
/// meant a wrong `--rest-url`, or a deployment not implementing the route, was
/// reported as a missing object — sending the operator to check storage for an
/// object that was never the problem. Raised by Mitri on !221 as finding 11.
///
/// The readiness probe catches a base URL wrong enough to break `--ready-path`,
/// so what is left for this is the narrower case: an endpoint that serves health
/// and not the route under test.
pub(crate) fn classify_not_found(context: &str, body: &str) -> TestError {
    let carries_error_response = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error_message")
                .and_then(|m| m.as_str())
                .map(|m| !m.trim().is_empty())
        })
        .unwrap_or(false);

    if carries_error_response {
        TestError::unresolvable_source(format!(
            "{context} returned 404 with the documented ErrorResponse, so the route exists and \
             the object does not: --source-file must name an object that resolves in storage. \
             Body: {body}"
        ))
    } else {
        TestError::configuration(
            ConfigurationFault::EndpointMismatch,
            format!(
                "{context} returned 404 without this API's ErrorResponse body, so the route \
                 itself is missing rather than the object: check --rest-url and that the \
                 deployment serves this API version. Body: {body}"
            ),
        )
    }
}

/// Run every REST case against `rest` for the given fixture source file.
pub async fn run(
    runner: &mut TestRunner,
    rest: &RestClient,
    source_file: &str,
    bound: VerifiedBound,
) {
    associations::run(runner, rest, source_file).await;
    thumbnails::run(runner, rest, source_file, bound).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_the_runs_fault_and_other_statuses_are_the_deployments() {
        for (status, fault) in [
            (
                reqwest::StatusCode::UNAUTHORIZED,
                ConfigurationFault::RejectedCredentials,
            ),
            (
                reqwest::StatusCode::FORBIDDEN,
                ConfigurationFault::AccessDenied,
            ),
        ] {
            assert_eq!(
                classify_unexpected_status(status, "ctx", "body").configuration_fault,
                Some(fault),
                "{status} must retain the distinction between credentials and access"
            );
        }

        for status in [
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::BAD_REQUEST,
            reqwest::StatusCode::OK,
            reqwest::StatusCode::BAD_GATEWAY,
        ] {
            assert_eq!(
                classify_unexpected_status(status, "ctx", "body").configuration_fault,
                None,
                "{status} is the deployment's answer and must stay a contract failure"
            );
        }
    }

    /// The whole point of the split: both are `404`, and blaming the wrong one
    /// sends an operator to check storage for an object that was never missing.
    #[test]
    fn a_404_is_the_source_only_when_it_carries_the_documented_error_body() {
        let with_body = classify_not_found(
            "ctx",
            r#"{"source":"thumbnail-service","error_message":"resource_address was not found"}"#,
        );
        assert_eq!(
            with_body.configuration_fault,
            Some(ConfigurationFault::UnresolvableSource)
        );

        // A router with no such route: axum's own 404, and the empty-body case.
        for body in ["", "Not Found", "<html>404</html>", "{}"] {
            assert_eq!(
                classify_not_found("ctx", body).configuration_fault,
                Some(ConfigurationFault::EndpointMismatch),
                "a 404 without this API's ErrorResponse is the route, not the object: {body:?}"
            );
        }
    }

    /// An `ErrorResponse` whose `error_message` is present but blank is not
    /// evidence the route exists, and must not be read as one.
    #[test]
    fn a_blank_error_message_does_not_count_as_the_documented_body() {
        assert_eq!(
            classify_not_found("ctx", r#"{"error_message":"   "}"#).configuration_fault,
            Some(ConfigurationFault::EndpointMismatch)
        );
    }
}

/// Recognize locally observed reqwest timeouts, including response-body reads.
pub(crate) fn classify_request_error(error: &reqwest::Error, context: &str) -> TestError {
    if error.is_timeout() {
        TestError::configuration(
            ConfigurationFault::ClientTimeout,
            format!("{context}: incomplete observation after --http-timeout: {error}"),
        )
    } else {
        TestError::new(format!("{context}: {error}"))
    }
}

/// Verify the served bound before probing, retaining the cause of an unavailable schema.
pub async fn advertised_bound(rest: &RestClient) -> Result<VerifiedBound, TestError> {
    let response = rest
        .with_auth(rest.client.get(format!("{}{OPENAPI_PATH}", rest.base_url)))
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(openapi_request_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.map_err(openapi_request_error)?;
        return Err(classify_unexpected_status(
            status,
            &format!("unverifiable contract: GET {OPENAPI_PATH} (--rest-url)"),
            &body,
        ));
    }
    let document: serde_json::Value = response.json().await.map_err(openapi_request_error)?;
    crate::contract::verify_advertised_bound(&document)
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use clap::Parser;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn openapi_failures_preserve_status_classification_and_operator_advice() {
        for (status, body, fault, advice) in [
            (
                401,
                "unauthenticated",
                Some(ConfigurationFault::RejectedCredentials),
                "--bearer-token",
            ),
            (
                403,
                "forbidden",
                Some(ConfigurationFault::AccessDenied),
                "caller permissions",
            ),
            (
                404,
                "missing",
                Some(ConfigurationFault::EndpointMismatch),
                "--rest-url",
            ),
            (500, "server fault", None, "500"),
            (503, "unavailable", None, "503"),
            (
                200,
                "not json",
                Some(ConfigurationFault::UnverifiableContract),
                "advertised contract could not be verified",
            ),
            (
                200,
                "{}",
                Some(ConfigurationFault::UnverifiableContract),
                "advertised contract could not be verified",
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(OPENAPI_PATH))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&server)
                .await;
            let client = RestClient::new(&server.uri(), None, Duration::from_secs(1));
            let error = advertised_bound(&client).await.unwrap_err();
            assert_eq!(error.configuration_fault, fault, "HTTP {status}: {error}");
            assert!(error.message.contains("unverifiable") && error.message.contains(OPENAPI_PATH));
            let mut runner = TestRunner::new();
            runner.record_fail(crate::style::Suite::Grpc, "OpenAPI preflight", 0, &error);
            let summary = runner.summary_lines().join("\n");
            assert!(summary.contains(advice), "{summary}");
            if fault.is_none() {
                assert!(!summary.contains("misconfigured run"), "{summary}");
            }
            assert!(!runner.all_passed());
        }
    }

    #[tokio::test]
    async fn failed_preflight_never_sends_a_create_probe_including_grpc_only() {
        for (status, body) in [
            (401, "unauthenticated".to_string()),
            (403, "forbidden".to_string()),
            (404, "missing".to_string()),
            (503, "unavailable".to_string()),
            (200, "not json".to_string()),
            (200, "{}".to_string()),
            (
                200,
                crate::contract::document_with_bound(999.into()).to_string(),
            ),
            (
                200,
                crate::contract::document_with_bound(u64::MAX.into()).to_string(),
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(OPENAPI_PATH))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            let args = crate::config::Args::parse_from([
                "conformance",
                "--test-suite",
                "grpc",
                "--rest-url",
                &server.uri(),
                "--grpc-url",
                "http://127.0.0.1:1",
                "--source-file",
                "source",
            ]);
            assert!(!crate::run(&args).await.unwrap());
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests.len(),
                1,
                "failed preflight must send no generation requests"
            );
            assert_eq!(requests[0].url.path(), OPENAPI_PATH);
        }
    }

    #[tokio::test]
    async fn rest_readiness_waits_for_delayed_health_and_reports_exhaustion() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let server = MockServer::start().await;
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        Mock::given(path("/health"))
            .respond_with(move |_: &wiremock::Request| {
                if observed.fetch_add(1, Ordering::SeqCst) < 2 {
                    ResponseTemplate::new(503)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"OK"}))
                }
            })
            .mount(&server)
            .await;
        let client = RestClient::new(&server.uri(), None, Duration::from_secs(1));
        wait_for_ready(&client, "/health", 3, Duration::from_millis(1))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        let error = wait_for_ready(&client, "/absent", 2, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.message.contains("did not become ready"));
    }
}

fn openapi_request_error(error: reqwest::Error) -> TestError {
    if error.is_timeout() {
        classify_request_error(
            &error,
            &format!("unverifiable contract at {OPENAPI_PATH} (--rest-url)"),
        )
    } else {
        crate::contract::unverifiable(error)
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};
    #[tokio::test]
    async fn readiness_and_openapi_retain_known_client_timeout_diagnostics() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
            .mount(&server)
            .await;
        let client = RestClient::new(&server.uri(), None, Duration::from_millis(20));
        let error = wait_for_ready(&client, "/health", 1, Duration::ZERO)
            .await
            .unwrap_err();
        assert_eq!(
            error.configuration_fault,
            Some(ConfigurationFault::ClientTimeout)
        );
        assert!(
            error.message.contains("--http-timeout")
                && error.message.contains("run incomplete")
                && error.message.contains("/health")
        );
        let error = advertised_bound(&client).await.unwrap_err();
        assert_eq!(
            error.configuration_fault,
            Some(ConfigurationFault::ClientTimeout)
        );
        assert!(
            error.message.contains("--http-timeout")
                && error.message.contains(OPENAPI_PATH)
                && error.message.contains("unverifiable")
        );
    }
}
