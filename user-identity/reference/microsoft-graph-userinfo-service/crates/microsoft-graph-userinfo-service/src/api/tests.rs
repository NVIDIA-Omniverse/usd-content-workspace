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

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use axum::http::StatusCode;
use axum_test::TestServer;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use mockito::{Matcher, Server};
use serde::Deserialize;
use serde_json::json;

use crate::entra::client::EntraClient;
use crate::entra::provider::{EntraAppRoleAssignments, EntraProvider};
use crate::models::{Group, MAX_PAGE_SIZE, Page, User};
use crate::permission::{PermissionChecker, PermissionConfig};
use crate::provider::{AppRoleAssignmentsSource, ProviderError};

use super::{AppState, router};

fn graph_user_json(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "displayName": "Jane Doe",
        "givenName": "Jane",
        "surname": "Doe",
        "mail": "jane@example.com",
        "userPrincipalName": "jane@corp.com",
        "accountEnabled": true,
        "jobTitle": "Engineer"
    })
}

fn graph_group_json(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "displayName": "Admins",
        "description": "Admin group"
    })
}

async fn mock_token(server: &mut Server) -> mockito::Mock {
    server
        .mock("POST", "/token")
        .with_body(r#"{"access_token":"test-token","expires_in":3600}"#)
        .create_async()
        .await
}

fn jwt_with_oid(oid: &str) -> String {
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        oid: &'a str,
        sub: &'a str,
    }

    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &Claims { oid, sub: oid },
        &jsonwebtoken::EncodingKey::from_secret(b"test"),
    )
    .unwrap()
}

fn build_test_server(mock_url: &str) -> TestServer {
    build_test_server_with_retries(mock_url, 3)
}

fn build_test_server_with_retries(mock_url: &str, max_retries: u32) -> TestServer {
    let client = Arc::new(EntraClient::with_base_url(
        "test-tenant".into(),
        "test-client".into(),
        "test-secret".into(),
        mock_url,
        max_retries,
    ));
    let provider = EntraProvider::new(client, MAX_PAGE_SIZE);
    let state = AppState {
        provider: Arc::new(provider),
        ready: Arc::new(AtomicBool::new(true)),
        permission_checker: None,
        access: Arc::new(crate::access::AccessLog::new(false, Vec::new())),
        forward_headers: Arc::new(crate::config::ForwardHeaders(Vec::new())),
    };
    TestServer::new(router(state)).expect("failed to create test server")
}

fn assert_sample_user(user: &User, expected_id: &str) {
    assert_eq!(user.id, expected_id);
    assert_eq!(user.display_name, "Jane Doe");
    assert_eq!(user.given_name.as_deref(), Some("Jane"));
    assert_eq!(user.family_name.as_deref(), Some("Doe"));
    assert_eq!(user.email.as_deref(), Some("jane@example.com"));
    assert_eq!(user.username, "jane@corp.com");
    assert_eq!(user.active, Some(true));
    assert_eq!(user.job_title.as_deref(), Some("Engineer"));
}

fn assert_sample_group(group: &Group, expected_id: &str) {
    assert_eq!(group.id, expected_id);
    assert_eq!(group.display_name, "Admins");
    assert_eq!(group.description.as_deref(), Some("Admin group"));
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ErrorResponse {
    error: String,
    message: String,
}

// ========== GET /api/v1alpha/users ==========

#[tokio::test]
async fn list_users_returns_page() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1"), graph_user_json("u2")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 2);
    assert_sample_user(&page.items[0], "u1");
    assert_sample_user(&page.items[1], "u2");
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn list_users_forwards_default_limit() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded("$top".into(), "20".into()))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server.get("/api/v1alpha/users").await.assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_forwards_email_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "mail eq 'jane@example.com'".into(),
        ))
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?email=jane@example.com")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_forwards_display_name_search() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded(
            "$search".into(),
            "\"displayName:Jane\"".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?display_name=Jane")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_forwards_custom_limit() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded("$top".into(), "50".into()))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?limit=50")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_caps_limit_at_100() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded("$top".into(), "100".into()))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?limit=999")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_encodes_next_link_as_cursor() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let next_link = format!("{}/users?$skiptoken=page2", mock.url());
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(
            json!({ "value": [graph_user_json("u1")], "@odata.nextLink": &next_link }).to_string(),
        )
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<User> = server.get("/api/v1alpha/users").await.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u1");
    let cursor = page.next_cursor.unwrap();
    let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(&cursor).unwrap()).unwrap();
    assert_eq!(decoded, next_link);
}

