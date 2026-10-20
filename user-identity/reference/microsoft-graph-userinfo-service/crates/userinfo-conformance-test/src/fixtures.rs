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

//! Fixture sampling phase: probe the target service for the small set of
//! test fixtures (sample user, sample group, membership pair) that the
//! per-endpoint contract tests need. The conformance test does not assert
//! that any specific entity exists in the directory; it only asserts that
//! the service behaves correctly for whatever entities the directory
//! happens to contain.
//!
//! The phase is named "fixture sampling" rather than "discovery" to avoid
//! confusion with the unrelated "service discovery" concept commonly used
//! in microservice infrastructure.
//!
//! Each suite (REST, gRPC) samples its own fixtures against its own
//! transport so that a fault on one transport does not block testing of
//! the other. The sampled values are then used as inputs to the
//! per-endpoint tests.
//!
//! Membership sampling is **group-first**: the test lists groups, then for
//! each group probes `GET /groups/{id}/users?limit=1` (or the gRPC
//! equivalent) to find one with at least one direct member. Iterating from
//! the group side is dramatically cheaper than the user-first alternative
//! because most users are in at least one group, but the converse is not
//! true — many directories have a long tail of users who belong to no
//! group, which would otherwise force the probe to walk the entire user
//! list before finding a hit. Reversing the search makes the
//! membership-dependent tests run reliably without skipping.

use crate::grpc::{UserInfoServiceClient, bearer_request, proto, status_to_error};
use crate::rest::users::{rest_get_json, urlencoding};
use crate::{HINT, TestError, style};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::transport::Channel;

/// Inputs needed by the per-endpoint tests, all sampled from the live
/// service.
///
/// All fields are mandatory: if the service cannot supply any of them
/// (no users at all, no user with an `email`, no group with at least one
/// direct member), sampling fails with a clear error rather than letting
/// the dependent tests be silently skipped.
#[derive(Clone, Debug)]
pub struct SampledFixtures {
    /// Any user returned by the listing. Used for GetUser, display-name
    /// prefix tests, and as the user side of negative membership tests.
    pub user_id: String,
    pub user_display_name: String,

    /// A user that has a non-empty `email` field. Drives the email-filter
    /// test for `ListUsers`.
    pub user_with_email: UserWithEmail,

    /// Any group returned by the listing. Used for GetGroup, display-name
    /// prefix tests, and as the group side of negative membership tests.
    pub group_id: String,
    pub group_display_name: String,

    /// A pair `(user_id, group_id)` such that the user is a direct member
    /// of the group, sampled by listing groups and asking each one for its
    /// first member. Drives the positive membership tests
    /// (`ListUserGroups`, `GetUserGroup`, `ListGroupMembers`,
    /// `GetGroupMember`) and the `ListGroupMembers` display-name-prefix
    /// filter test.
    pub membership: Membership,

    /// Identifiers that are highly unlikely to exist in any directory.
    /// Generated fresh on each run so the tests do not depend on operator
    /// configuration or default values that an implementation might happen
    /// to special-case.
    pub nonexistent_user_id: String,
    pub nonexistent_group_id: String,
}

#[derive(Clone, Debug)]
pub struct UserWithEmail {
    pub id: String,
    pub email: String,
}

#[derive(Clone, Debug)]
pub struct Membership {
    pub user_id: String,
    /// Display name of the sampled member, used as the seed for the
    /// `ListGroupMembers` prefix-filter test.
    pub user_display_name: String,
    pub group_id: String,
    pub group_display_name: String,
}

/// Reports a single sampled fixture to stdout.
fn log_sample(label: &str, value: &str) {
    println!(
        "   {} sampled {}: {}",
        style::info_marker(),
        style::bold(label),
        style::dim(value),
    );
}

/// Generates a non-existent identifier. Includes the current time in
/// nanoseconds so each run gets a fresh value that no real directory entry
/// would match.
fn generate_nonexistent_id(kind: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("conformance-test-nonexistent-{kind}-{nanos}")
}

