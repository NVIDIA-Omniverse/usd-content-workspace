// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

// Integration tests for GET /api/v2beta/services (and its /api/v2/services alias) using an in-process mock HTTP server.
//
// Run with:
//   cargo nextest run --test rest_api

use auto_discovery_service::api::rest;
use auto_discovery_service::state::{
    CapabilityEntry, GrpcEndpointEntry, RestEndpointEntry, RestVersionEntry, ServiceInfo,
    SharedState,
};
use auto_discovery_service::v1::V1EndpointMapping;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

const AUTH_CONFIG_PATHS: [&str; 4] = [
    "/api/v1/auth-config",
    "/api/v1alpha/auth-config",
    "/api/v2/auth-config",
    "/api/v2beta/auth-config",
];

async fn start_mock_server(state: SharedState) -> String {
    start_mock_server_with_options(state, None, None, Arc::new(Vec::new())).await
}

async fn start_mock_server_with_auth_config(
    state: SharedState,
    auth_config_json: Option<String>,
) -> String {
    start_mock_server_with_options(state, auth_config_json, None, Arc::new(Vec::new())).await
}

async fn start_mock_server_with_v1(
    state: SharedState,
    v1_mappings: Arc<Vec<V1EndpointMapping>>,
) -> String {
    start_mock_server_with_options(state, None, None, v1_mappings).await
}

async fn start_mock_server_with_options(
    state: SharedState,
    auth_config_json: Option<String>,
    root_redirect: Option<rest::RootRedirect>,
    v1_mappings: Arc<Vec<V1EndpointMapping>>,
) -> String {
    let app = rest::router(state, auth_config_json, root_redirect, v1_mappings);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("mock server address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock server");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn auth_config_returns_404_when_not_configured() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server(state).await;
    let client = reqwest::Client::new();

    for path in AUTH_CONFIG_PATHS {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn auth_config_returns_404_when_json_is_empty() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server_with_auth_config(state, Some("   ".to_string())).await;
    let client = reqwest::Client::new();

    for path in AUTH_CONFIG_PATHS {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn auth_config_returns_configured_openid_json() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server_with_auth_config(
        state,
        Some(
            r#"{"_comment":"OpenID Authentication","openid_configuration":"https://issuer.example.com/.well-known/openid-configuration","clients":{"client_library":{"client_id":"client-id","scope":"openid profile"}}}"#
                .to_string(),
        ),
    )
    .await;
    let client = reqwest::Client::new();

    for path in AUTH_CONFIG_PATHS {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let body: Value = response.json().await.expect("response json");
        assert_eq!(
            body["openid_configuration"],
            "https://issuer.example.com/.well-known/openid-configuration"
        );
        assert_eq!(body["clients"]["client_library"]["client_id"], "client-id");
        assert_eq!(body["clients"]["client_library"]["scope"], "openid profile");
    }
}

#[tokio::test]
async fn root_redirect_returns_404_when_not_configured() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server(state).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");

    let response = client
        .get(format!("{base_url}/"))
        .send()
        .await
        .expect("GET /");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_redirect_returns_configured_location() {
    let state = auto_discovery_service::state::new_shared_state();
    let redirect = rest::RootRedirect::new("https://navigator.example.com/".to_string(), 302)
        .expect("root redirect");
    let base_url =
        start_mock_server_with_options(state, None, Some(redirect), Arc::new(Vec::new())).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");

    let response = client
        .get(format!("{base_url}/"))
        .send()
        .await
        .expect("GET /");

    assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("https://navigator.example.com/")
    );
}

async fn seed_state(state: &SharedState, services: HashMap<String, ServiceInfo>) {
    let mut guard = state.write().await;
    *guard = services;
}

fn sample_capability_entries() -> Vec<CapabilityEntry> {
    vec![
        CapabilityEntry {
            service_name: "fileobject".to_string(),
            rest: vec![RestEndpointEntry {
                name: "fileobject".to_string(),
                versions: vec![
                    RestVersionEntry {
                        version: "v1".to_string(),
                        path: Some("/api/v1/fileobject".to_string()),
                    },
                    RestVersionEntry {
                        version: "v1alpha".to_string(),
                        path: Some("/api/v1alpha/fileobject".to_string()),
                    },
                ],
            }],
            grpc: vec![
                GrpcEndpointEntry {
                    service: "nvidia.omniverse.storage.fileobject.v1.FileObjectService".to_string(),
                },
                GrpcEndpointEntry {
                    service: "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
                        .to_string(),
                },
            ],
        },
        CapabilityEntry {
            service_name: "filefolder".to_string(),
            rest: vec![],
            grpc: vec![
                GrpcEndpointEntry {
                    service: "nvidia.omniverse.storage.filefolder.v1.FileFolderService".to_string(),
                },
                GrpcEndpointEntry {
                    service: "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService"
                        .to_string(),
                },
            ],
        },
    ]
}

fn sample_service_info() -> ServiceInfo {
    ServiceInfo::from_capabilities(
        "storage-service".to_string(),
        Some("https://storage.example.com".to_string()),
        Some("https://grpc.storage.example.com".to_string()),
        sample_capability_entries(),
    )
}

#[tokio::test]
async fn list_services_returns_empty_array_when_state_is_empty() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server(state).await;

    let response = reqwest::Client::new()
        .get(format!("{base_url}/api/v2beta/services"))
        .send()
        .await
        .expect("GET /api/v2beta/services");

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.expect("response json");
    assert_eq!(body["schema-version"], 2);
    assert_eq!(body["services"], Value::Array(vec![]));
}

