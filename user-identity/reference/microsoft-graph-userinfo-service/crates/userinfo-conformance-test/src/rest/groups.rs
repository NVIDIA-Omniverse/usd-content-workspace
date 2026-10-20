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

//! REST conformance tests for the group-related endpoints.
//!
//! Endpoints covered:
//! - `GET /api/v1alpha/groups`
//! - `GET /api/v1alpha/groups/{groupId}`
//! - `GET /api/v1alpha/groups/{groupId}/users`
//! - `GET /api/v1alpha/groups/{groupId}/users/{userId}`

use crate::fixtures::SampledFixtures;
use crate::rest::users::{
    pretty, rest_get_expect_status, rest_get_json, urlencoding, validate_page, validate_user_object,
};
use crate::{HINT, TestError, TestRunner};
use serde_json::Value;
use std::time::Instant;

const GROUP_REQUIRED: &[&str] = &["id", "display_name"];
const GROUP_OPTIONAL: &[&str] = &["description"];

pub(crate) async fn run_tests(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    test_get_group_by_id(runner, client, rest_url, auth_header, data).await;
    test_get_non_existent_group(runner, client, rest_url, auth_header, data).await;
    test_list_groups_by_display_name_prefix(runner, client, rest_url, auth_header, data).await;
    test_list_group_members(runner, client, rest_url, auth_header, data).await;
    test_list_group_members_filtered_by_display_name_prefix(
        runner,
        client,
        rest_url,
        auth_header,
        data,
    )
    .await;
    test_get_group_member(runner, client, rest_url, auth_header, data).await;
    test_get_group_non_existent_member(runner, client, rest_url, auth_header, data).await;
}

/// Validates a `Group` JSON object: required fields are present and non-empty,
/// no fields outside the documented schema are present.
pub(crate) fn validate_group_object(value: &Value) -> Result<(), TestError> {
    let obj = value.as_object().ok_or_else(|| {
        TestError::new(
            "Group response is not a JSON object",
            format!("{HINT}   * The Group schema is a JSON object"),
        )
    })?;
    for &field in GROUP_REQUIRED {
        match obj.get(field) {
            None => {
                return Err(TestError::new(
                    format!("Group object is missing required field '{field}'"),
                    format!("{HINT}   * Required Group fields per spec: {GROUP_REQUIRED:?}"),
                ));
            }
            Some(Value::String(s)) if s.is_empty() => {
                return Err(TestError::new(
                    format!("Group object's required field '{field}' is empty"),
                    format!("{HINT}   * The Group schema marks '{field}' as required"),
                ));
            }
            Some(Value::String(_)) => {}
            Some(other) => {
                return Err(TestError::new(
                    format!("Group object's field '{field}' is not a string ({other})"),
                    format!("{HINT}   * Group.{field} is of type string"),
                ));
            }
        }
    }
    let known: Vec<&str> = GROUP_REQUIRED
        .iter()
        .chain(GROUP_OPTIONAL.iter())
        .copied()
        .collect();
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            return Err(TestError::new(
                format!("Group object contains unexpected field '{key}'"),
                format!(
                    "{HINT}   * Allowed Group fields per spec: {known:?}\n\
                     {HINT}   * Implementations MUST NOT add fields outside the spec at the alpha API",
                ),
            ));
        }
    }
    Ok(())
}

async fn test_get_group_by_id(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/groups/{groupId} returns the requested group";
    let path = format!("/api/v1alpha/groups/{}", urlencoding(&data.group_id));
    let start = Instant::now();
    let (status, body, request_str) =
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Ok(r) => r,
            Err(e) => return runner.record_fail(name, start.elapsed().as_millis(), &e),
        };
    let response_str = format!("HTTP {status}\n{}", pretty(&body));
    if let Err(e) = validate_group_object(&body) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str).with_response(response_str),
        );
    }
    let id = body["id"].as_str().unwrap_or("");
    if id != data.group_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Returned group id '{id}' != requested '{}'", data.group_id),
                format!("{HINT}   * The service must return the same group that was requested"),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_non_existent_group(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/groups/{groupId} returns 404 for unknown group";
    let path = format!(
        "/api/v1alpha/groups/{}",
        urlencoding(&data.nonexistent_group_id)
    );
    let start = Instant::now();
    if let Err(e) = rest_get_expect_status(client, rest_url, &path, auth_header, 404).await {
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_groups_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/groups?display_name=<prefix>* filters by display name prefix";
    let prefix = data.group_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let path = format!(
        "/api/v1alpha/groups?display_name={}&limit=100",
        urlencoding(&prefix_query),
    );
    let start = Instant::now();
    let (status, body, request_str) =
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Ok(r) => r,
            Err(e) => return runner.record_fail(name, start.elapsed().as_millis(), &e),
        };
    let response_str = format!("HTTP {status}\n{}", pretty(&body));
    if let Err(e) = validate_page(&body, "Group", validate_group_object) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str).with_response(response_str),
        );
    }
    let items = body["items"].as_array().unwrap();
    if !items
        .iter()
        .any(|g| g["id"].as_str() == Some(data.group_id.as_str()))
    {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled group '{}' (display_name '{}') not present in prefix-filter results for prefix '{prefix}*'",
                    data.group_id, data.group_display_name
                ),
                format!(
                    "{HINT}   * Trailing '*' selects the prefix variant of the display_name filter\n\
                     {HINT}   * The group observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
                ),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    let prefix_lower = prefix.to_lowercase();
    for g in items {
        let dn = g.get("display_name").and_then(|v| v.as_str()).unwrap_or("");
        if !dn.to_lowercase().starts_with(&prefix_lower) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Prefix-filter response contains group '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        g.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        dn
                    ),
                    format!(
                        "{HINT}   * Every item returned by a prefix filter MUST have a display_name starting with the prefix"
                    ),
                )
                .with_request(request_str)
                .with_response(response_str),
            );
        }
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_group_members(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/groups/{groupId}/users lists direct group members";
    let membership = &data.membership;
    let path = format!(
        "/api/v1alpha/groups/{}/users?limit=100",
        urlencoding(&membership.group_id),
    );
    let start = Instant::now();
    let (status, body, request_str) =
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Ok(r) => r,
            Err(e) => return runner.record_fail(name, start.elapsed().as_millis(), &e),
        };
    let response_str = format!("HTTP {status}\n{}", pretty(&body));
    if let Err(e) = validate_page(&body, "User", validate_user_object) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str).with_response(response_str),
        );
    }
    let items = body["items"].as_array().unwrap();
    if !items
        .iter()
        .any(|u| u["id"].as_str() == Some(membership.user_id.as_str()))
    {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "User '{}' not present in members of group '{}', though fixture sampling just observed this membership via GET /api/v1alpha/users/{}/groups",
                    membership.user_id, membership.group_id, membership.user_id
                ),
                format!(
                    "{HINT}   * Membership is symmetric: if user U is in group G then ListGroupMembers(G) MUST include U"
                ),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

