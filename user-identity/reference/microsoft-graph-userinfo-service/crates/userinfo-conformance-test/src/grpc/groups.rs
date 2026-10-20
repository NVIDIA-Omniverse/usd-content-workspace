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

//! gRPC conformance tests for the group-related RPCs.
//!
//! Mirrors the REST groups-suite coverage over `UserInfoService.GetGroup`,
//! `ListGroups`, `ListGroupMembers`, and `GetGroupMember`.

use crate::config::Args;
use crate::fixtures::SampledFixtures;
use crate::grpc::{
    UserInfoServiceClient, bearer_request, format_grpc_request, proto, status_to_error,
    validate_group_proto, validate_user_proto,
};
use crate::{HINT, TestError, TestRunner};
use std::time::Instant;
use tonic::Code;
use tonic::transport::Channel;

pub(crate) async fn run_tests(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    _args: &Args,
    data: &SampledFixtures,
) {
    test_get_group(runner, client, bearer, data).await;
    test_get_non_existent_group(runner, client, bearer, data).await;
    test_list_groups_by_display_name_prefix(runner, client, bearer, data).await;
    test_list_group_members(runner, client, bearer, data).await;
    test_list_group_members_filtered_by_display_name_prefix(runner, client, bearer, data).await;
    test_get_group_member(runner, client, bearer, data).await;
    test_get_group_non_existent_member(runner, client, bearer, data).await;
}

async fn test_get_group(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetGroup returns the requested group";
    let start = Instant::now();
    let request_str = format_grpc_request("GetGroup", &[("group_id", &data.group_id)]);
    let req = match bearer_request(
        proto::GetGroupRequest {
            group_id: data.group_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    let resp = match client.get_group(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("GetGroup", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    let group = match resp.group.as_ref() {
        Some(g) => g,
        None => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    "GetGroupResponse.group is missing",
                    format!("{HINT}   * The response must contain the requested Group"),
                )
                .with_request(request_str),
            );
        }
    };
    if let Err(e) = validate_group_proto(group) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str)
                .with_response(format!("{group:?}")),
        );
    }
    if group.id != data.group_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned Group.id '{}' != requested '{}'",
                    group.id, data.group_id
                ),
                format!("{HINT}   * The service must return the same group that was requested"),
            )
            .with_request(request_str)
            .with_response(format!("{group:?}")),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_non_existent_group(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetGroup returns NOT_FOUND for unknown group";
    let start = Instant::now();
    let request_str = format_grpc_request("GetGroup", &[("group_id", &data.nonexistent_group_id)]);
    let req = match bearer_request(
        proto::GetGroupRequest {
            group_id: data.nonexistent_group_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    match client.get_group(req).await {
        Ok(r) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected NOT_FOUND but got OK",
                format!("{HINT}   * Expected to receive NOT_FOUND when the group does not exist"),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", r.into_inner())),
        ),
        Err(s) if s.code() == Code::NotFound => {
            runner.record_pass(name, start.elapsed().as_millis())
        }
        Err(s) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected NOT_FOUND but got {:?}", s.code()),
                format!(
                    "{HINT}   * Expected to receive NOT_FOUND for unknown groups\n\
                     {HINT}   * Server message: {}",
                    s.message()
                ),
            )
            .with_request(request_str),
        ),
    }
}

async fn test_list_groups_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.ListGroups filters by display_name prefix (suffix '*')";
    let start = Instant::now();
    let prefix = data.group_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let request_str = format_grpc_request(
        "ListGroups",
        &[("display_name", &prefix_query), ("page_size", "100")],
    );
    let req = match bearer_request(
        proto::ListGroupsRequest {
            display_name: Some(prefix_query.clone()),
            page_size: 100,
            page_token: String::new(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    let resp = match client.list_groups(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListGroups", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    let prefix_lower = prefix.to_lowercase();
    for g in &resp.groups {
        if let Err(e) = validate_group_proto(g) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str)
                    .with_response(format!("{:?}", resp.groups)),
            );
        }
        if !g.display_name.to_lowercase().starts_with(&prefix_lower) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "ListGroups prefix-filter response contains group '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        g.id, g.display_name
                    ),
                    format!("{HINT}   * Every item returned by a prefix filter MUST have a display_name starting with the prefix"),
                )
                .with_request(request_str)
                .with_response(format!("{:?}", resp.groups)),
            );
        }
    }
    if !resp.groups.iter().any(|g| g.id == data.group_id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled group '{}' (display_name '{}') not present in ListGroups prefix-filter results for '{prefix}*'",
                    data.group_id, data.group_display_name
                ),
                format!(
                    "{HINT}   * The group observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
                ),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.groups)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_group_members(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.ListGroupMembers lists direct group members";
    let membership = &data.membership;
    let start = Instant::now();
    let request_str =
        format_grpc_request("ListGroupMembers", &[("group_id", &membership.group_id)]);
    let req = match bearer_request(
        proto::ListGroupMembersRequest {
            group_id: membership.group_id.clone(),
            email: None,
            display_name: None,
            page_size: 100,
            page_token: String::new(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    let resp = match client.list_group_members(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListGroupMembers", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    for u in &resp.users {
        if let Err(e) = validate_user_proto(u) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str)
                    .with_response(format!("{:?}", resp.users)),
            );
        }
    }
    if !resp.users.iter().any(|u| u.id == membership.user_id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "User '{}' not present in members of group '{}', though fixture sampling just observed this membership via ListUserGroups",
                    membership.user_id, membership.group_id
                ),
                format!("{HINT}   * Membership is symmetric: if user U is in group G then ListGroupMembers(G) MUST include U"),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.users)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