#[tokio::test]
async fn list_users_decodes_cursor_as_continuation_url() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let continuation = format!("{}/users?$skiptoken=page2", mock.url());
    let cursor = URL_SAFE_NO_PAD.encode(&continuation);
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded("$skiptoken".into(), "page2".into()))
        .with_body(json!({ "value": [graph_user_json("u2")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<User> = server
        .get(&format!("/api/v1alpha/users?cursor={cursor}"))
        .await
        .json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u2");
}

#[tokio::test]
async fn list_users_invalid_cursor_returns_400() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());

    let resp = server.get("/api/v1alpha/users?cursor=!!!invalid!!!").await;
    resp.assert_status(StatusCode::BAD_REQUEST);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "bad_request");
}

#[tokio::test]
async fn list_users_empty() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<User> = server.get("/api/v1alpha/users").await.json();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn list_users_wildcard_sends_starts_with_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "startsWith(displayName,'Jan')".into(),
        ))
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?display_name=Jan*")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_wildcard_escapes_single_quotes() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "startsWith(displayName,'O''Bri')".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?display_name=O'Bri*")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_wildcard_combined_with_email_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let users_mock = mock
        .mock("GET", "/users")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "mail eq 'jane@example.com' and startsWith(displayName,'Jan')".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users?email=jane@example.com&display_name=Jan*")
        .await
        .assert_status_ok();
    users_mock.assert_async().await;
}

// ========== GET /api/v1alpha/users/:id ==========

#[tokio::test]
async fn get_user_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/uid-42")
        .match_query(Matcher::Any)
        .with_body(graph_user_json("uid-42").to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-42").await;
    resp.assert_status_ok();
    let user: User = resp.json();
    assert_sample_user(&user, "uid-42");
}

#[tokio::test]
async fn get_user_not_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/nonexistent")
        .match_query(Matcher::Any)
        .with_status(404)
        .with_body("user not found")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/nonexistent").await;
    resp.assert_status(StatusCode::NOT_FOUND);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "not_found");
}

// ========== GET /api/v1alpha/users/me ==========

#[tokio::test]
async fn get_me_resolves_user_from_jwt_oid() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let user_mock = mock
        .mock("GET", "/users/uid-from-jwt")
        .match_query(Matcher::Any)
        .with_body(graph_user_json("uid-from-jwt").to_string())
        .expect(1)
        .create_async()
        .await;

    let bearer = jwt_with_oid("uid-from-jwt");
    let server = build_test_server(&mock.url());
    let resp = server
        .get("/api/v1alpha/users/me")
        .add_header("Authorization", format!("Bearer {bearer}"))
        .await;
    resp.assert_status_ok();
    let user: User = resp.json();
    assert_sample_user(&user, "uid-from-jwt");
    user_mock.assert_async().await;
}

#[tokio::test]
async fn get_me_missing_auth_header() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());

    let resp = server.get("/api/v1alpha/users/me").await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "unauthorized");
}

#[tokio::test]
async fn get_me_non_bearer_auth_header() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());

    let resp = server
        .get("/api/v1alpha/users/me")
        .add_header("Authorization", "Basic user:pass")
        .await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_me_non_jwt_token_returns_401() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());
    let resp = server
        .get("/api/v1alpha/users/me")
        .add_header("Authorization", "Bearer not-a-jwt")
        .await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
}

// ========== GET /api/v1alpha/users/:id/groups ==========

#[tokio::test]
async fn list_user_groups_returns_page() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _groups = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_group_json("g1"), graph_group_json("g2")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1/groups").await;
    resp.assert_status_ok();
    let page: Page<Group> = resp.json();
    assert_eq!(page.items.len(), 2);
    assert_sample_group(&page.items[0], "g1");
    assert_sample_group(&page.items[1], "g2");
}

#[tokio::test]
async fn list_user_groups_forwards_display_name_search() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let groups_mock = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("$search".into(), "\"displayName:Admin\"".into()),
            Matcher::UrlEncoded("$top".into(), "5".into()),
        ]))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users/uid-1/groups?display_name=Admin&limit=5")
        .await
        .assert_status_ok();
    groups_mock.assert_async().await;
}

#[tokio::test]
async fn list_user_groups_wildcard_sends_starts_with_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let groups_mock = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "startsWith(displayName,'Adm')".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/users/uid-1/groups?display_name=Adm*")
        .await
        .assert_status_ok();
    groups_mock.assert_async().await;
}

// ========== GET /api/v1alpha/users/:id/groups/:groupId ==========

#[tokio::test]
async fn get_user_group_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _groups = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "id eq 'gid-1'".into(),
        ))
        .with_body(json!({ "value": [graph_group_json("gid-1")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1/groups/gid-1").await;
    resp.assert_status_ok();
    let group: Group = resp.json();
    assert_sample_group(&group, "gid-1");
}

#[tokio::test]
async fn get_user_group_not_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _groups = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1/groups/gid-99").await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ========== GET /api/v1alpha/groups/:groupId/users ==========

#[tokio::test]
async fn list_group_members_returns_page() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _members = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1"), graph_user_json("u2")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/gid-1/users").await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 2);
    assert_sample_user(&page.items[0], "u1");
    assert_sample_user(&page.items[1], "u2");
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn list_group_members_forwards_email_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let members_mock = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "mail eq 'jane@example.com'".into(),
        ))
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups/gid-1/users?email=jane@example.com")
        .await
        .assert_status_ok();
    members_mock.assert_async().await;
}

