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

use super::check_permission::{attach_auth, connect, decision_name};
use super::proto::{
    permission_service_client::PermissionServiceClient, Action, CheckPermissionBatch,
    CheckPermissionBatchRequest, CheckPermissionBatchResponse, Condition, Decision, Principal,
    Resource,
};
use crate::config::Args;
use crate::{TestError, TestRunner, DENY_ACTION_NAME, DENY_ACTION_SERVICE, HINT};
use std::time::Instant;
use tonic::transport::Channel;
use tonic::Request;

fn build_batch_request(
    args: &Args,
    condition: Option<Condition>,
    auth_header: Option<&str>,
) -> Request<CheckPermissionBatchRequest> {
    let allow_action = Action {
        name: args.action_name.clone(),
        service: args.action_service.clone(),
    };
    let deny_action = Action {
        name: DENY_ACTION_NAME.to_string(),
        service: DENY_ACTION_SERVICE.to_string(),
    };

    let batch = CheckPermissionBatch {
        principal: Some(Principal {
            sub: args.principal.clone(),
            info: None,
        }),
        actions: vec![allow_action, deny_action],
        resource: Some(Resource {
            id: args.resource_id.clone(),
            r#type: args.resource_type.clone(),
            data: None,
        }),
        context: None,
    };

    let msg = CheckPermissionBatchRequest {
        condition: condition.map(|c| c as i32),
        batches: vec![batch],
    };

    attach_auth(Request::new(msg), auth_header)
}

async fn call_batch(
    client: &mut PermissionServiceClient<Channel>,
    request: Request<CheckPermissionBatchRequest>,
    test_name: &str,
    request_str: &str,
) -> Result<CheckPermissionBatchResponse, TestError> {
    client
        .check_permission_batch(request)
        .await
        .map(|resp| resp.into_inner())
        .map_err(|status| {
            TestError::new(
                format!(
                    "{}: gRPC call failed: {} ({})",
                    test_name,
                    status.message(),
                    status.code()
                ),
                format!("{}   Check that the gRPC service is running", HINT),
            )
            .with_request(request_str)
            .with_response(format!(
                "gRPC error: {} ({})",
                status.message(),
                status.code()
            ))
        })
}

fn verify_decisions_structure(response: &CheckPermissionBatchResponse) -> Result<(), TestError> {
    if response.decisions.is_empty() {
        return Err(TestError::new("'decisions' is empty in batch response", ""));
    }

    let batch = &response.decisions[0];
    if batch.results.is_empty() {
        return Err(TestError::new("First batch has no results", ""));
    }

    Ok(())
}

fn verify_result(
    response: &CheckPermissionBatchResponse,
    index: usize,
    expected_action: &str,
    expected_service: &str,
    expected_decision: &str,
) -> Result<(), TestError> {
    let results = &response.decisions[0].results;
    let result = results.get(index).ok_or_else(|| {
        TestError::new(
            format!("Results array has no item at index {}", index),
            format!("Expected at least {} result(s)", index + 1),
        )
    })?;

    if result.action != expected_action || result.service != expected_service {
        return Err(TestError::new(
            format!(
                "Result[{}]: expected action '{}:{}' but got '{}:{}'",
                index, expected_service, expected_action, result.service, result.action
            ),
            "",
        ));
    }

    let got = decision_name(result.decision);
    if got != expected_decision {
        return Err(TestError::new(
            format!(
                "Result[{}] '{}:{}': expected decision '{}' but got '{}'",
                index, expected_service, expected_action, expected_decision, got
            ),
            "",
        ));
    }

    Ok(())
}

fn verify_summary_decision(
    response: &CheckPermissionBatchResponse,
    expected: &str,
) -> Result<(), TestError> {
    let summary = response.summary.as_ref().ok_or_else(|| {
        TestError::new(
            "Missing 'summary' in batch response (required when condition is set)",
            "",
        )
    })?;

    let got = decision_name(summary.decision);
    if got != expected {
        return Err(TestError::new(
            format!("Expected summary decision '{}' but got '{}'", expected, got),
            "",
        ));
    }

    Ok(())
}

fn format_grpc_batch_request(args: &Args, condition: Option<&str>) -> String {
    format!(
        "gRPC CheckPermissionBatch\n  condition: {}\n  principal: \"{}\"\n  actions: [{}:{}, {}:{}]\n  resource: {}::\"{}\"",
        condition.unwrap_or("unspecified"),
        args.principal,
        args.action_service, args.action_name,
        DENY_ACTION_SERVICE, DENY_ACTION_NAME,
        args.resource_type, args.resource_id
    )
}