/// Samples fixtures via the REST API.
pub async fn sample_via_rest(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    page_limit: u32,
) -> Result<SampledFixtures, TestError> {
    let users = rest_list_users(client, rest_url, auth_header, page_limit).await?;
    let (user_id, user_display_name) = first_user(&users)?;
    log_sample(
        "user",
        &format!("id={user_id}, display_name={user_display_name:?}"),
    );

    let user_with_email = users
        .iter()
        .find_map(|u| {
            let id = u.get("id")?.as_str()?.to_string();
            let email = u.get("email")?.as_str()?.to_string();
            (!email.is_empty()).then_some(UserWithEmail { id, email })
        })
        .ok_or_else(|| {
            TestError::new(
                format!(
                    "Fixture sampling (REST): no user in the first {page_limit} returned by GET /api/v1alpha/users had a non-empty email",
                ),
                format!(
                    "{HINT}   * The conformance test needs at least one user with an email to verify the email filter\n\
                     {HINT}   * Increase --page-limit to scan more users, or seed the directory with a test user that has an email"
                ),
            )
        })?;
    log_sample(
        "user with email",
        &format!(
            "id={}, email={:?}",
            user_with_email.id, user_with_email.email
        ),
    );

    let groups = rest_list_groups(client, rest_url, auth_header, page_limit).await?;
    let (group_id, group_display_name) = first_group(&groups)?;
    log_sample(
        "group",
        &format!("id={group_id}, display_name={group_display_name:?}"),
    );

    let membership =
        rest_find_membership(client, rest_url, auth_header, &groups, page_limit).await?;
    log_sample(
        "membership pair",
        &format!(
            "group_id={}, group_display_name={:?}, user_id={}, user_display_name={:?}",
            membership.group_id,
            membership.group_display_name,
            membership.user_id,
            membership.user_display_name
        ),
    );

    Ok(SampledFixtures {
        user_id,
        user_display_name,
        user_with_email,
        group_id,
        group_display_name,
        membership,
        nonexistent_user_id: generate_nonexistent_id("user"),
        nonexistent_group_id: generate_nonexistent_id("group"),
    })
}

async fn rest_list_users(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    page_limit: u32,
) -> Result<Vec<Value>, TestError> {
    let path = format!("/api/v1alpha/users?limit={page_limit}");
    let (_, body, request_str) = rest_get_json(client, rest_url, &path, auth_header)
        .await
        .map_err(|e| e.with_request("Fixture sampling (REST): GET /api/v1alpha/users"))?;
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): GET /api/v1alpha/users response is missing 'items' array",
                format!(
                    "{HINT}   * The API contract requires a paginated `{{ items, next_cursor }}` envelope"
                ),
            )
            .with_request(request_str.clone())
        })?;
    if items.is_empty() {
        return Err(TestError::new(
            "Fixture sampling (REST): GET /api/v1alpha/users returned an empty page",
            format!(
                "{HINT}   * The conformance test needs at least one user to exist in the directory\n\
                 {HINT}   * Verify that the upstream identity provider has users and that the directory sync has completed"
            ),
        )
        .with_request(request_str));
    }
    Ok(items)
}

async fn rest_list_groups(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    page_limit: u32,
) -> Result<Vec<Value>, TestError> {
    let path = format!("/api/v1alpha/groups?limit={page_limit}");
    let (_, body, request_str) = rest_get_json(client, rest_url, &path, auth_header)
        .await
        .map_err(|e| e.with_request("Fixture sampling (REST): GET /api/v1alpha/groups"))?;
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): GET /api/v1alpha/groups response is missing 'items' array",
                format!(
                    "{HINT}   * The API contract requires a paginated `{{ items, next_cursor }}` envelope"
                ),
            )
            .with_request(request_str.clone())
        })?;
    if items.is_empty() {
        return Err(TestError::new(
            "Fixture sampling (REST): GET /api/v1alpha/groups returned an empty page",
            format!(
                "{HINT}   * The conformance test needs at least one group to exist in the directory\n\
                 {HINT}   * Verify that the upstream identity provider has groups and that the directory sync has completed"
            ),
        )
        .with_request(request_str));
    }
    Ok(items)
}

fn first_user(items: &[Value]) -> Result<(String, String), TestError> {
    let u = &items[0];
    let id = u
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): first user is missing 'id'",
                format!("{HINT}   * User.id is required"),
            )
        })?
        .to_string();
    let display_name = u
        .get("display_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): first user is missing 'display_name'",
                format!("{HINT}   * User.display_name is required"),
            )
        })?
        .to_string();
    Ok((id, display_name))
}

fn first_group(items: &[Value]) -> Result<(String, String), TestError> {
    let g = &items[0];
    let id = g
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): first group is missing 'id'",
                format!("{HINT}   * Group.id is required"),
            )
        })?
        .to_string();
    let display_name = g
        .get("display_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TestError::new(
                "Fixture sampling (REST): first group is missing 'display_name'",
                format!("{HINT}   * Group.display_name is required"),
            )
        })?
        .to_string();
    Ok((id, display_name))
}