#[tokio::test]
async fn list_group_members_forwards_display_name_search() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let members_mock = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded(
            "$search".into(),
            "\"displayName:Jane\"".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups/gid-1/users?display_name=Jane")
        .await
        .assert_status_ok();
    members_mock.assert_async().await;
}

#[tokio::test]
async fn list_group_members_wildcard_sends_starts_with_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let members_mock = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "startsWith(displayName,'Jan')".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups/gid-1/users?display_name=Jan*")
        .await
        .assert_status_ok();
    members_mock.assert_async().await;
}

#[tokio::test]
async fn list_group_members_forwards_custom_limit() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let members_mock = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded("$top".into(), "5".into()))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups/gid-1/users?limit=5")
        .await
        .assert_status_ok();
    members_mock.assert_async().await;
}

#[tokio::test]
async fn list_group_members_with_pagination() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let next_link = format!(
        "{}/groups/gid-1/members/microsoft.graph.user?$skiptoken=page2",
        mock.url()
    );
    let _members = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(
            json!({ "value": [graph_user_json("u1")], "@odata.nextLink": &next_link }).to_string(),
        )
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<User> = server.get("/api/v1alpha/groups/gid-1/users").await.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u1");
    let cursor = page.next_cursor.unwrap();
    let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(&cursor).unwrap()).unwrap();
    assert_eq!(decoded, next_link);
}

#[tokio::test]
async fn list_group_members_empty() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _members = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<User> = server.get("/api/v1alpha/groups/gid-1/users").await.json();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn list_group_members_group_not_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _members = mock
        .mock("GET", "/groups/nonexistent/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_status(404)
        .with_body("group not found")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/nonexistent/users").await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ========== GET /api/v1alpha/groups/:groupId/users/:userId ==========

#[tokio::test]
async fn get_group_member_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _members = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "id eq 'uid-1'".into(),
        ))
        .with_body(json!({ "value": [graph_user_json("uid-1")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/gid-1/users/uid-1").await;
    resp.assert_status_ok();
    let user: User = resp.json();
    assert_sample_user(&user, "uid-1");
}

#[tokio::test]
async fn get_group_member_not_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _members = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/gid-1/users/uid-99").await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ========== GET /api/v1alpha/groups ==========

#[tokio::test]
async fn list_groups_returns_page() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _groups = mock
        .mock("GET", "/groups")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_group_json("g1"), graph_group_json("g2")] }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<Group> = server.get("/api/v1alpha/groups").await.json();
    assert_eq!(page.items.len(), 2);
    assert_sample_group(&page.items[0], "g1");
    assert_sample_group(&page.items[1], "g2");
}

#[tokio::test]
async fn list_groups_forwards_display_name_search() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let groups_mock = mock
        .mock("GET", "/groups")
        .match_query(Matcher::UrlEncoded(
            "$search".into(),
            "\"displayName:Engineering\"".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups?display_name=Engineering")
        .await
        .assert_status_ok();
    groups_mock.assert_async().await;
}

#[tokio::test]
async fn list_groups_wildcard_sends_starts_with_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let groups_mock = mock
        .mock("GET", "/groups")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "startsWith(displayName,'Eng')".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server
        .get("/api/v1alpha/groups?display_name=Eng*")
        .await
        .assert_status_ok();
    groups_mock.assert_async().await;
}

#[tokio::test]
async fn list_groups_forwards_default_limit() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let groups_mock = mock
        .mock("GET", "/groups")
        .match_query(Matcher::UrlEncoded("$top".into(), "20".into()))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    server.get("/api/v1alpha/groups").await.assert_status_ok();
    groups_mock.assert_async().await;
}

#[tokio::test]
async fn list_groups_with_pagination() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let next_link = format!("{}/groups?$skiptoken=page2", mock.url());
    let _groups = mock
        .mock("GET", "/groups")
        .match_query(Matcher::Any)
        .with_body(
            json!({ "value": [graph_group_json("g1")], "@odata.nextLink": &next_link }).to_string(),
        )
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let page: Page<Group> = server.get("/api/v1alpha/groups?limit=1").await.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_group(&page.items[0], "g1");
    let cursor = page.next_cursor.unwrap();
    let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(&cursor).unwrap()).unwrap();
    assert_eq!(decoded, next_link);
}

// ========== GET /api/v1alpha/groups/:id ==========

#[tokio::test]
async fn get_group_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _group = mock
        .mock("GET", "/groups/gid-42")
        .match_query(Matcher::Any)
        .with_body(graph_group_json("gid-42").to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/gid-42").await;
    resp.assert_status_ok();
    let group: Group = resp.json();
    assert_sample_group(&group, "gid-42");
}

