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

fn build_batch_payload(args: &Args, condition: Option<&str>) -> Value {
    let allow_action = json!({ "name": args.action_name, "service": args.action_service });
    let deny_action = json!({ "name": DENY_ACTION_NAME, "service": DENY_ACTION_SERVICE });

    let mut payload = json!({
        "batches": [{
            "principal": { "sub": args.principal },
            "actions": [allow_action, deny_action],
            "resource": { "id": args.resource_id, "type": args.resource_type }
        }]
    });

    if let Some(cond) = condition {
        payload["condition"] = json!(cond);
    }

    payload
}

async fn post_batch(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    payload: &Value,
) -> Result<(reqwest::StatusCode, Value), TestError> {
    let url = format!("{}/v1beta/authorization/batch/", rest_url);
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

fn validate_batch_response_structure(body: &Value) -> Result<&Vec<Value>, TestError> {
    let obj = body
        .as_object()
        .ok_or_else(|| TestError::new("Response is not a JSON object", ""))?;

    if !obj.contains_key("decisions") {
        return Err(TestError::new(
            "Missing required field 'decisions' in batch response",
            format!("Response keys: {:?}", obj.keys().collect::<Vec<_>>()),
        ));
    }

    let known_fields = ["decisions", "summary"];
    for key in obj.keys() {
        if !known_fields.contains(&key.as_str()) {
            return Err(TestError::new(
                format!("Unexpected field '{}' in batch response", key),
                format!("Expected fields: {:?}", known_fields),
            ));
        }
    }

    body["decisions"]
        .as_array()
        .ok_or_else(|| TestError::new("'decisions' is not an array", ""))
}

/// Extracts the decision string from a decisions-array item keyed by `service:action`.
fn extract_decision_from_item(item: &Value, key: &str) -> Result<String, TestError> {
    let entry = item.get(key).ok_or_else(|| {
        TestError::new(
            format!("Decision item missing expected key '{}'", key),
            format!(
                "Item keys: {:?}",
                item.as_object().map(|o| o.keys().collect::<Vec<_>>())
            ),
        )
    })?;

    let decision = entry
        .get("decision")
        .and_then(|d| d.as_str())
        .ok_or_else(|| {
            TestError::new(
                format!("Decision entry for '{}' missing 'decision' field", key),
                format!("Entry: {}", entry),
            )
        })?;

    Ok(decision.to_string())
}

fn allow_action_key(args: &Args) -> String {
    format!("{}:{}", args.action_service, args.action_name)
}

fn deny_action_key() -> String {
    format!("{}:{}", DENY_ACTION_SERVICE, DENY_ACTION_NAME)
}

// ── condition: "and" ────────────────────────────────────────────────────────
// First action is allow, second is deny.
// With AND short-circuit: first evaluates to allow, second evaluates to deny,
// summary should be deny. Actions after the first deny have decision "skip".
async fn test_condition_and(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - condition: and";

    let payload = build_batch_payload(args, Some("and"));
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 1) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 0, &deny_key, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary(&body, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── condition: "or" ─────────────────────────────────────────────────────────
// First action is allow, second is deny.
// With OR short-circuit: first evaluates to allow → summary is allow,
// remaining actions get "skip".
async fn test_condition_or(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - condition: or";

    let payload = build_batch_payload(args, Some("or"));
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 1) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 0, &deny_key, "skip") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary(&body, "allow") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── condition: none/null ────────────────────────────────────────────────────
// No condition → all actions evaluated individually, no summary.
async fn test_condition_none(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - condition: none";

    let payload = build_batch_payload(args, None);
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 1) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 0, &deny_key, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    // No summary should be present when condition is unset
    if body.get("summary").is_some() && !body["summary"].is_null() {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected no 'summary' when condition is not set, but summary was present",
                format!("summary: {}", body["summary"]),
            )
            .with_request(&request_str)
            .with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

struct DecisionError {
    message: String,
    hint: String,
}

impl DecisionError {
    fn with_context(self, args: &Args) -> TestError {
        if self.hint.is_empty() {
            TestError::unexpected_decision("allow", &self.message, args)
        } else {
            TestError::new(self.message, self.hint)
        }
    }
}

impl From<DecisionError> for TestError {
    fn from(e: DecisionError) -> Self {
        TestError::new(e.message, e.hint)
    }
}

fn verify_decisions_count(decisions: &[Value], expected: usize) -> Result<(), TestError> {
    if decisions.len() != expected {
        return Err(TestError::new(
            format!(
                "Expected {} item(s) in 'decisions' array but got {}",
                expected,
                decisions.len()
            ),
            "",
        ));
    }
    Ok(())
}

fn verify_decision_item(
    decisions: &[Value],
    index: usize,
    expected_key: &str,
    expected_decision: &str,
) -> Result<(), DecisionError> {
    let item = decisions.get(index).ok_or_else(|| DecisionError {
        message: format!("'decisions' array has no item at index {}", index),
        hint: format!("Expected at least {} item(s)", index + 1),
    })?;

    let decision = extract_decision_from_item(item, expected_key).map_err(|e| DecisionError {
        message: e.message,
        hint: e.hint,
    })?;

    if decision != expected_decision {
        return Err(DecisionError {
            message: format!(
                "Expected decision '{}' for '{}' but got '{}'",
                expected_decision, expected_key, decision
            ),
            hint: String::new(),
        });
    }

    Ok(())
}

fn verify_summary(body: &Value, expected_decision: &str) -> Result<(), TestError> {
    let summary = body.get("summary").ok_or_else(|| {
        TestError::new(
            "Missing 'summary' in batch response (required when condition is set)",
            "",
        )
    })?;

    if summary.is_null() {
        return Err(TestError::new(
            "Expected 'summary' to be present when condition is set, but it was null",
            "",
        ));
    }

    let decision = summary
        .get("decision")
        .and_then(|d| d.as_str())
        .ok_or_else(|| {
            TestError::new(
                "Summary is missing 'decision' field",
                format!("Summary: {}", summary),
            )
        })?;

    if decision != expected_decision {
        return Err(TestError::new(
            format!(
                "Expected summary decision '{}' but got '{}'",
                expected_decision, decision
            ),
            "",
        ));
    }

    Ok(())
}

fn build_multi_batch_payload(args: &Args, condition: Option<&str>) -> Value {
    let allow_action = json!({ "name": args.action_name, "service": args.action_service });
    let deny_action = json!({ "name": DENY_ACTION_NAME, "service": DENY_ACTION_SERVICE });

    let mut payload = json!({
        "batches": [
            {
                "principal": { "sub": args.principal },
                "actions": [allow_action],
                "resource": { "id": args.resource_id, "type": args.resource_type }
            },
            {
                "principal": { "sub": args.principal },
                "actions": [deny_action],
                "resource": { "id": args.resource_id, "type": args.resource_type }
            }
        ]
    });

    if let Some(cond) = condition {
        payload["condition"] = json!(cond);
    }

    payload
}

// ── Multi-batch condition: "and" ────────────────────────────────────────────
// Two batch items: batch 0 has allow action, batch 1 has deny action.
// AND: allow → continue, deny → summary=deny. Each batch yields one map entry.
async fn test_multi_batch_condition_and(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - multi-batch, condition: and";

    let payload = build_multi_batch_payload(args, Some("and"));
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 1, &deny_key, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary(&body, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── Multi-batch condition: "or" ─────────────────────────────────────────────
// Two batch items: batch 0 has allow action, batch 1 has deny action.
// OR: allow → short-circuit, batch 1 actions skipped, summary=allow.
async fn test_multi_batch_condition_or(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - multi-batch, condition: or";

    let payload = build_multi_batch_payload(args, Some("or"));
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 1, &deny_key, "skip") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary(&body, "allow") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── Multi-batch condition: none ─────────────────────────────────────────────
// Two batch items: batch 0 has allow action, batch 1 has deny action.
// No condition: both evaluated individually, no summary.
async fn test_multi_batch_condition_none(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "POST /v1beta/authorization/batch/ - multi-batch, condition: none";

    let payload = build_multi_batch_payload(args, None);
    let result = post_batch(client, rest_url, auth_header, &payload).await;

    let (status, body) = match result {
        Ok(tuple) => tuple,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let request_str = format!(
        "POST {}/v1beta/authorization/batch/\n{}",
        rest_url,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let response_str = format!(
        "HTTP {}\n{}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    let decisions = match validate_batch_response_structure(&body) {
        Ok(d) => d,
        Err(e) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(&request_str).with_response(&response_str),
            );
            return;
        }
    };

    if let Err(e) = verify_decisions_count(decisions, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    let allow_key = allow_action_key(args);
    let deny_key = deny_action_key();

    if let Err(e) = verify_decision_item(decisions, 0, &allow_key, "allow") {
        let e = e
            .with_context(args)
            .with_request(&request_str)
            .with_response(&response_str);
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }

    if let Err(e) = verify_decision_item(decisions, 1, &deny_key, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::from(e)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    if body.get("summary").is_some() && !body["summary"].is_null() {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected no 'summary' when condition is not set, but summary was present",
                format!("summary: {}", body["summary"]),
            )
            .with_request(&request_str)
            .with_response(&response_str),
        );
        return;
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
    test_condition_and(runner, client, rest_url, auth_header, args).await;
    test_condition_or(runner, client, rest_url, auth_header, args).await;
    test_condition_none(runner, client, rest_url, auth_header, args).await;
    test_multi_batch_condition_and(runner, client, rest_url, auth_header, args).await;
    test_multi_batch_condition_or(runner, client, rest_url, auth_header, args).await;
    test_multi_batch_condition_none(runner, client, rest_url, auth_header, args).await;
}
