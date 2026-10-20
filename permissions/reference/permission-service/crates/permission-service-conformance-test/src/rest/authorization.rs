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

use crate::config::Args;
use crate::{TestError, TestRunner, DENY_ACTION_NAME, DENY_ACTION_SERVICE};
use serde_json::{json, Value};
use std::time::Instant;

fn build_allow_payload(args: &Args) -> Value {
    json!({
        "principal": { "sub": args.principal },
        "action": { "name": args.action_name, "service": args.action_service },
        "resource": { "id": args.resource_id, "type": args.resource_type }
    })
}

fn build_deny_payload(args: &Args) -> Value {
    json!({
        "principal": { "sub": args.principal },
        "action": { "name": DENY_ACTION_NAME, "service": DENY_ACTION_SERVICE },
        "resource": { "id": args.resource_id, "type": args.resource_type }
    })
}

async fn post_authorization(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    payload: &Value,
) -> Result<(reqwest::StatusCode, Value), TestError> {
    let url = format!("{}/v1beta/authorization/", rest_url);
    let request_str = format!(
        "POST {}\n{}",
        url,
        serde_json::to_string_pretty(payload).unwrap_or_default()
    );
    let mut request = client.post(&url).json(payload);
    if let Some(auth) = auth_header {
        request = request.header("Authorization", auth);
    }

    let response = request
        .send()
        .await
        .map_err(|e| TestError::from_reqwest_error(&e, rest_url).with_request(&request_str))?;

    let status = response.status();
    let body_text = response.text().await.map_err(|e| {
        TestError::new(format!("Failed to read response body: {}", e), "")
            .with_request(&request_str)
            .with_response(format!("HTTP {}", status))
    })?;

    if !status.is_success() {
        return Err(TestError::from_status(status, &body_text, rest_url)
            .with_request(&request_str)
            .with_response(format!("HTTP {}\n{}", status, body_text)));
    }

    let body: Value = serde_json::from_str(&body_text).map_err(|e| {
        TestError::new(
            format!("Response is not valid JSON: {}", e),
            format!("Raw body: {}", &body_text[..body_text.len().min(256)]),
        )
        .with_request(&request_str)
        .with_response(format!("HTTP {}\n{}", status, body_text))
    })?;

    Ok((status, body))
}

const REQUIRED_FIELDS: &[&str] = &["decision"];
const OPTIONAL_FIELDS: &[&str] = &["reason", "action", "service"];

fn validate_response_fields(body: &Value) -> Result<(), TestError> {
    let obj = body
        .as_object()
        .ok_or_else(|| TestError::new("Response is not a JSON object", ""))?;

    for &field in REQUIRED_FIELDS {
        if !obj.contains_key(field) {
            return Err(TestError::new(
                format!("Missing required field '{}' in response", field),
                format!("Response keys: {:?}", obj.keys().collect::<Vec<_>>()),
            ));
        }
    }

    let known: Vec<&str> = REQUIRED_FIELDS
        .iter()
        .chain(OPTIONAL_FIELDS.iter())
        .copied()
        .collect();
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            return Err(TestError::new(
                format!("Unexpected field '{}' in response", key),
                format!("Expected fields: {:?}", known),
            ));
        }
    }

    Ok(())
}

async fn test_allow(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/ - allow";

    let payload = build_allow_payload(args);
    let result = post_authorization(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    if let Err(e) = validate_response_fields(&body) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let decision = body["decision"].as_str().unwrap_or("");
    if decision != "allow" {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::unexpected_decision("allow", decision, args)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if let Some(action_val) = body.get("action").and_then(|v| v.as_str()) {
        if action_val != args.action_name {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Response 'action' mismatch: expected '{}', got '{}'",
                        args.action_name, action_val
                    ),
                    "",
                )
                .with_request(&request_str)
                .with_response(&response_str),
            );
            return;
        }
    }

    if let Some(service_val) = body.get("service").and_then(|v| v.as_str()) {
        if service_val != args.action_service {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Response 'service' mismatch: expected '{}', got '{}'",
                        args.action_service, service_val
                    ),
                    "",
                )
                .with_request(&request_str)
                .with_response(&response_str),
            );
            return;
        }
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_deny(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/ - deny";

    let payload = build_deny_payload(args);
    let result = post_authorization(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    if let Err(e) = validate_response_fields(&body) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let decision = body["decision"].as_str().unwrap_or("");
    if decision != "deny" {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected 'deny' but got '{}'", decision),
                format!(
                    "The deny test uses a non-existent action '{}:{}' which should never be allowed",
                    DENY_ACTION_SERVICE, DENY_ACTION_NAME
                ),
            )
            .with_request(&request_str)
            .with_response(&response_str),
        );
        return;
    }

    if let Some(action_val) = body.get("action").and_then(|v| v.as_str()) {
        if action_val != DENY_ACTION_NAME {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Response 'action' mismatch: expected '{}', got '{}'",
                        DENY_ACTION_NAME, action_val
                    ),
                    "",
                )
                .with_request(&request_str)
                .with_response(&response_str),
            );
            return;
        }
    }

    if let Some(service_val) = body.get("service").and_then(|v| v.as_str()) {
        if service_val != DENY_ACTION_SERVICE {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Response 'service' mismatch: expected '{}', got '{}'",
                        DENY_ACTION_SERVICE, service_val
                    ),
                    "",
                )
                .with_request(&request_str)
                .with_response(&response_str),
            );
            return;
        }
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

pub async fn run_tests(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    test_allow(runner, client, rest_url, auth_header, args).await;
    test_deny(runner, client, rest_url, auth_header, args).await;
}
