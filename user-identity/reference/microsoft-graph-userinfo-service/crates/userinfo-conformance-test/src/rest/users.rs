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

//! REST conformance tests for the user-related endpoints.
//!
//! Endpoints covered:
//! - `GET /api/v1alpha/users`
//! - `GET /api/v1alpha/users/{userId}`
//! - `GET /api/v1alpha/users/{userId}/groups`
//! - `GET /api/v1alpha/users/{userId}/groups/{groupId}`

use crate::fixtures::SampledFixtures;
use crate::rest::groups::validate_group_object;
use crate::{HINT, TestError, TestRunner};
use serde_json::Value;
use std::time::Instant;

/// Required and optional User fields per the OpenAPI spec.
const USER_REQUIRED: &[&str] = &["id", "display_name", "username"];
const USER_OPTIONAL: &[&str] = &["given_name", "family_name", "email", "active", "job_title"];

const PAGE_REQUIRED: &[&str] = &["items"];
const PAGE_OPTIONAL: &[&str] = &["next_cursor"];

pub(crate) async fn run_tests(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    test_get_user_by_id(runner, client, rest_url, auth_header, data).await;
    test_get_non_existent_user(runner, client, rest_url, auth_header, data).await;
    test_list_users_by_email(runner, client, rest_url, auth_header, data).await;
    test_list_users_by_display_name_prefix(runner, client, rest_url, auth_header, data).await;
    test_list_users_pagination(runner, client, rest_url, auth_header).await;
    test_list_users_invalid_cursor(runner, client, rest_url, auth_header).await;
    test_list_user_groups(runner, client, rest_url, auth_header, data).await;
    test_get_user_group(runner, client, rest_url, auth_header, data).await;
    test_get_user_non_existent_group(runner, client, rest_url, auth_header, data).await;
}

/// Sends a GET request and returns the parsed JSON value or a `TestError`
/// containing the request transcript and the response body.
pub(crate) async fn rest_get_json(
    client: &reqwest::Client,
    rest_url: &str,
    path: &str,
    auth_header: Option<&str>,
) -> Result<(reqwest::StatusCode, Value, String), TestError> {
    let url = format!("{rest_url}{path}");
    let request_str = format!("GET {url}");
    let mut req = client.get(&url);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| TestError::from_reqwest_error(&e, &url).with_request(&request_str))?;
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TestError::unexpected_status("GET", status, &body_text)
            .with_request(&request_str)
            .with_response(format!("HTTP {status}\n{body_text}")));
    }
    let body: Value = serde_json::from_str(&body_text).map_err(|e| {
        TestError::new(
            format!("Response is not valid JSON: {e}"),
            format!(
                "{HINT}   * The endpoint must return a JSON body matching the documented schema\n\
                 {HINT}   * Raw body (truncated): {}",
                &body_text[..body_text.len().min(256)]
            ),
        )
        .with_request(&request_str)
        .with_response(format!("HTTP {status}\n{body_text}"))
    })?;
    Ok((status, body, request_str))
}

/// Issues a GET expecting a specific non-success status code (e.g. 404). On
/// success it returns the parsed body as well so additional assertions on the
/// error body can be made by the caller.
pub(crate) async fn rest_get_expect_status(
    client: &reqwest::Client,
    rest_url: &str,
    path: &str,
    auth_header: Option<&str>,
    expected: u16,
) -> Result<(reqwest::StatusCode, String, String), TestError> {
    let url = format!("{rest_url}{path}");
    let request_str = format!("GET {url}");
    let mut req = client.get(&url);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| TestError::from_reqwest_error(&e, &url).with_request(&request_str))?;
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status.as_u16() != expected {
        return Err(TestError::new(
            format!(
                "Expected HTTP {expected} but got {} for {path}",
                status.as_u16()
            ),
            format!(
                "{HINT}   * The API contract specifies status {expected} for this case\n\
                 {HINT}   * Response body: {body_text}",
            ),
        )
        .with_request(&request_str)
        .with_response(format!("HTTP {status}\n{body_text}")));
    }
    Ok((status, body_text, request_str))
}

