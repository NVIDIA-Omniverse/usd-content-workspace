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

use super::access_token_file::AccessTokenFileProvider;
use super::client_credentials::{ClientCredentialsArgs, Token, TokenManager};
use super::TokenRefresher;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::Duration;
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn access_token_file_json(access_token: &str) -> String {
    format!(
        r#"{{"access_token":"{}","expires_on":"2999-01-01T00:00:00Z"}}"#,
        access_token
    )
}

#[tokio::test]
async fn test_token_creation_and_expiry() {
    let token = Token::new("test_token".to_string(), 1);

    assert!(!token.is_expired());
    assert_eq!(token.access_token, "test_token");

    let token = Token::new("test_token".to_string(), 0);
    assert!(token.is_expired());
}

#[tokio::test]
async fn test_token_expiring_soon() {
    let token = Token::new("test_token".to_string(), 300);
    assert!(!token.is_expiring_soon());

    let token = Token::new("test_token".to_string(), 3);
    assert!(token.is_expiring_soon());
}

#[tokio::test]
async fn test_token_exchange_success() {
    let mock_response = json!({
        "access_token": "mock_access_token",
        "expires_in": 3600,
        "token_type": "Bearer"
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string("grant_type=client_credentials&client_id=test_client_id&client_secret=test_client_secret&scope=test_scope"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(&mock_response),
        )
        .mount(&server)
        .await;

    let client = Arc::new(reqwest::Client::new());
    let token_manager = TokenManager::new(
        client,
        "test_client_id".to_string(),
        "test_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        HashMap::new(),
        HashMap::new(),
    );

    let token_handle = token_manager.get_token_handle();
    let token_received = token_manager.token_received.clone();

    let refresh_handle = tokio::spawn(async move {
        token_manager.start_refresh_loop().await;
    });
    token_received.notified().await;

    let token_guard = token_handle.lock().unwrap();
    assert!(token_guard.is_some());

    let token = token_guard.as_ref().unwrap();
    assert_eq!(token.access_token, "mock_access_token");
    assert!(!token.is_expired());

    refresh_handle.abort();
}

#[tokio::test]
async fn test_token_exchange_failure() {
    let mock_error = json!({
        "error": "invalid_client",
        "error_description": "Client authentication failed"
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string("grant_type=client_credentials&client_id=invalid_client_id&client_secret=invalid_client_secret&scope=test_scope"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("content-type", "application/json")
                .set_body_json(&mock_error),
        )
        .mount(&server)
        .await;

    let client = Arc::new(reqwest::Client::new());
    let token_manager = TokenManager::new(
        client,
        "invalid_client_id".to_string(),
        "invalid_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        HashMap::new(),
        HashMap::new(),
    );

    let token_handle = token_manager.get_token_handle();
    let token_failure = token_manager.token_failure.clone();

    let refresh_handle = tokio::spawn(async move {
        token_manager.start_refresh_loop().await;
    });
    token_failure.notified().await;

    let token_guard = token_handle.lock().unwrap();
    assert!(token_guard.is_none());

    refresh_handle.abort();
}

#[tokio::test]
async fn test_token_refresh_loop_with_expiring_token() {
    let server = MockServer::start().await;

    let client = Arc::new(reqwest::Client::new());
    let mut token_manager = TokenManager::new(
        client,
        "test_client_id".to_string(),
        "test_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        HashMap::new(),
        HashMap::new(),
    );
    token_manager.refresh_interval = Duration::from_secs(1);

    let token_handle = token_manager.get_token_handle();
    let token_received = token_manager.token_received.clone();
    let refresh_handle = tokio::spawn(async move {
        token_manager.start_refresh_loop().await;
    });

    // Verify initial token
    {
        let mock_response = json!({
            "access_token": "mock_access_token",
            "expires_in": 1,
            "token_type": "Bearer"
        });

        let _guard = Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string("grant_type=client_credentials&client_id=test_client_id&client_secret=test_client_secret&scope=test_scope"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(&mock_response),
            )
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        token_received.notified().await;
        let token_guard = token_handle.lock().unwrap();
        assert!(token_guard.is_some());
        assert_eq!(
            token_guard.as_ref().unwrap().access_token,
            "mock_access_token"
        );
    }

    // Verify token was refreshed
    {
        let mock_response = json!({
            "access_token": "mock_access_token2",
            "expires_in": 1,
            "token_type": "Bearer"
        });

        let _guard = Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string("grant_type=client_credentials&client_id=test_client_id&client_secret=test_client_secret&scope=test_scope"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(&mock_response),
            )
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        token_received.notified().await;
        let token_guard = token_handle.lock().unwrap();
        assert!(token_guard.is_some());
        assert_eq!(
            token_guard.as_ref().unwrap().access_token,
            "mock_access_token2"
        );
    }

    refresh_handle.abort();
}

#[tokio::test]
async fn test_token_exchange_with_extra_body_params() {
    let mock_response = json!({
        "access_token": "mock_access_token",
        "expires_in": 3600,
        "token_type": "Bearer"
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string("grant_type=client_credentials&client_id=test_client_id&client_secret=test_client_secret&scope=test_scope&audience=https%3A%2F%2Fapi.example.com"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(&mock_response),
        )
        .mount(&server)
        .await;

    let client = Arc::new(reqwest::Client::new());
    let extra_body_params = HashMap::from([(
        "audience".to_string(),
        "https://api.example.com".to_string(),
    )]);
    let token_manager = TokenManager::new(
        client,
        "test_client_id".to_string(),
        "test_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        HashMap::new(),
        extra_body_params,
    );

    let token_handle = token_manager.get_token_handle();
    let token_received = token_manager.token_received.clone();

    let refresh_handle = tokio::spawn(async move {
        token_manager.start_refresh_loop().await;
    });
    token_received.notified().await;

    let token_guard = token_handle.lock().unwrap();
    assert!(token_guard.is_some());
    assert_eq!(
        token_guard.as_ref().unwrap().access_token,
        "mock_access_token"
    );

    refresh_handle.abort();
}

#[tokio::test]
async fn test_token_exchange_with_extra_headers() {
    let mock_response = json!({
        "access_token": "mock_access_token",
        "expires_in": 3600,
        "token_type": "Bearer"
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("X-Custom", "test-value"))
        .and(body_string("grant_type=client_credentials&client_id=test_client_id&client_secret=test_client_secret&scope=test_scope"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(&mock_response),
        )
        .mount(&server)
        .await;

    let client = Arc::new(reqwest::Client::new());
    let extra_headers = HashMap::from([("X-Custom".to_string(), "test-value".to_string())]);
    let token_manager = TokenManager::new(
        client,
        "test_client_id".to_string(),
        "test_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        extra_headers,
        HashMap::new(),
    );

    let token_handle = token_manager.get_token_handle();
    let token_received = token_manager.token_received.clone();

    let refresh_handle = tokio::spawn(async move {
        token_manager.start_refresh_loop().await;
    });
    token_received.notified().await;

    let token_guard = token_handle.lock().unwrap();
    assert!(token_guard.is_some());
    assert_eq!(
        token_guard.as_ref().unwrap().access_token,
        "mock_access_token"
    );

    refresh_handle.abort();
}

#[tokio::test]
async fn test_get_extra_headers_invalid_json() {
    let args = ClientCredentialsArgs {
        enabled: false,
        openid_configuration: String::new(),
        client_id: String::new(),
        client_secret: None,
        client_secret_file: None,
        client_scope: String::new(),
        extra_headers: "not-valid-json".to_string(),
        extra_body_params: "{}".to_string(),
    };

    let result = args.get_extra_headers();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("CLIENT_CREDENTIALS_EXTRA_HEADERS"));
    assert!(err_msg.contains("not-valid-json"));
}

#[tokio::test]
async fn test_get_extra_body_params_invalid_json() {
    let args = ClientCredentialsArgs {
        enabled: false,
        openid_configuration: String::new(),
        client_id: String::new(),
        client_secret: None,
        client_secret_file: None,
        client_scope: String::new(),
        extra_headers: "{}".to_string(),
        extra_body_params: "[1,2,3]".to_string(),
    };

    let result = args.get_extra_body_params();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS"));
}

#[tokio::test]
async fn test_access_token_file_provider_get_token() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(&file_path, access_token_file_json("my-test-token")).unwrap();

    let provider = AccessTokenFileProvider::new(file_path.clone(), Duration::from_secs(15));
    provider.load().await;
    let token = provider.get_token();
    assert_eq!(token, Some("my-test-token".to_string()));

    let token2 = provider.get_token();
    assert_eq!(token2, Some("my-test-token".to_string()));
}

#[tokio::test]
async fn test_access_token_file_provider_invalidate_and_reread() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(&file_path, access_token_file_json("token-v1")).unwrap();

    let provider = AccessTokenFileProvider::new(file_path.clone(), Duration::from_secs(15));
    provider.load().await;
    assert_eq!(provider.get_token(), Some("token-v1".to_string()));

    std::fs::write(&file_path, access_token_file_json("token-v2")).unwrap();
    assert_eq!(provider.get_token(), Some("token-v1".to_string()));

    provider.invalidate();
    assert_eq!(provider.get_token(), None);

    provider.load().await;
    assert_eq!(provider.get_token(), Some("token-v2".to_string()));
}

#[tokio::test]
async fn test_access_token_file_provider_missing_file() {
    let provider = AccessTokenFileProvider::new(
        std::path::PathBuf::from("/nonexistent/path/to/token"),
        Duration::from_secs(15),
    );
    provider.load().await;
    assert_eq!(provider.get_token(), None);
}

#[tokio::test]
async fn test_access_token_file_provider_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(&file_path, "  \n").unwrap();

    let provider = AccessTokenFileProvider::new(file_path, Duration::from_secs(15));
    provider.load().await;
    assert_eq!(provider.get_token(), None);
}

#[tokio::test]
async fn test_access_token_file_provider_expired_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(
        &file_path,
        r#"{"access_token":"expired-token","expires_on":"2000-01-01T00:00:00Z"}"#,
    )
    .unwrap();

    let provider = AccessTokenFileProvider::new(file_path, Duration::from_secs(15));
    provider.load().await;
    assert_eq!(provider.get_token(), None);
}

fn access_token_file_json_with_expiry(
    access_token: &str,
    expires_at: time::OffsetDateTime,
) -> String {
    let (year, month, day) = expires_at.to_calendar_date();
    let (hour, minute, second) = expires_at.to_hms();
    let expires_on = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        u8::from(month),
        day,
        hour,
        minute,
        second
    );
    format!(
        r#"{{"access_token":"{}","expires_on":"{}"}}"#,
        access_token, expires_on
    )
}

#[tokio::test]
async fn test_access_token_file_provider_expiring_soon() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");

    let contents = access_token_file_json_with_expiry(
        "expiring-soon-token",
        time::OffsetDateTime::now_utc() + time::Duration::seconds(60),
    );
    std::fs::write(&file_path, contents).unwrap();

    let provider = AccessTokenFileProvider::new(file_path, Duration::from_secs(15));
    provider.load().await;
    assert_eq!(provider.get_token(), None);
}

#[tokio::test]
async fn test_access_token_file_provider_cached_token_expires() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");

    let contents = access_token_file_json_with_expiry(
        "short-lived-token",
        time::OffsetDateTime::now_utc() + time::Duration::seconds(1),
    );
    std::fs::write(&file_path, contents).unwrap();

    let provider = AccessTokenFileProvider::new(file_path, Duration::from_secs(15));
    provider.load().await;

    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(provider.get_token(), None);
}

