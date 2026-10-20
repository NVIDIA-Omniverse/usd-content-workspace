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

//! gRPC conformance tests for the user-related RPCs.
//!
//! Mirrors the REST users-suite coverage over `UserInfoService.GetUser`,
//! `ListUsers`, `ListUserGroups`, and `GetUserGroup`.

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
    test_get_user(runner, client, bearer, data).await;
    test_get_non_existent_user(runner, client, bearer, data).await;
    test_list_users_by_email(runner, client, bearer, data).await;
    test_list_users_by_display_name_prefix(runner, client, bearer, data).await;
    test_list_users_pagination(runner, client, bearer).await;
    test_list_users_invalid_page_token(runner, client, bearer).await;
    test_list_user_groups(runner, client, bearer, data).await;
    test_get_user_group(runner, client, bearer, data).await;
    test_get_user_non_existent_group(runner, client, bearer, data).await;
}

async fn test_get_user(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetUser returns the requested user";
    let start = Instant::now();
    let request_str = format_grpc_request("GetUser", &[("user_id", &data.user_id)]);
    let req = match bearer_request(
        proto::GetUserRequest {
            user_id: data.user_id.clone(),
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
    let resp = match client.get_user(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("GetUser", &s).with_request(request_str);
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
                    "GetUserResponse.user is missing",
                    format!("{HINT}   * The response must contain the requested User"),
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
    if user.id != data.user_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned User.id '{}' != requested '{}'",
                    user.id, data.user_id
                ),
                format!("{HINT}   * The service must return the same user that was requested"),
            )
            .with_request(request_str)
            .with_response(format!("{user:?}")),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_non_existent_user(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetUser returns NOT_FOUND for unknown user";
    let start = Instant::now();
    let request_str = format_grpc_request("GetUser", &[("user_id", &data.nonexistent_user_id)]);
    let req = match bearer_request(
        proto::GetUserRequest {
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
    match client.get_user(req).await {
        Ok(r) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected NOT_FOUND but got OK",
                format!("{HINT}   * Expected to receive NOT_FOUND when the user does not exist"),
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
                    "{HINT}   * Expected to receive NOT_FOUND for unknown users\n\
                     {HINT}   * Server message: {}",
                    s.message()
                ),
            )
            .with_request(request_str),
        ),
    }
}

async fn test_list_users_by_email(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.ListUsers filters by exact email";
    let target = &data.user_with_email;
    let start = Instant::now();
    let request_str = format_grpc_request(
        "ListUsers",
        &[("email", &target.email), ("page_size", "100")],
    );
    let req = match bearer_request(
        proto::ListUsersRequest {
            email: Some(target.email.clone()),
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
    let resp = match client.list_users(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListUsers", &s).with_request(request_str);
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
        let email = u.email.as_deref().unwrap_or("");
        if !email.eq_ignore_ascii_case(&target.email) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "ListUsers email-filter response contains user '{}' with email '{email}' that does not match filter '{}'",
                        u.id, target.email
                    ),
                    format!("{HINT}   * 'email' is an exact-match filter; every returned item MUST satisfy it"),
                )
                .with_request(request_str)
                .with_response(format!("{:?}", resp.users)),
            );
        }
    }
    if !resp.users.iter().any(|u| u.id == target.id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled user '{}' (email '{}') not present in ListUsers email-filter results",
                    target.id, target.email
                ),
                format!(
                    "{HINT}   * Filter must return the user that fixture sampling just observed via ListUsers"
                ),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.users)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.ListUsers filters by display_name prefix (suffix '*')";
    let start = Instant::now();
    let prefix = data.user_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let request_str = format_grpc_request(
        "ListUsers",
        &[("display_name", &prefix_query), ("page_size", "100")],
    );
    let req = match bearer_request(
        proto::ListUsersRequest {
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
    let resp = match client.list_users(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListUsers", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    let prefix_lower = prefix.to_lowercase();
    for u in &resp.users {
        if !u.display_name.to_lowercase().starts_with(&prefix_lower) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "ListUsers prefix-filter response contains user '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        u.id, u.display_name
                    ),
                    format!("{HINT}   * Every item returned by a prefix filter MUST have a display_name starting with the prefix"),
                )
                .with_request(request_str)
                .with_response(format!("{:?}", resp.users)),
            );
        }
    }
    if !resp.users.iter().any(|u| u.id == data.user_id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled user '{}' (display_name '{}') not present in ListUsers prefix-filter results for '{prefix}*'",
                    data.user_id, data.user_display_name
                ),
                format!(
                    "{HINT}   * The user observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
                ),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.users)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_pagination(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
) {
    let name = "UserInfoService.ListUsers respects page_size=1";
    let start = Instant::now();
    let request_str = format_grpc_request("ListUsers", &[("page_size", "1")]);
    let req = match bearer_request(
        proto::ListUsersRequest {
            email: None,
            display_name: None,
            page_size: 1,
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
    let resp = match client.list_users(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListUsers", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    if resp.users.len() > 1 {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Expected at most 1 user with page_size=1 but got {}",
                    resp.users.len()
                ),
                format!("{HINT}   * page_size caps the number of items returned per page"),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.users)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_invalid_page_token(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
) {
    let name = "UserInfoService.ListUsers returns INVALID_ARGUMENT for a malformed page_token";
    let start = Instant::now();
    let request_str = format_grpc_request("ListUsers", &[("page_token", "!!!invalid!!!")]);
    let req = match bearer_request(
        proto::ListUsersRequest {
            email: None,
            display_name: None,
            page_size: 1,
            page_token: "!!!invalid!!!".to_string(),
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
    match client.list_users(req).await {
        Ok(r) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                "Expected INVALID_ARGUMENT but got OK",
                format!("{HINT}   * A malformed page_token is a client error and MUST return INVALID_ARGUMENT"),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", r.into_inner())),
        ),
        Err(s) if s.code() == Code::InvalidArgument => {
            runner.record_pass(name, start.elapsed().as_millis())
        }
        Err(s) => runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected INVALID_ARGUMENT but got {:?}", s.code()),
                format!(
                    "{HINT}   * A malformed page_token is a client error and MUST return INVALID_ARGUMENT\n\
                     {HINT}   * Server message: {}",
                    s.message()
                ),
            )
            .with_request(request_str),
        ),
    }
}

async fn test_list_user_groups(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.ListUserGroups returns the user's direct group memberships";
    let membership = &data.membership;
    let start = Instant::now();
    let request_str = format_grpc_request("ListUserGroups", &[("user_id", &membership.user_id)]);
    let req = match bearer_request(
        proto::ListUserGroupsRequest {
            user_id: membership.user_id.clone(),
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
    let resp = match client.list_user_groups(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("ListUserGroups", &s).with_request(request_str);
            return runner.record_fail(name, start.elapsed().as_millis(), &err);
        }
    };
    for g in &resp.groups {
        if let Err(e) = validate_group_proto(g) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &e.with_request(request_str)
                    .with_response(format!("{:?}", resp.groups)),
            );
        }
    }
    if !resp.groups.iter().any(|g| g.id == membership.group_id) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Group '{}' not present in groups for user '{}', though fixture sampling just observed this membership",
                    membership.group_id, membership.user_id
                ),
                format!("{HINT}   * ListUserGroups MUST consistently return the same memberships between calls"),
            )
            .with_request(request_str)
            .with_response(format!("{:?}", resp.groups)),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_user_group(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetUserGroup confirms membership and returns the group";
    let membership = &data.membership;
    let start = Instant::now();
    let request_str = format_grpc_request(
        "GetUserGroup",
        &[
            ("user_id", &membership.user_id),
            ("group_id", &membership.group_id),
        ],
    );
    let req = match bearer_request(
        proto::GetUserGroupRequest {
            user_id: membership.user_id.clone(),
            group_id: membership.group_id.clone(),
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
    let resp = match client.get_user_group(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            let err = status_to_error("GetUserGroup", &s).with_request(request_str);
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
                    "GetUserGroupResponse.group is missing",
                    format!(
                        "{HINT}   * The response must contain the group when membership is confirmed"
                    ),
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
    if group.id != membership.group_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned Group.id '{}' != requested '{}'",
                    group.id, membership.group_id
                ),
                "".to_string(),
            )
            .with_request(request_str)
            .with_response(format!("{group:?}")),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_user_non_existent_group(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "UserInfoService.GetUserGroup returns NOT_FOUND when group is not a member";
    let start = Instant::now();
    let request_str = format_grpc_request(
        "GetUserGroup",
        &[
            ("user_id", &data.user_id),
            ("group_id", &data.nonexistent_group_id),
        ],
    );
    let req = match bearer_request(
        proto::GetUserGroupRequest {
            user_id: data.user_id.clone(),
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
    match client.get_user_group(req).await {
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