/// Walks the supplied groups and returns the first `(group, member)` pair
/// for which the group has at least one direct member. Errors with a clear
/// message — including the last underlying probe failure if any — when no
/// such pair exists in the first `page_limit` groups.
async fn rest_find_membership(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    groups: &[Value],
    page_limit: u32,
) -> Result<Membership, TestError> {
    let mut probed = 0u32;
    let mut empty = 0u32;
    let mut last_failure: Option<TestError> = None;
    for g in groups {
        let Some(group_id) = g.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let group_display_name = g
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let path = format!(
            "/api/v1alpha/groups/{}/users?limit=1",
            urlencoding(group_id)
        );
        probed += 1;
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Err(e) => {
                last_failure = Some(e.with_request(format!("Fixture sampling (REST): GET {path}")));
                continue;
            }
            Ok((_, body, _)) => {
                let Some(items) = body.get("items").and_then(|v| v.as_array()) else {
                    continue;
                };
                let Some(member) = items.first() else {
                    empty += 1;
                    continue;
                };
                let Some(user_id) = member.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(user_display_name) = member
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                return Ok(Membership {
                    user_id: user_id.to_string(),
                    user_display_name: user_display_name.to_string(),
                    group_id: group_id.to_string(),
                    group_display_name,
                });
            }
        }
    }
    Err(membership_sample_error(
        "REST",
        "GET /api/v1alpha/groups/{groupId}/users?limit=1",
        page_limit,
        probed,
        empty,
        last_failure,
    ))
}

/// Builds a structured failure for membership sampling, distinguishing
/// "directory is empty / no group has members" from "every probe failed
/// with the same underlying error" (for example, the upstream identity
/// provider is unreachable). When at least one probe failed, the last
/// failure's message is included so the operator can act on the real
/// cause without grepping the service logs. The `protocol` argument
/// (e.g. "REST", "gRPC") tags the message so the suite that produced
/// the failure is unambiguous.
fn membership_sample_error(
    protocol: &str,
    probe_label: &str,
    page_limit: u32,
    probed: u32,
    empty: u32,
    last_failure: Option<TestError>,
) -> TestError {
    let failed = probed.saturating_sub(empty);
    let summary = format!(
        "Fixture sampling ({protocol}): probed {probed} of the first {page_limit} groups via {probe_label} \
         and could not obtain a (group, member) pair ({empty} returned an empty page, \
         {failed} failed before returning a page)",
    );
    let mut hint = format!(
        "{HINT}   * The conformance test needs at least one group with at least one direct member to verify membership endpoints\n\
         {HINT}   * Increase --page-limit to scan more groups, or seed the directory with a test group that has at least one member"
    );
    if let Some(e) = last_failure {
        hint.push_str(&format!(
            "\n{HINT}   * Last underlying probe failure: {}",
            e.message
        ));
        if !e.hint.is_empty() {
            for line in e.hint.lines() {
                hint.push_str(&format!("\n     {line}"));
            }
        }
        if let Some(req) = e.request {
            hint.push_str(&format!("\n{HINT}   * Last failing request: {req}"));
        }
        if let Some(resp) = e.response {
            let preview: String = resp.chars().take(500).collect();
            hint.push_str(&format!(
                "\n{HINT}   * Last failing response (truncated): {preview}"
            ));
        }
    }
    TestError::new(summary, hint)
}