/// Verifies the `display_name` filter on the group-members listing using
/// the sampled member's own `display_name` as a prefix. Asserts that
/// (a) the filtered page contains the sampled member, and (b) every
/// returned user's `display_name` starts with the prefix — i.e. the filter
/// actually applies to every item the service returns.
async fn test_list_group_members_filtered_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/groups/{groupId}/users?display_name=<prefix>* filters members by display name prefix";
    let membership = &data.membership;
    let prefix = membership.user_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let path = format!(
        "/api/v1alpha/groups/{}/users?display_name={}&limit=100",
        urlencoding(&membership.group_id),
        urlencoding(&prefix_query),
    );
    let start = Instant::now();
    let (status, body, request_str) =
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Ok(r) => r,
            Err(e) => return runner.record_fail(name, start.elapsed().as_millis(), &e),
        };
    let response_str = format!("HTTP {status}\n{}", pretty(&body));
    if let Err(e) = validate_page(&body, "User", validate_user_object) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str).with_response(response_str),
        );
    }
    let items = body["items"].as_array().unwrap();
    if !items
        .iter()
        .any(|u| u["id"].as_str() == Some(membership.user_id.as_str()))
    {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled member '{}' (display_name '{}') of group '{}' is missing from members filtered by prefix '{prefix}*'",
                    membership.user_id, membership.user_display_name, membership.group_id
                ),
                format!(
                    "{HINT}   * Trailing '*' selects the prefix variant of the display_name filter\n\
                     {HINT}   * The member observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
                ),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    let prefix_lower = prefix.to_lowercase();
    for u in items {
        let dn = u.get("display_name").and_then(|v| v.as_str()).unwrap_or("");
        if !dn.to_lowercase().starts_with(&prefix_lower) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Members prefix-filter response contains user '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        u.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        dn
                    ),
                    format!(
                        "{HINT}   * Every item returned by a prefix filter on /groups/{{groupId}}/users MUST have a display_name starting with the prefix"
                    ),
                )
                .with_request(request_str)
                .with_response(response_str),
            );
        }
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_group_member(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name =
        "GET /api/v1alpha/groups/{groupId}/users/{userId} confirms membership and returns the user";
    let membership = &data.membership;
    let path = format!(
        "/api/v1alpha/groups/{}/users/{}",
        urlencoding(&membership.group_id),
        urlencoding(&membership.user_id)
    );
    let start = Instant::now();
    let (status, body, request_str) =
        match rest_get_json(client, rest_url, &path, auth_header).await {
            Ok(r) => r,
            Err(e) => return runner.record_fail(name, start.elapsed().as_millis(), &e),
        };
    let response_str = format!("HTTP {status}\n{}", pretty(&body));
    if let Err(e) = validate_user_object(&body) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &e.with_request(request_str).with_response(response_str),
        );
    }
    if body["id"].as_str() != Some(membership.user_id.as_str()) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned user id '{}' != requested '{}'",
                    body["id"].as_str().unwrap_or(""),
                    membership.user_id
                ),
                format!("{HINT}   * The service must return the requested user when membership is confirmed"),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_group_non_existent_member(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name =
        "GET /api/v1alpha/groups/{groupId}/users/{userId} returns 404 when user is not a member";
    let path = format!(
        "/api/v1alpha/groups/{}/users/{}",
        urlencoding(&data.group_id),
        urlencoding(&data.nonexistent_user_id)
    );
    let start = Instant::now();
    if let Err(e) = rest_get_expect_status(client, rest_url, &path, auth_header, 404).await {
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }
    runner.record_pass(name, start.elapsed().as_millis());
}
