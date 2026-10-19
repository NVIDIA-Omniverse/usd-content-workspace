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

use super::proto::{
    permission_service_client::PermissionServiceClient, Action, CheckPermissionRequest, Decision,
    Principal, Resource,
};
use crate::config::Args;
use crate::{TestError, TestRunner, DENY_ACTION_NAME, DENY_ACTION_SERVICE, HINT};
use std::time::Instant;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::Request;

pub(super) async fn connect(grpc_url: &str) -> Result<PermissionServiceClient<Channel>, TestError> {
    let channel = Channel::from_shared(grpc_url.to_string())
        .map_err(|e| {
            TestError::new(
                format!("Invalid gRPC URL '{}': {}", grpc_url, e),
                format!(
                    "{}   Provide a valid URL (e.g. http://localhost:3010)",
                    HINT
                ),
            )
        })?
        .connect()
        .await
        .map_err(|e| TestError::connection_failed(grpc_url, &e.to_string()))?;

    Ok(PermissionServiceClient::new(channel))
}

fn build_allow_request(args: &Args, auth_header: Option<&str>) -> Request<CheckPermissionRequest> {
    let msg = CheckPermissionRequest {
        principal: Some(Principal {
            sub: args.principal.clone(),
            info: None,
        }),
        action: Some(Action {
            name: args.action_name.clone(),
            service: args.action_service.clone(),
        }),
        resource: Some(Resource {
            id: args.resource_id.clone(),
            r#type: args.resource_type.clone(),
            data: None,
        }),
        context: None,
    };
    attach_auth(Request::new(msg), auth_header)
}

fn build_deny_request(args: &Args, auth_header: Option<&str>) -> Request<CheckPermissionRequest> {
    let msg = CheckPermissionRequest {
        principal: Some(Principal {
            sub: args.principal.clone(),
            info: None,
        }),
        action: Some(Action {
            name: DENY_ACTION_NAME.to_string(),
            service: DENY_ACTION_SERVICE.to_string(),
        }),
        resource: Some(Resource {
            id: args.resource_id.clone(),
            r#type: args.resource_type.clone(),
            data: None,
        }),
        context: None,
    };
    attach_auth(Request::new(msg), auth_header)
}

pub(super) fn attach_auth<T>(mut request: Request<T>, auth_header: Option<&str>) -> Request<T> {
    if let Some(token) = auth_header {
        if let Ok(val) = token.parse::<MetadataValue<_>>() {
            request.metadata_mut().insert("authorization", val);
        }
    }
    request
}

pub(super) fn decision_name(value: i32) -> &'static str {
    match Decision::try_from(value) {
        Ok(Decision::Allow) => "allow",
        Ok(Decision::Deny) => "deny",
        Ok(Decision::Skip) => "skip",
        Ok(Decision::Unspecified) => "unspecified",
        Err(_) => "unknown",
    }
}

fn format_grpc_request(
    method: &str,
    principal: &str,
    action_service: &str,
    action_name: &str,
    resource_type: &str,
    resource_id: &str,
) -> String {
    format!(
        "gRPC {}\n  principal: \"{}\"\n  action: {}:{}\n  resource: {}::\"{}\"",
        method, principal, action_service, action_name, resource_type, resource_id
    )
}

async fn test_allow(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermission - allow";

    let request_str = format_grpc_request(
        "CheckPermission",
        &args.principal,
        &args.action_service,
        &args.action_name,
        &args.resource_type,
        &args.resource_id,
    );

    let request = build_allow_request(args, auth_header);
    let response = match client.check_permission(request).await {
        Ok(resp) => resp.into_inner(),
        Err(status) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!("gRPC call failed: {} ({})", status.message(), status.code()),
                    format!("{}   Check that the gRPC service is running", HINT),
                )
                .with_request(&request_str)
                .with_response(format!(
                    "gRPC error: {} ({})",
                    status.message(),
                    status.code()
                )),
            );
            return;
        }
    };

    let response_str = format!("{:?}", response);
    let got = decision_name(response.decision);
    if got != "allow" {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::unexpected_decision("allow", got, args)
                .with_request(&request_str)
                .with_response(&response_str),
        );
        return;
    }

    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_deny(
    runner: &mut TestRunner,
    client: &mut PermissionServiceClient<Channel>,
    auth_header: Option<&str>,
    args: &Args,
) {
    let start = Instant::now();
    let name = "PermissionService.CheckPermission - deny";

    let request_str = format_grpc_request(
        "CheckPermission",
        &args.principal,
        DENY_ACTION_SERVICE,
        DENY_ACTION_NAME,
        &args.resource_type,
        &args.resource_id,
    );

    let request = build_deny_request(args, auth_header);
    let response = match client.check_permission(request).await {
        Ok(resp) => resp.into_inner(),
        Err(status) => {
            runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!("gRPC call failed: {} ({})", status.message(), status.code()),
                    format!("{}   Check that the gRPC service is running", HINT),
                )
                .with_request(&request_str)
                .with_response(format!(
                    "gRPC error: {} ({})",
                    status.message(),
                    status.code()
                )),
            );
            return;
        }
    };

    let response_str = format!("{:?}", response);
    let got = decision_name(response.decision);
    if got != "deny" {
        runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected 'deny' but got '{}'", got),
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
            runner.record_fail("PermissionService.CheckPermission - connect", 0, &e);
            return;
        }
    };

    test_allow(runner, &mut client, auth_header, args).await;
    test_deny(runner, &mut client, auth_header, args).await;
}