#[tokio::test]
async fn list_services_returns_rest_and_grpc_blocks() {
    let state = auto_discovery_service::state::new_shared_state();
    seed_state(
        &state,
        HashMap::from([("storage-service".to_string(), sample_service_info())]),
    )
    .await;
    let base_url = start_mock_server(state).await;

    let response = reqwest::Client::new()
        .get(format!("{base_url}/api/v2beta/services"))
        .send()
        .await
        .expect("GET /api/v2beta/services");

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.expect("response json");
    assert_eq!(body["schema-version"], 2);

    let service = &body["services"][0];
    assert_eq!(service["name"], "storage-service");
    assert_eq!(service["capability_source"], "grpc");
    assert_eq!(service["rest"]["url"], "https://storage.example.com");
    assert_eq!(service["rest"]["endpoints"][0]["name"], "fileobject");
    assert_eq!(
        service["rest"]["endpoints"][0]["versions"]["v1"]["path"],
        "/api/v1/fileobject"
    );
    assert_eq!(
        service["rest"]["endpoints"][0]["versions"]["v1alpha"]["path"],
        "/api/v1alpha/fileobject"
    );
    assert_eq!(service["grpc"]["url"], "https://grpc.storage.example.com");
    assert_eq!(
        service["grpc"]["endpoints"][0]["service"],
        "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
    );
    assert_eq!(
        service["grpc"]["endpoints"][1]["service"],
        "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService"
    );
    assert_eq!(
        service["grpc"]["endpoints"][2]["service"],
        "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
    );
    assert_eq!(
        service["grpc"]["endpoints"][3]["service"],
        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
    );
}

#[tokio::test]
async fn list_services_sets_cache_control_header() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server(state).await;

    let response = reqwest::Client::new()
        .get(format!("{base_url}/api/v2beta/services"))
        .send()
        .await
        .expect("GET /api/v2beta/services");

    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("max-age=30, must-revalidate")
    );
}

#[tokio::test]
async fn list_services_v2_alias_matches_v2beta() {
    let state = auto_discovery_service::state::new_shared_state();
    seed_state(
        &state,
        HashMap::from([("storage-service".to_string(), sample_service_info())]),
    )
    .await;
    let base_url = start_mock_server(state).await;
    let client = reqwest::Client::new();

    let mut bodies = Vec::new();
    for path in ["/api/v2beta/services", "/api/v2/services"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("max-age=30, must-revalidate"),
            "{path}"
        );
        bodies.push(response.json::<Value>().await.expect("response json"));
    }

    assert_eq!(bodies[0]["schema-version"], 2);
    assert_eq!(bodies[0], bodies[1]);
}

fn storage_v1_mappings() -> Arc<Vec<V1EndpointMapping>> {
    Arc::new(
        auto_discovery_service::v1::parse_mappings(
            r#"[
              {
                "name": "Storage Service",
                "idPrefix": "storage-service",
                "type": "storage",
                "endpoints": {
                  "grpc": { "from": ["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"] },
                  "rest": { "from": ["fileobject"] }
                }
              }
            ]"#,
        )
        .expect("parse mappings"),
    )
}

#[tokio::test]
async fn list_services_v1_returns_404_when_disabled() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server(state).await;
    let client = reqwest::Client::new();

    for path in ["/api/v1/services", "/api/v1alpha/services"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn list_services_v1_maps_v2_capabilities() {
    let state = auto_discovery_service::state::new_shared_state();
    seed_state(
        &state,
        HashMap::from([("storage-service".to_string(), sample_service_info())]),
    )
    .await;
    let base_url = start_mock_server_with_v1(state, storage_v1_mappings()).await;
    let client = reqwest::Client::new();

    for path in ["/api/v1/services", "/api/v1alpha/services"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let body: Value = response.json().await.expect("response json");
        assert_eq!(body["schema-version"], 1);

        let service = &body["services"][0];
        assert_eq!(service["id"], "storage-service");
        assert_eq!(service["name"], "Storage Service");
        assert_eq!(service["type"], "storage");
        assert_eq!(service["grpc"], "https://grpc.storage.example.com");
        assert_eq!(service["rest"], "https://storage.example.com");
    }
}

#[tokio::test]
async fn list_services_v1_sets_cache_control_header() {
    let state = auto_discovery_service::state::new_shared_state();
    let base_url = start_mock_server_with_v1(state, storage_v1_mappings()).await;
    let client = reqwest::Client::new();

    for path in ["/api/v1/services", "/api/v1alpha/services"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap_or_else(|_| panic!("GET {path}"));

        assert_eq!(
            response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("max-age=30, must-revalidate")
        );
    }
}