/// Verifies the `display_name` filter on `ListGroupMembers` using the
/// sampled member's own `display_name` as a prefix. Asserts that the
/// filtered page contains the sampled member and that every returned
/// user's `display_name` starts with the prefix.
async fn test_list_group_members_filtered_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name =
        "UserInfoService.ListGroupMembers filters members by display_name prefix (suffix '*')";
    let membership = &data.membership;
    let prefix = membership.user_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let start = Instant::now();
    let request_str = format_grpc_request(
        "ListGroupMembers",
        &[
            ("group_id", &membership.group_id),
            ("display_name", &prefix_query),
            ("page_size", "100"),
        ],
    );
    let req = match bearer_request(
        proto::ListGroupMembersRequest {
            group_id: membership.group_id.clone(),
            email: None,
            display_name: Some(prefix_query.clone()),
            page_size: 100,
            page_token: String::new(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    let resp = match client.list_group_members(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListGroupMembers", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    let prefix_lower = prefix.to_lowercase();
    for u in &resp.users {
        if let Err(e) = validate_user_proto(u) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str)
                    .with_response(format!("{:?}", resp.users)),
            );
        }
        if !u.display_name.to_lowercase().starts_with(&prefix_lower) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "ListGroupMembers prefix-filter response contains user '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        u.id, u.display_name
                    ),
                    format!(
                        "{HINT}   * Every item returned by a prefix filter on ListGroupMembers MUST have a display_name starting with the prefix"
                    ),
                )
                .with_request(request_str)
                .with_response(format!("{:?}", resp.users)),
            );
        }
    }
    if !resp.users.iter().any(|u| u.id == membership.user_id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled member '{}' (display_name '{}') of group '{}' is missing from members filtered by prefix '{prefix}*'",
                    membership.user_id, membership.user_display_name, membership.group_id
                ),
                format!(
                    "{HINT}   * The member observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
                ),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.users)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_group_member(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetGroupMember confirms membership and returns the user";
    let membership = &data.membership;
    let start = Instant::now();
    let request_str = format_grpc_request(
        "GetGroupMember",
        &[
            ("group_id", &membership.group_id),
            ("user_id", &membership.user_id),
        ],
    );
    let req = match bearer_request(
        proto::GetGroupMemberRequest {
            group_id: membership.group_id.clone(),
            user_id: membership.user_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    let resp = match client.get_group_member(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("GetGroupMember", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    let user = match resp.user.as_ref() {
        Some(u) => u,
        None => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    "GetGroupMemberResponse.user is missing",
                    format!(
                        "{HINT}   * The response must contain the user when membership is confirmed"
                    ),
                )
                .with_request(request_str),
            );
        }
    };
    if let Err(e) = validate_user_proto(user) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str)
                .with_response(format!("{user:?}")),
        );
    }
    if user.id != membership.user_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned User.id '{}' != requested '{}'",
                    user.id, membership.user_id
                ),
                "".to_string(),
            )
            .with_request(request_str)
            .with_response(format!("{user:?}")),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_group_non_existent_member(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetGroupMember returns NOT_FOUND when user is not a member";
    let start = Instant::now();
    let request_str = format_grpc_request(
        "GetGroupMember",
        &[
            ("group_id", &data.group_id),
            ("user_id", &data.nonexistent_user_id),
        ],
    );
    let req = match bearer_request(
        proto::GetGroupMemberRequest {
            group_id: data.group_id.clone(),
            user_id: data.nonexistent_user_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str),
            );
        }
    };
    match client.get_group_member(req).await {
        Ok(r) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected NOT_FOUND but got OK",
                format!(
                    "{HINT}   * Expected to receive NOT_FOUND when the user is not a direct member"
                ),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", r.into_inner())),
        ),
        Err(s) if s.code() == Code::NotFound => {
            runner.record_pass(name, start.elapsed().as_millis())
        }
        Err(s) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected NOT_FOUND but got {:?}", s.code()),
                format!("{HINT}   * Server message: {}", s.message()),
            )
            .with_request(request_str),
        ),
    }
}