#[tokio::test]
async fn get_group_not_found() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _group = mock
        .mock("GET", "/groups/nonexistent")
        .match_query(Matcher::Any)
        .with_status(404)
        .with_body("group not found")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/groups/nonexistent").await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ========== Error Mapping ==========

#[tokio::test]
async fn graph_500_maps_to_bad_gateway() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body("internal server error")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status(StatusCode::BAD_GATEWAY);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "bad_gateway");
}

#[tokio::test]
async fn graph_403_maps_to_forbidden() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/uid-1")
        .match_query(Matcher::Any)
        .with_status(403)
        .with_body("insufficient privileges")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1").await;
    resp.assert_status(StatusCode::FORBIDDEN);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "forbidden");
}

// ========== Field Mapping (Entra → Public Models) ==========

#[tokio::test]
async fn user_fields_mapped_from_graph() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/u1")
        .match_query(Matcher::Any)
        .with_body(graph_user_json("u1").to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let user: User = server.get("/api/v1alpha/users/u1").await.json();
    assert_sample_user(&user, "u1");
}

#[tokio::test]
async fn user_none_fields_omitted_in_json() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/sparse")
        .match_query(Matcher::Any)
        .with_body(json!({ "id": "sparse" }).to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let body: serde_json::Value = server.get("/api/v1alpha/users/sparse").await.json();
    assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("sparse"));
    assert_eq!(body.get("display_name").and_then(|v| v.as_str()), Some(""));
    assert!(body.get("email").is_none());
}

#[tokio::test]
async fn group_fields_mapped_from_graph() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _group = mock
        .mock("GET", "/groups/g1")
        .match_query(Matcher::Any)
        .with_body(graph_group_json("g1").to_string())
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let group: Group = server.get("/api/v1alpha/groups/g1").await.json();
    assert_sample_group(&group, "g1");
}

// ========== 429 Retry Logic ==========

#[tokio::test]
async fn retry_on_429_then_success() {
    tokio::time::pause();
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;

    let _success = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .create_async()
        .await;

    let _rate_limited = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_status(429)
        .with_header("Retry-After", "1")
        .with_body("rate limited")
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u1");
}

#[tokio::test]
async fn retry_exhausted_on_429_returns_429() {
    tokio::time::pause();
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;

    let _rate_limited = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_status(429)
        .with_header("Retry-After", "2")
        .with_body("rate limited")
        .create_async()
        .await;

    let server = build_test_server_with_retries(&mock.url(), 2);
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status(StatusCode::TOO_MANY_REQUESTS);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "rate_limited");
}

// ========== Readiness Gating ==========

#[tokio::test]
async fn readyz_returns_200_when_ready() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());
    let resp = server.get("/readyz").await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn readyz_returns_503_when_not_ready() {
    let mock = Server::new_async().await;
    let client = Arc::new(EntraClient::with_base_url(
        "test-tenant".into(),
        "test-client".into(),
        "test-secret".into(),
        &mock.url(),
        3,
    ));
    let provider = EntraProvider::new(client, MAX_PAGE_SIZE);
    let state = AppState {
        provider: Arc::new(provider),
        ready: Arc::new(AtomicBool::new(false)),
        permission_checker: None,
        access: Arc::new(crate::access::AccessLog::new(false, Vec::new())),
        forward_headers: Arc::new(crate::config::ForwardHeaders(Vec::new())),
    };
    let server = TestServer::new(router(state)).expect("failed to create test server");
    let resp = server.get("/readyz").await;
    resp.assert_status(StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn healthz_always_returns_200() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());
    let resp = server.get("/healthz").await;
    resp.assert_status_ok();
}

// ========== Permission Checks ==========

fn build_test_server_with_permissions(graph_url: &str, perm_url: &str) -> TestServer {
    let client = Arc::new(EntraClient::with_base_url(
        "test-tenant".into(),
        "test-client".into(),
        "test-secret".into(),
        graph_url,
        3,
    ));
    let provider = EntraProvider::new(client, MAX_PAGE_SIZE);
    let checker = PermissionChecker::new(PermissionConfig {
        url: perm_url.to_string(),
        service_name: "test-service".to_string(),
        cache_ttl: Duration::from_secs(60),
        cache_max_capacity: 100,
    });
    let state = AppState {
        provider: Arc::new(provider),
        ready: Arc::new(AtomicBool::new(true)),
        permission_checker: Some(Arc::new(checker)),
        access: Arc::new(crate::access::AccessLog::new(false, Vec::new())),
        forward_headers: Arc::new(crate::config::ForwardHeaders(Vec::new())),
    };
    TestServer::new(router(state)).expect("failed to create test server")
}