// ── CONDITION_AND ───────────────────────────────────────────────────────────
// Actions: [allow, deny]. With AND: first=allow, second=deny, summary=deny.
async fn test_condition_and(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - condition: and";

    let request_str = format_grpc_batch_request(args, Some("and"));
    let request = build_batch_request(args, Some(Condition::And), auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_decisions_structure(&response) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(
        &response,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(&response, 1, DENY_ACTION_NAME, DENY_ACTION_SERVICE, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary_decision(&response, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── CONDITION_OR ────────────────────────────────────────────────────────────
// Actions: [allow, deny]. With OR: first=allow → short-circuit, second=skip, summary=allow.
async fn test_condition_or(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - condition: or";

    let request_str = format_grpc_batch_request(args, Some("or"));
    let request = build_batch_request(args, Some(Condition::Or), auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_decisions_structure(&response) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(
        &response,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(&response, 1, DENY_ACTION_NAME, DENY_ACTION_SERVICE, "skip") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary_decision(&response, "allow") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── CONDITION_UNSPECIFIED ───────────────────────────────────────────────────
// No condition → all actions evaluated individually, no summary.
async fn test_condition_unspecified(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - condition: unspecified";

    let request_str = format_grpc_batch_request(args, None);
    let request = build_batch_request(args, None, auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_decisions_structure(&response) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(
        &response,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_result(&response, 1, DENY_ACTION_NAME, DENY_ACTION_SERVICE, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if response.summary.is_some() {
        let summary = response.summary.as_ref().unwrap();
        if summary.decision != Decision::Unspecified as i32 {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    "Expected no summary when condition is unspecified, but summary was present with a non-default decision",
                    format!("Summary decision: {}", decision_name(summary.decision)),
                )
                .with_request(&request_str)
                .with_response(&response_str),
            );
            return;
        }
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

fn decision_related(e: &TestError) -> bool {
    e.message.contains("expected decision")
}

fn build_multi_batch_request(
    args: &Args,
    condition: Option<Condition>,
    auth_header: Option<&str>,
) -> Request<CheckPermissionBatchRequest> {
    let allow_action = Action {
        name: args.action_name.clone(),
        service: args.action_service.clone(),
    };
    let deny_action = Action {
        name: DENY_ACTION_NAME.to_string(),
        service: DENY_ACTION_SERVICE.to_string(),
    };

    let batch_allow = CheckPermissionBatch {
        principal: Some(Principal {
            sub: args.principal.clone(),
            info: None,
        }),
        actions: vec![allow_action],
        resource: Some(Resource {
            id: args.resource_id.clone(),
            r#type: args.resource_type.clone(),
            data: None,
        }),
        context: None,
    };

    let batch_deny = CheckPermissionBatch {
        principal: Some(Principal {
            sub: args.principal.clone(),
            info: None,
        }),
        actions: vec![deny_action],
        resource: Some(Resource {
            id: args.resource_id.clone(),
            r#type: args.resource_type.clone(),
            data: None,
        }),
        context: None,
    };

    let msg = CheckPermissionBatchRequest {
        condition: condition.map(|c| c as i32),
        batches: vec![batch_allow, batch_deny],
    };

    attach_auth(Request::new(msg), auth_header)
}

fn verify_multi_batch_structure(
    response: &CheckPermissionBatchResponse,
    expected_batches: usize,
) -> Result<(), TestError> {
    if response.decisions.len() != expected_batches {
        return Err(TestError::new(
            format!(
                "Expected {} batch(es) in 'decisions' but got {}",
                expected_batches,
                response.decisions.len()
            ),
            "",
        ));
    }
    for (i, batch) in response.decisions.iter().enumerate() {
        if batch.results.is_empty() {
            return Err(TestError::new(format!("Batch {} has no results", i), ""));
        }
    }
    Ok(())
}

fn verify_multi_batch_result(
    response: &CheckPermissionBatchResponse,
    batch_index: usize,
    result_index: usize,
    expected_action: &str,
    expected_service: &str,
    expected_decision: &str,
) -> Result<(), TestError> {
    let batch = response.decisions.get(batch_index).ok_or_else(|| {
        TestError::new(
            format!("No batch at index {}", batch_index),
            format!("Expected at least {} batch(es)", batch_index + 1),
        )
    })?;
    let result = batch.results.get(result_index).ok_or_else(|| {
        TestError::new(
            format!(
                "Batch[{}] has no result at index {}",
                batch_index, result_index
            ),
            format!("Expected at least {} result(s)", result_index + 1),
        )
    })?;

    if result.action != expected_action || result.service != expected_service {
        return Err(TestError::new(
            format!(
                "Batch[{}].Result[{}]: expected action '{}:{}' but got '{}:{}'",
                batch_index,
                result_index,
                expected_service,
                expected_action,
                result.service,
                result.action
            ),
            "",
        ));
    }

    let got = decision_name(result.decision);
    if got != expected_decision {
        return Err(TestError::new(
            format!(
                "Batch[{}].Result[{}] '{}:{}': expected decision '{}' but got '{}'",
                batch_index,
                result_index,
                expected_service,
                expected_action,
                expected_decision,
                got
            ),
            "",
        ));
    }

    Ok(())
}

fn format_grpc_multi_batch_request(args: &Args, condition: Option<&str>) -> String {
    format!(
        "gRPC CheckPermissionBatch (multi-batch)\n  condition: {}\n  batch[0]: principal=\"{}\" action={}:{} resource={}::\"{}\"\n  batch[1]: principal=\"{}\" action={}:{} resource={}::\"{}\"",
        condition.unwrap_or("unspecified"),
        args.principal,
        args.action_service, args.action_name,
        args.resource_type, args.resource_id,
        args.principal,
        DENY_ACTION_SERVICE, DENY_ACTION_NAME,
        args.resource_type, args.resource_id
    )
}

// ── Multi-batch CONDITION_AND ───────────────────────────────────────────────
// Batch 0: allow action. Batch 1: deny action.
// AND: allow → continue, deny → summary=deny.
async fn test_multi_batch_condition_and(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - multi-batch, condition: and";

    let request_str = format_grpc_multi_batch_request(args, Some("and"));
    let request = build_multi_batch_request(args, Some(Condition::And), auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_multi_batch_structure(&response, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        0,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        1,
        0,
        DENY_ACTION_NAME,
        DENY_ACTION_SERVICE,
        "deny",
    ) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary_decision(&response, "deny") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── Multi-batch CONDITION_OR ────────────────────────────────────────────────
// Batch 0: allow action. Batch 1: deny action.
// OR: allow → short-circuit, batch 1 skipped, summary=allow.
async fn test_multi_batch_condition_or(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - multi-batch, condition: or";

    let request_str = format_grpc_multi_batch_request(args, Some("or"));
    let request = build_multi_batch_request(args, Some(Condition::Or), auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_multi_batch_structure(&response, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        0,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        1,
        0,
        DENY_ACTION_NAME,
        DENY_ACTION_SERVICE,
        "skip",
    ) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_summary_decision(&response, "allow") {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

// ── Multi-batch CONDITION_UNSPECIFIED ────────────────────────────────────────
// Batch 0: allow action. Batch 1: deny action.
// No condition: both evaluated individually, no summary.
async fn test_multi_batch_condition_unspecified(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermissionBatch - multi-batch, condition: unspecified";

    let request_str = format_grpc_multi_batch_request(args, None);
    let request = build_multi_batch_request(args, None, auth_header);
    let response = match call_batch(client, request, name, &request_str).await {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail(name, start.elapsed().as_millis(), &e);
            return;
        }
    };

    let response_str = format!("{:?}", response);

    if let Err(e) = verify_multi_batch_structure(&response, 2) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        0,
        0,
        &args.action_name,
        &args.action_service,
        "allow",
    ) {
        let e = if decision_related(&e) {
            TestError::unexpected_decision("allow", "other", args)
        } else {
            e
        };
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if let Err(e) = verify_multi_batch_result(
        &response,
        1,
        0,
        DENY_ACTION_NAME,
        DENY_ACTION_SERVICE,
        "deny",
    ) {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(&request_str).with_response(&response_str),
        );
        return;
    }

    if response.summary.is_some() {
        let summary = response.summary.as_ref().unwrap();
        if summary.decision != Decision::Unspecified as i32 {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    "Expected no summary when condition is unspecified, but summary was present with a non-default decision",
                    format!("Summary decision: {}", decision_name(summary.decision)),
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
    grpc_url: &str,
    auth_header: Option<&str>,
    args: &Args,
) {
    let mut client = match connect(grpc_url).await {
        Ok(c) => c,
        Err(e) => {
            runner.record_fail("PermissionService.CheckPermissionBatch - connect", 0, &e);
            return;
        }
    };

    test_condition_and(runner, &mut client, auth_header, args).await;
    test_condition_or(runner, &mut client, auth_header, args).await;
    test_condition_unspecified(runner, &mut client, auth_header, args).await;
    test_multi_batch_condition_and(runner, &mut client, auth_header, args).await;
    test_multi_batch_condition_or(runner, &mut client, auth_header, args).await;
    test_multi_batch_condition_unspecified(runner, &mut client, auth_header, args).await;
}