/// Validates a `User` JSON object. Checks that required fields are present and
/// non-empty, and that no field outside the documented schema is present.
pub(crate) fn validate_user_object(value: &Value) -> Result<(), TestError> {
    let obj = value.as_object().ok_or_else(|| {
        TestError::new(
            "User response is not a JSON object",
            format!("{HINT}   * The User schema is a JSON object"),
        )
    })?;
    for &field in USER_REQUIRED {
        match obj.get(field) {
            None => {
                return Err(TestError::new(
                    format!("User object is missing required field '{field}'"),
                    format!("{HINT}   * Required User fields per spec: {USER_REQUIRED:?}"),
                ));
            }
            Some(Value::String(s)) if s.is_empty() => {
                return Err(TestError::new(
                    format!("User object's required field '{field}' is empty"),
                    format!("{HINT}   * The User schema marks '{field}' as required"),
                ));
            }
            Some(Value::String(_)) => {}
            Some(other) => {
                return Err(TestError::new(
                    format!("User object's field '{field}' is not a string ({other})"),
                    format!("{HINT}   * User.{field} is of type string"),
                ));
            }
        }
    }
    if let Some(active) = obj.get("active")
        && !active.is_boolean()
    {
        return Err(TestError::new(
            format!("User.active is not a boolean ({active})"),
            format!("{HINT}   * User.active is of type boolean"),
        ));
    }
    let known: Vec<&str> = USER_REQUIRED
        .iter()
        .chain(USER_OPTIONAL.iter())
        .copied()
        .collect();
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            return Err(TestError::new(
                format!("User object contains unexpected field '{key}'"),
                format!(
                    "{HINT}   * Allowed User fields per spec: {known:?}\n\
                     {HINT}   * Implementations MUST NOT add fields outside the spec at the alpha API",
                ),
            ));
        }
    }
    Ok(())
}

/// Validates a paginated response object: required `items` (array) and
/// optional nullable `next_cursor` (string). Each item is validated with
/// `validate_item`.
pub(crate) fn validate_page<F>(
    value: &Value,
    item_kind: &str,
    validate_item: F,
) -> Result<(), TestError>
where
    F: Fn(&Value) -> Result<(), TestError>,
{
    let obj = value.as_object().ok_or_else(|| {
        TestError::new(
            format!("{item_kind} page response is not a JSON object"),
            format!("{HINT}   * Page schemas are JSON objects"),
        )
    })?;
    let items = obj.get("items").ok_or_else(|| {
        TestError::new(
            format!("{item_kind} page response is missing required field 'items'"),
            format!("{HINT}   * Required page fields per spec: {PAGE_REQUIRED:?}"),
        )
    })?;
    let arr = items.as_array().ok_or_else(|| {
        TestError::new(
            format!("{item_kind} page response field 'items' is not an array"),
            format!("{HINT}   * 'items' must be an array of {item_kind} objects"),
        )
    })?;
    for (i, item) in arr.iter().enumerate() {
        validate_item(item).map_err(|e| {
            TestError::new(
                format!(
                    "{item_kind} page response items[{i}] is invalid: {}",
                    e.message
                ),
                e.hint,
            )
        })?;
    }
    if let Some(cursor) = obj.get("next_cursor")
        && !cursor.is_null()
        && !cursor.is_string()
    {
        return Err(TestError::new(
            format!("{item_kind} page 'next_cursor' is neither null nor string"),
            format!("{HINT}   * next_cursor is `string | null` per spec"),
        ));
    }
    let known: Vec<&str> = PAGE_REQUIRED
        .iter()
        .chain(PAGE_OPTIONAL.iter())
        .copied()
        .collect();
    for key in obj.keys() {
        if !known.contains(&key.as_str()) {
            return Err(TestError::new(
                format!("{item_kind} page response contains unexpected field '{key}'"),
                format!("{HINT}   * Allowed page fields per spec: {known:?}"),
            ));
        }
    }
    Ok(())
}