async fn mock_permission_allow(server: &mut Server) -> mockito::Mock {
    server
        .mock("POST", "/v1beta/authorization/")
        .with_body(json!({"decision": "allow"}).to_string())
        .create_async()
        .await
}

async fn mock_permission_deny(server: &mut Server) -> mockito::Mock {
    server
        .mock("POST", "/v1beta/authorization/")
        .with_body(json!({"decision": "deny"}).to_string())
        .create_async()
        .await
}

async fn mock_permission_batch(
    server: &mut Server,
    action: &str,
    decisions: &[&str],
) -> mockito::Mock {
    let decisions_json: Vec<serde_json::Value> = decisions
        .iter()
        .map(|d| {
            let mut map = serde_json::Map::new();
            map.insert(action.to_string(), json!({"decision": d}));
            serde_json::Value::Object(map)
        })
        .collect();
    server
        .mock("POST", "/v1beta/authorization/batch/")
        .with_body(json!({"decisions": decisions_json}).to_string())
        .create_async()
        .await
}

#[tokio::test]
async fn permission_allows_list_users() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _users = graph
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .create_async()
        .await;
    let _allow = mock_permission_allow(&mut perm).await;
    let _batch = mock_permission_batch(&mut perm, "get-user", &["allow"]).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u1");
}

#[tokio::test]
async fn permission_denies_list_users() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _deny = mock_permission_deny(&mut perm).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status(StatusCode::FORBIDDEN);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "forbidden");
}

#[tokio::test]
async fn permission_denies_get_user() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _deny = mock_permission_deny(&mut perm).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users/uid-42")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn permission_missing_bearer_returns_401() {
    let graph = Server::new_async().await;
    let perm = Server::new_async().await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn permission_service_401_returns_401() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _unauth = perm
        .mock("POST", "/v1beta/authorization/")
        .with_status(401)
        .with_body("invalid token")
        .create_async()
        .await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer bad-token")
        .await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn permission_service_error_returns_500() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _err = perm
        .mock("POST", "/v1beta/authorization/")
        .with_status(500)
        .with_body("internal error")
        .create_async()
        .await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn get_current_user_exempt_from_permission_check() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _user = graph
        .mock("GET", "/users/me-id")
        .match_query(Matcher::Any)
        .with_body(graph_user_json("me-id").to_string())
        .create_async()
        .await;
    let perm_mock = perm
        .mock("POST", "/v1beta/authorization/")
        .expect(0)
        .create_async()
        .await;

    let bearer = jwt_with_oid("me-id");
    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users/me")
        .add_header("Authorization", format!("Bearer {bearer}"))
        .await;
    resp.assert_status_ok();
    let user: User = resp.json();
    assert_eq!(user.id, "me-id");
    perm_mock.assert_async().await;
}

#[tokio::test]
async fn permission_check_forwards_bearer_token() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let perm_mock = perm
        .mock("POST", "/v1beta/authorization/")
        .match_header("authorization", "Bearer my-jwt-token")
        .with_body(json!({"decision": "deny"}).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer my-jwt-token")
        .await;
    perm_mock.assert_async().await;
}

#[tokio::test]
async fn permission_check_includes_resource_for_get_user() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _user = graph
        .mock("GET", "/users/uid-42")
        .match_query(Matcher::Any)
        .with_body(graph_user_json("uid-42").to_string())
        .create_async()
        .await;
    let perm_mock = perm
        .mock("POST", "/v1beta/authorization/")
        .match_body(Matcher::Json(json!({
            "action": { "name": "get-user", "service": "test-service" },
            "resource": { "id": "uid-42", "type": "User" }
        })))
        .with_body(json!({"decision": "allow"}).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users/uid-42")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    perm_mock.assert_async().await;
}

#[tokio::test]
async fn list_users_filters_denied_items() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _users = graph
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(
            json!({
                "value": [
                    graph_user_json("u1"),
                    graph_user_json("u2"),
                    graph_user_json("u3"),
                ]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _allow = mock_permission_allow(&mut perm).await;
    let _batch = mock_permission_batch(&mut perm, "get-user", &["allow", "deny", "allow"]).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, "u1");
    assert_eq!(page.items[1].id, "u3");
}

#[tokio::test]
async fn permission_allows_list_group_members() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _members = graph
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .create_async()
        .await;
    let _allow = mock_permission_allow(&mut perm).await;
    let _batch = mock_permission_batch(&mut perm, "get-user", &["allow"]).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/groups/gid-1/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_user(&page.items[0], "u1");
}