/// Samples fixtures via the gRPC API. Mirrors REST sampling so that the
/// gRPC suite is independent of REST.
pub async fn sample_via_grpc(
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    page_limit: u32,
) -> Result<SampledFixtures, TestError> {
    let users = grpc_list_users(client, bearer, page_limit).await?;
    let user = users.first().ok_or_else(|| {
        TestError::new(
            "Fixture sampling (gRPC): ListUsers returned no users",
            format!(
                "{HINT}   * The conformance test needs at least one user to exist in the directory\n\
                 {HINT}   * Verify the directory and that the initial sync completed"
            ),
        )
    })?;
    let user_id = user.id.clone();
    let user_display_name = user.display_name.clone();
    log_sample(
        "user",
        &format!("id={user_id}, display_name={user_display_name:?}"),
    );

    let user_with_email = users
        .iter()
        .find_map(|u| {
            let email = u.email.as_ref()?.clone();
            (!email.is_empty()).then(|| UserWithEmail {
                id: u.id.clone(),
                email,
            })
        })
        .ok_or_else(|| {
            TestError::new(
                format!(
                    "Fixture sampling (gRPC): no user in the first {page_limit} returned by ListUsers had a non-empty email",
                ),
                format!(
                    "{HINT}   * The conformance test needs at least one user with an email to verify the email filter\n\
                     {HINT}   * Increase --page-limit to scan more users, or seed the directory with a test user that has an email"
                ),
            )
        })?;
    log_sample(
        "user with email",
        &format!(
            "id={}, email={:?}",
            user_with_email.id, user_with_email.email
        ),
    );

    let groups = grpc_list_groups(client, bearer, page_limit).await?;
    let group = groups.first().ok_or_else(|| {
        TestError::new(
            "Fixture sampling (gRPC): ListGroups returned no groups",
            format!(
                "{HINT}   * The conformance test needs at least one group to exist in the directory\n\
                 {HINT}   * Verify the directory and that the initial sync completed"
            ),
        )
    })?;
    let group_id = group.id.clone();
    let group_display_name = group.display_name.clone();
    log_sample(
        "group",
        &format!("id={group_id}, display_name={group_display_name:?}"),
    );

    let membership = grpc_find_membership(client, bearer, &groups, page_limit).await?;
    log_sample(
        "membership pair",
        &format!(
            "group_id={}, group_display_name={:?}, user_id={}, user_display_name={:?}",
            membership.group_id,
            membership.group_display_name,
            membership.user_id,
            membership.user_display_name
        ),
    );

    Ok(SampledFixtures {
        user_id,
        user_display_name,
        user_with_email,
        group_id,
        group_display_name,
        membership,
        nonexistent_user_id: generate_nonexistent_id("user"),
        nonexistent_group_id: generate_nonexistent_id("group"),
    })
}

async fn grpc_list_users(
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    page_limit: u32,
) -> Result<Vec<proto::User>, TestError> {
    let req = bearer_request(
        proto::ListUsersRequest {
            email: None,
            display_name: None,
            page_size: page_limit as i32,
            page_token: String::new(),
        },
        bearer,
    )
    .map_err(|e| e.with_request("Fixture sampling (gRPC): UserInfoService/ListUsers"))?;
    let resp = client
        .list_users(req)
        .await
        .map_err(|s| status_to_error("Fixture sampling (gRPC): ListUsers", &s))?;
    Ok(resp.into_inner().users)
}

async fn grpc_list_groups(
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    page_limit: u32,
) -> Result<Vec<proto::Group>, TestError> {
    let req = bearer_request(
        proto::ListGroupsRequest {
            display_name: None,
            page_size: page_limit as i32,
            page_token: String::new(),
        },
        bearer,
    )
    .map_err(|e| e.with_request("Fixture sampling (gRPC): UserInfoService/ListGroups"))?;
    let resp = client
        .list_groups(req)
        .await
        .map_err(|s| status_to_error("Fixture sampling (gRPC): ListGroups", &s))?;
    Ok(resp.into_inner().groups)
}

/// gRPC mirror of `rest_find_membership`: walks the supplied groups and
/// returns the first `(group, member)` pair it can confirm via
/// `ListGroupMembers`. Captures the last underlying probe failure for the
/// sampling error message.
async fn grpc_find_membership(
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    groups: &[proto::Group],
    page_limit: u32,
) -> Result<Membership, TestError> {
    let mut probed = 0u32;
    let mut empty = 0u32;
    let mut last_failure: Option<TestError> = None;
    for g in groups {
        let req = match bearer_request(
            proto::ListGroupMembersRequest {
                group_id: g.id.clone(),
                email: None,
                display_name: None,
                page_size: 1,
                page_token: String::new(),
            },
            bearer,
        ) {
            Ok(r) => r,
            Err(e) => {
                last_failure = Some(e);
                continue;
            }
        };
        probed += 1;
        match client.list_group_members(req).await {
            Err(s) => {
                last_failure = Some(
                    status_to_error("Fixture sampling (gRPC): ListGroupMembers", &s)
                        .with_request(format!("ListGroupMembers(group_id={}, page_size=1)", g.id)),
                );
                continue;
            }
            Ok(resp) => {
                let Some(member) = resp.into_inner().users.into_iter().next() else {
                    empty += 1;
                    continue;
                };
                if member.id.is_empty() || member.display_name.is_empty() {
                    continue;
                }
                return Ok(Membership {
                    user_id: member.id,
                    user_display_name: member.display_name,
                    group_id: g.id.clone(),
                    group_display_name: g.display_name.clone(),
                });
            }
        }
    }
    Err(membership_sample_error(
        "gRPC",
        "ListGroupMembers(group_id, page_size=1)",
        page_limit,
        probed,
        empty,
        last_failure,
    ))
}