async fn test_get_user_by_id(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users/{userId} returns the requested user";
    let path = format!("/api/v1alpha/users/{}", urlencoding(&data.user_id));
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
    let id = body["id"].as_str().unwrap_or("");
    if id != data.user_id {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Returned user id '{id}' != requested '{}'", data.user_id),
                format!("{HINT}   * The service must return the same user that was requested"),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_non_existent_user(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users/{userId} returns 404 for unknown user";
    let path = format!(
        "/api/v1alpha/users/{}",
        urlencoding(&data.nonexistent_user_id)
    );
    let start = Instant::now();
    if let Err(e) = rest_get_expect_status(client, rest_url, &path, auth_header, 404).await {
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_by_email(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users?email= filters by exact email";
    let target = &data.user_with_email;
    let path = format!(
        "/api/v1alpha/users?email={}&limit=100",
        urlencoding(&target.email),
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
    let found = items
        .iter()
        .any(|u| u["id"].as_str() == Some(target.id.as_str()));
    if !found {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled user '{}' (email '{}') not found in email-filter results",
                    target.id, target.email
                ),
                format!(
                    "{HINT}   * The 'email' parameter must match exactly\n\
                     {HINT}   * Filter must return the user that fixture sampling just observed via GET /api/v1alpha/users"
                ),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    for u in items {
        let email = u.get("email").and_then(|v| v.as_str()).unwrap_or("");
        if !email.eq_ignore_ascii_case(&target.email) {
            return runner.record_fail(
                name,
                start.elapsed().as_millis(),
                &TestError::new(
                    format!(
                        "Email-filter response contains user '{}' with email '{}' that does not match filter '{}'",
                        u.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        email,
                        target.email
                    ),
                    format!(
                        "{HINT}   * 'email' is an exact-match filter; every returned item MUST satisfy it"
                    ),
                )
                .with_request(request_str)
                .with_response(response_str),
            );
        }
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_by_display_name_prefix(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users?display_name=<prefix>* filters by display name prefix";
    let prefix = data.user_display_name.as_str();
    let prefix_query = format!("{prefix}*");
    let path = format!(
        "/api/v1alpha/users?display_name={}&limit=100",
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
        .any(|u| u["id"].as_str() == Some(data.user_id.as_str()))
    {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Sampled user '{}' (display_name '{}') not present in prefix-filter results for prefix '{prefix}*'",
                    data.user_id, data.user_display_name
                ),
                format!(
                    "{HINT}   * Trailing '*' selects the prefix variant of the display_name filter\n\
                     {HINT}   * The user observed by fixture sampling MUST satisfy a prefix filter built from its own display_name"
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
                        "Prefix-filter response contains user '{}' with display_name '{}' that does not start with prefix '{prefix}'",
                        u.get("id").and_then(|v| v.as_str()).unwrap_or(""),
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

async fn test_list_users_pagination(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
) {
    let name = "GET /api/v1alpha/users?limit=1 returns at most one item";
    let path = "/api/v1alpha/users?limit=1";
    let start = Instant::now();
    let (status, body, request_str) = match rest_get_json(client, rest_url, path, auth_header).await
    {
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
    let count = body["items"].as_array().unwrap().len();
    if count > 1 {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!("Expected at most 1 item with limit=1 but received {count}"),
                format!("{HINT}   * The 'limit' parameter must cap the page size as specified by the API contract"),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_users_invalid_cursor(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
) {
    let name = "GET /api/v1alpha/users?cursor=<malformed> returns 400";
    let path = format!("/api/v1alpha/users?cursor={}", urlencoding("!!!invalid!!!"));
    let start = Instant::now();
    if let Err(e) = rest_get_expect_status(client, rest_url, &path, auth_header, 400).await {
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_list_user_groups(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users/{userId}/groups lists the user's direct group memberships";
    let membership = &data.membership;
    let path = format!(
        "/api/v1alpha/users/{}/groups?limit=100",
        urlencoding(&membership.user_id),
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
        .any(|g| g["id"].as_str() == Some(membership.group_id.as_str()))
    {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Group '{}' not present in groups for user '{}', though fixture sampling just observed this membership via GET /api/v1alpha/users/{}/groups?limit=1",
                    membership.group_id, membership.user_id, membership.user_id
                ),
                format!(
                    "{HINT}   * GET /api/v1alpha/users/{{userId}}/groups MUST return the same memberships consistently between calls"
                ),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_user_group(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name = "GET /api/v1alpha/users/{userId}/groups/{groupId} confirms membership and returns the group";
    let membership = &data.membership;
    let path = format!(
        "/api/v1alpha/users/{}/groups/{}",
        urlencoding(&membership.user_id),
        urlencoding(&membership.group_id)
    );
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
    if body["id"].as_str() != Some(membership.group_id.as_str()) {
        return runner.record_fail(
            name,
            start.elapsed().as_millis(),
            &TestError::new(
                format!(
                    "Returned group id '{}' != requested '{}'",
                    body["id"].as_str().unwrap_or(""),
                    membership.group_id
                ),
                format!("{HINT}   * The service must return the requested group when membership is confirmed"),
            )
            .with_request(request_str)
            .with_response(response_str),
        );
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

async fn test_get_user_non_existent_group(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let name =
        "GET /api/v1alpha/users/{userId}/groups/{groupId} returns 404 when group is not a member";
    let path = format!(
        "/api/v1alpha/users/{}/groups/{}",
        urlencoding(&data.user_id),
        urlencoding(&data.nonexistent_group_id)
    );
    let start = Instant::now();
    if let Err(e) = rest_get_expect_status(client, rest_url, &path, auth_header, 404).await {
        runner.record_fail(name, start.elapsed().as_millis(), &e);
        return;
    }
    runner.record_pass(name, start.elapsed().as_millis());
}

pub(crate) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Minimal percent-encoding for path segments and query string values. We
/// avoid pulling in an additional dependency for a handful of characters.
pub(crate) fn urlencoding(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