#[tokio::test]
async fn permission_denies_list_group_members() {
    let graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _deny = mock_permission_deny(&mut perm).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/groups/gid-1/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_group_members_filters_denied_items() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _members = graph
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(
            json!({
                "value": [
                    graph_user_json("u1"),
                    graph_user_json("u2"),
                    graph_user_json("u3"),
                ]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _allow = mock_permission_allow(&mut perm).await;
    let _batch = mock_permission_batch(&mut perm, "get-user", &["allow", "deny", "allow"]).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/groups/gid-1/users")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    let page: Page<User> = resp.json();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id, "u1");
    assert_eq!(page.items[1].id, "u3");
}

#[tokio::test]
async fn list_groups_filters_denied_items() {
    let mut graph = Server::new_async().await;
    let mut perm = Server::new_async().await;
    let _token = mock_token(&mut graph).await;
    let _groups = graph
        .mock("GET", "/groups")
        .match_query(Matcher::Any)
        .with_body(
            json!({
                "value": [
                    graph_group_json("g1"),
                    graph_group_json("g2"),
                ]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _allow = mock_permission_allow(&mut perm).await;
    let _batch = mock_permission_batch(&mut perm, "get-group", &["deny", "allow"]).await;

    let server = build_test_server_with_permissions(&graph.url(), &perm.url());
    let resp = server
        .get("/api/v1alpha/groups")
        .add_header("Authorization", "Bearer test-jwt")
        .await;
    resp.assert_status_ok();
    let page: Page<Group> = resp.json();
    assert_eq!(page.items.len(), 1);
    assert_sample_group(&page.items[0], "g2");
}

// ========== Upstream-injection hardening ==========

/// The `group_id` from the URL is embedded into an OData `$filter` expression.
/// Single quotes must be doubled so an attacker cannot terminate the literal
/// and inject additional predicates against Graph.
#[tokio::test]
async fn get_user_group_escapes_single_quotes_in_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let filter_mock = mock
        .mock("GET", "/users/uid-1/memberOf/microsoft.graph.group")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "id eq 'g'' or startswith(id,'''')'' or '''".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server
        .get(
            "/api/v1alpha/users/uid-1/groups/g%27%20or%20startswith%28id%2C%27%27%29%27%20or%20%27",
        )
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
    filter_mock.assert_async().await;
}

#[tokio::test]
async fn get_group_member_escapes_single_quotes_in_filter() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let filter_mock = mock
        .mock("GET", "/groups/gid-1/members/microsoft.graph.user")
        .match_query(Matcher::UrlEncoded(
            "$filter".into(),
            "id eq 'x'' or ''a'' eq ''a'".into(),
        ))
        .with_body(json!({ "value": [] }).to_string())
        .expect(1)
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server
        .get("/api/v1alpha/groups/gid-1/users/x%27%20or%20%27a%27%20eq%20%27a")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
    filter_mock.assert_async().await;
}

// ========== Error surface sanitization ==========

#[tokio::test]
async fn graph_5xx_upstream_body_is_not_leaked_to_client() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body("graph-internal-correlation-id-deadbeef")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users").await;
    resp.assert_status(StatusCode::BAD_GATEWAY);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "bad_gateway");
    assert!(
        !body
            .message
            .contains("graph-internal-correlation-id-deadbeef"),
        "client message leaked upstream body: {}",
        body.message
    );
    assert!(
        !body.message.contains("Graph API"),
        "client message leaked provider context: {}",
        body.message
    );
    assert!(
        !body.message.contains("500"),
        "client message leaked upstream status code: {}",
        body.message
    );
}

#[tokio::test]
async fn graph_403_upstream_body_is_not_leaked_to_client() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/uid-1")
        .match_query(Matcher::Any)
        .with_status(403)
        .with_body("Insufficient privileges: Authorization_RequestDenied")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1").await;
    resp.assert_status(StatusCode::FORBIDDEN);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "forbidden");
    assert!(
        !body.message.contains("Authorization_RequestDenied"),
        "client message leaked upstream body: {}",
        body.message
    );
    assert!(
        !body.message.contains("Insufficient privileges"),
        "client message leaked upstream body: {}",
        body.message
    );
}

#[tokio::test]
async fn graph_401_upstream_body_is_not_leaked_to_client() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/uid-1")
        .match_query(Matcher::Any)
        .with_status(401)
        .with_body("tenant-id-secret-leak InvalidAuthenticationToken")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/uid-1").await;
    resp.assert_status(StatusCode::UNAUTHORIZED);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "unauthorized");
    assert!(
        !body.message.contains("tenant-id-secret-leak"),
        "client message leaked upstream body: {}",
        body.message
    );
    assert!(
        !body.message.contains("InvalidAuthenticationToken"),
        "client message leaked upstream body: {}",
        body.message
    );
}

#[tokio::test]
async fn graph_404_upstream_body_is_not_leaked_to_client() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _user = mock
        .mock("GET", "/users/nonexistent")
        .match_query(Matcher::Any)
        .with_status(404)
        .with_body("x-ms-correlation-request-id: 11111111 Request_ResourceNotFound")
        .create_async()
        .await;

    let server = build_test_server(&mock.url());
    let resp = server.get("/api/v1alpha/users/nonexistent").await;
    resp.assert_status(StatusCode::NOT_FOUND);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "not_found");
    assert!(
        !body.message.contains("x-ms-correlation-request-id"),
        "client message leaked upstream correlation id: {}",
        body.message
    );
    assert!(
        !body.message.contains("11111111"),
        "client message leaked upstream correlation id: {}",
        body.message
    );
}