#[tokio::test]
async fn test_access_token_file_token_refresher_impl() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(&file_path, access_token_file_json("original-token")).unwrap();

    let provider = Arc::new(AccessTokenFileProvider::new(
        file_path.clone(),
        Duration::from_secs(15),
    ));
    provider.load().await;
    assert_eq!(provider.get_token(), Some("original-token".to_string()));

    std::fs::write(&file_path, access_token_file_json("refreshed-token")).unwrap();

    let refresher: Arc<dyn TokenRefresher> = provider.clone();
    refresher.refresh().await;

    assert_eq!(provider.get_token(), Some("refreshed-token".to_string()));
}

#[tokio::test]
async fn test_access_token_file_watch_loop_detects_change() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("token");
    std::fs::write(&file_path, access_token_file_json("initial-token")).unwrap();

    let provider = Arc::new(AccessTokenFileProvider::new(
        file_path.clone(),
        Duration::from_millis(20),
    ));

    provider.load().await;
    assert_eq!(provider.get_token(), Some("initial-token".to_string()));

    let watcher = provider.clone();
    let handle = tokio::spawn(async move {
        watcher.start_watch_loop().await;
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline
        && provider.get_token().as_deref() != Some("updated-token")
    {
        std::fs::write(&file_path, access_token_file_json("updated-token")).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(provider.get_token(), Some("updated-token".to_string()));

    handle.abort();
}

#[tokio::test]
async fn test_token_manager_token_refresher_impl() {
    let mock_response = json!({
        "access_token": "refreshed_token",
        "expires_in": 3600,
        "token_type": "Bearer"
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(&mock_response),
        )
        .mount(&server)
        .await;

    let client = Arc::new(reqwest::Client::new());
    let token_manager = Arc::new(TokenManager::new(
        client,
        "test_client_id".to_string(),
        "test_client_secret".to_string(),
        "test_scope".to_string(),
        format!("{}/token", server.uri()),
        HashMap::new(),
        HashMap::new(),
    ));

    let token_handle = token_manager.get_token_handle();
    assert!(token_handle.lock().unwrap().is_none());

    let refresher: Arc<dyn TokenRefresher> = token_manager.clone();
    refresher.refresh().await;

    let guard = token_handle.lock().unwrap();
    assert!(guard.is_some());
    assert_eq!(guard.as_ref().unwrap().access_token, "refreshed_token");
}