#[tokio::test]
async fn invalid_cursor_details_are_not_leaked_to_client() {
    let mock = Server::new_async().await;
    let server = build_test_server(&mock.url());

    let resp = server.get("/api/v1alpha/users?cursor=!!!invalid!!!").await;
    resp.assert_status(StatusCode::BAD_REQUEST);
    let body: ErrorResponse = resp.json();
    assert_eq!(body.error, "bad_request");
    assert!(
        !body.message.contains("invalid cursor"),
        "client message leaked internal detail: {}",
        body.message
    );
    assert!(
        !body.message.to_ascii_lowercase().contains("base64"),
        "client message leaked internal detail: {}",
        body.message
    );
}

// ========== app-role assignments (enterprise-application scoping) ==========

fn app_role_source(
    mock_url: &str,
    service_principal_object_id: Option<&str>,
    include_service_principals: bool,
) -> EntraAppRoleAssignments {
    let client = Arc::new(EntraClient::with_base_url(
        "test-tenant".into(),
        "app-123".into(),
        "test-secret".into(),
        mock_url,
        0,
    ));
    EntraAppRoleAssignments::new(
        client,
        service_principal_object_id.map(str::to_string),
        "app-123".into(),
        include_service_principals,
        MAX_PAGE_SIZE,
    )
}

#[tokio::test]
async fn app_role_assignments_partitions_principals_and_scopes_groups() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;

    let _assigned = mock
        .mock("GET", "/servicePrincipals/sp-1/appRoleAssignedTo")
        .match_query(Matcher::Any)
        .with_body(
            json!({ "value": [
                { "principalId": "g1", "principalDisplayName": "Admins", "principalType": "Group" },
                { "principalId": "spx", "principalDisplayName": "Bot", "principalType": "ServicePrincipal" },
                { "principalId": "u9", "principalDisplayName": "Real User", "principalType": "User" },
            ] })
            .to_string(),
        )
        .create_async()
        .await;
    let _group = mock
        .mock("GET", "/groups/g1")
        .match_query(Matcher::Any)
        .with_body(graph_group_json("g1").to_string())
        .create_async()
        .await;
    let _members = mock
        .mock("GET", "/groups/g1/members/microsoft.graph.user")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1"), graph_user_json("u2")] }).to_string())
        .create_async()
        .await;

    let scope = app_role_source(&mock.url(), Some("sp-1"), true)
        .fetch()
        .await
        .expect("fetch succeeds");

    assert_eq!(scope.groups.len(), 1);
    assert_sample_group(&scope.groups[0], "g1");

    let members = scope.members_by_group.get("g1").expect("g1 members");
    assert_eq!(members.len(), 2);
    assert!(members.contains("u1") && members.contains("u2"));

    // The user assignment (u9) is ignored; only the service principal surfaces.
    assert_eq!(scope.service_principals.len(), 1);
    let sp = &scope.service_principals[0];
    assert_eq!(sp.id, "spx");
    assert_eq!(sp.username, "spx");
    assert_eq!(sp.display_name, "Bot");
    assert_eq!(sp.active, Some(true));
    assert!(sp.email.is_none());
}

#[tokio::test]
async fn app_role_assignments_omits_service_principals_when_disabled() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _assigned = mock
        .mock("GET", "/servicePrincipals/sp-1/appRoleAssignedTo")
        .match_query(Matcher::Any)
        .with_body(
            json!({ "value": [
                { "principalId": "spx", "principalDisplayName": "Bot", "principalType": "ServicePrincipal" },
            ] })
            .to_string(),
        )
        .create_async()
        .await;

    let scope = app_role_source(&mock.url(), Some("sp-1"), false)
        .fetch()
        .await
        .expect("fetch succeeds");

    assert!(scope.groups.is_empty());
    assert!(scope.service_principals.is_empty());
}

#[tokio::test]
async fn app_role_assignments_probe_fails_closed_on_403() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _assigned = mock
        .mock("GET", "/servicePrincipals/sp-1/appRoleAssignedTo")
        .match_query(Matcher::Any)
        .with_status(403)
        .with_body(
            json!({ "error": { "code": "Authorization_RequestDenied", "message": "Insufficient privileges." } })
                .to_string(),
        )
        .create_async()
        .await;

    let err = app_role_source(&mock.url(), Some("sp-1"), true)
        .probe()
        .await
        .expect_err("probe fails closed on 403");
    assert!(
        matches!(err, ProviderError::Forbidden(_)),
        "expected Forbidden, got {err:?}"
    );
}

#[tokio::test]
async fn app_role_assignments_resolves_own_service_principal_when_unset() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _resolve = mock
        .mock(
            "GET",
            Matcher::Regex(r"servicePrincipals\(appId=".to_string()),
        )
        .match_query(Matcher::Any)
        .with_body(json!({ "id": "sp-own" }).to_string())
        .create_async()
        .await;
    let _assigned = mock
        .mock("GET", "/servicePrincipals/sp-own/appRoleAssignedTo")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [] }).to_string())
        .create_async()
        .await;

    let scope = app_role_source(&mock.url(), None, true)
        .fetch()
        .await
        .expect("fetch resolves own service principal");
    assert!(scope.groups.is_empty());
    assert!(scope.service_principals.is_empty());
}

#[derive(Clone, Default)]
struct AccessCapture(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for AccessCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for AccessCapture {
    type Writer = AccessCapture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn access_subscriber(capture: AccessCapture) -> impl tracing::Subscriber + Send + Sync {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(capture)
        .with_filter(tracing_subscriber::filter::filter_fn(|meta| {
            meta.target() == "access"
        }));
    tracing_subscriber::registry().with(layer)
}

fn access_records(capture: &AccessCapture) -> Vec<serde_json::Value> {
    let bytes = capture.0.lock().unwrap().clone();
    let text = String::from_utf8(bytes).expect("access log output is UTF-8");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let outer: serde_json::Value =
                serde_json::from_str(line).expect("access log line is JSON");
            let access = outer["fields"]["access"]
                .as_str()
                .expect("access field is a string");
            serde_json::from_str::<serde_json::Value>(access).expect("access field is JSON")
        })
        .collect()
}

fn build_access_test_server(mock_url: &str) -> TestServer {
    let client = Arc::new(EntraClient::with_base_url(
        "test-tenant".into(),
        "test-client".into(),
        "test-secret".into(),
        mock_url,
        3,
    ));
    let provider = EntraProvider::new(client, MAX_PAGE_SIZE);
    let state = AppState {
        provider: Arc::new(provider),
        ready: Arc::new(AtomicBool::new(true)),
        permission_checker: None,
        access: Arc::new(crate::access::AccessLog::new(true, Vec::new())),
        forward_headers: Arc::new(crate::config::ForwardHeaders(Vec::new())),
    };
    TestServer::new(router(state)).expect("failed to create test server")
}

#[tokio::test]
async fn access_log_business_call_is_combined() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;
    let _users = mock
        .mock("GET", "/users")
        .match_query(Matcher::Any)
        .with_body(json!({ "value": [graph_user_json("u1")] }).to_string())
        .create_async()
        .await;

    let capture = AccessCapture::default();
    {
        let _guard = tracing::subscriber::set_default(access_subscriber(capture.clone()));
        let server = build_access_test_server(&mock.url());
        server
            .get("/api/v1alpha/users?email=jane@example.com")
            .await
            .assert_status_ok();
    }

    let records = access_records(&capture);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["transport"], "REST");
    assert_eq!(record["operation"], "GET /api/v1alpha/users");
    assert_eq!(record["request"]["params"]["email"], "jane@example.com");
    assert_eq!(record["response"]["status"], 200);
    assert_eq!(record["response"]["detail"]["item_count"], 1);
}

#[tokio::test]
async fn access_log_unauthenticated_is_call_level() {
    let mut mock = Server::new_async().await;
    let _token = mock_token(&mut mock).await;

    let capture = AccessCapture::default();
    {
        let _guard = tracing::subscriber::set_default(access_subscriber(capture.clone()));
        let server = build_access_test_server(&mock.url());
        server
            .get("/api/v1alpha/users/me")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    let records = access_records(&capture);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["transport"], "REST");
    assert_eq!(record["operation"], "GET /api/v1alpha/users/me");
    assert!(record.get("request").is_none());
    assert_eq!(record["response"]["status"], 401);
}

#[tokio::test]
async fn access_log_unknown_route_is_call_level() {
    let capture = AccessCapture::default();
    {
        let _guard = tracing::subscriber::set_default(access_subscriber(capture.clone()));
        let server = build_access_test_server("http://unused.example");
        server.get("/no/such/path").await.assert_status_not_found();
    }

    let records = access_records(&capture);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["transport"], "REST");
    assert_eq!(record["operation"], "GET /no/such/path");
    assert!(record.get("request").is_none());
    assert_eq!(record["response"]["status"], 404);
}

#[tokio::test]
async fn access_log_infra_probe_is_not_recorded() {
    let capture = AccessCapture::default();
    {
        let _guard = tracing::subscriber::set_default(access_subscriber(capture.clone()));
        let server = build_access_test_server("http://unused.example");
        server.get("/healthz").await.assert_status_ok();
    }

    assert!(access_records(&capture).is_empty());
}
