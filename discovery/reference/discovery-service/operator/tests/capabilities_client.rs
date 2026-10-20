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

// Integration tests against a live GripMock server.
//
// Start the mock server before running:
//   docker compose -f docker-compose.test.yml up -d
//
// Then run:
//   GRIPMOCK_ADDR=http://localhost:4770 cargo nextest run --test capabilities_client

mod proto {
    tonic::include_proto!("nvidia.omniverse.discovery.capabilities.v2beta");
}

use proto::capabilities_service_client::CapabilitiesServiceClient;
use proto::ListServicesRequest;

use auto_discovery_service::client::capabilities;
use auto_discovery_service::state::ServiceInfo;

fn require_gripmock_addr() -> String {
    match std::env::var("GRIPMOCK_ADDR") {
        Ok(addr) if !addr.is_empty() => addr,
        _ => panic!(
            "GRIPMOCK_ADDR is not set. Start GripMock with \
             `docker compose -f docker-compose.test.yml up -d`, then run \
             `GRIPMOCK_ADDR=http://localhost:4770 cargo nextest run --test capabilities_client`."
        ),
    }
}

#[tokio::test]
async fn list_services_returns_stub_services() {
    let addr = require_gripmock_addr();

    let mut client = CapabilitiesServiceClient::connect(addr)
        .await
        .expect("connect to gripmock");

    let services = client
        .list_services(ListServicesRequest::default())
        .await
        .expect("ListServices RPC")
        .into_inner()
        .services;

    assert_eq!(services.len(), 2);

    let names: Vec<&str> = services.iter().map(|s| s.service_name.as_str()).collect();
    assert!(names.contains(&"fileobject"), "expected fileobject");
    assert!(names.contains(&"filefolder"), "expected filefolder");
}

#[tokio::test]
async fn list_services_fileobject_has_grpc_and_rest() {
    let addr = require_gripmock_addr();

    let mut client = CapabilitiesServiceClient::connect(addr)
        .await
        .expect("connect to gripmock");

    let services = client
        .list_services(ListServicesRequest::default())
        .await
        .expect("ListServices RPC")
        .into_inner()
        .services;

    let svc = services
        .iter()
        .find(|s| s.service_name == "fileobject")
        .expect("fileobject service");

    let grpc_service = svc
        .grpc_services
        .first()
        .and_then(|g| g.endpoints.first())
        .map(|e| e.service_name.as_str())
        .expect("grpc endpoint");
    assert_eq!(
        grpc_service,
        "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
    );

    let rest = svc
        .rest_services
        .first()
        .and_then(|r| r.endpoints.first())
        .expect("rest endpoint");
    assert_eq!(rest.name, "fileobject");

    let v1_path = rest
        .versions
        .iter()
        .find(|v| v.version == "v1")
        .and_then(|v| v.url_detail.as_ref())
        .map(|d| d.path.as_str())
        .expect("v1 path");
    assert_eq!(v1_path, "/api/v1/fileobject");
}

#[tokio::test]
async fn list_services_filefolder_has_grpc_only() {
    let addr = require_gripmock_addr();

    let mut client = CapabilitiesServiceClient::connect(addr)
        .await
        .expect("connect to gripmock");

    let services = client
        .list_services(ListServicesRequest::default())
        .await
        .expect("ListServices RPC")
        .into_inner()
        .services;

    let svc = services
        .iter()
        .find(|s| s.service_name == "filefolder")
        .expect("filefolder service");

    assert!(!svc.grpc_services.is_empty(), "expected grpc endpoints");
    assert!(svc.rest_services.is_empty(), "expected no rest endpoints");
}

#[tokio::test]
async fn service_info_includes_rest_when_rest_url_is_configured() {
    let grpc_addr = require_gripmock_addr();
    let caps = capabilities::query_capabilities(&grpc_addr)
        .await
        .expect("query capabilities");

    let without_rest = ServiceInfo::from_capabilities(
        "mock".to_string(),
        None,
        Some(grpc_addr.clone()),
        caps.clone(),
    );
    assert!(
        without_rest.rest.is_none(),
        "REST block requires a configured REST base URL"
    );

    let with_rest = ServiceInfo::from_capabilities(
        "mock".to_string(),
        Some("http://localhost:8080".to_string()),
        Some(grpc_addr),
        caps,
    );
    let rest = with_rest.rest.expect("rest block");
    assert_eq!(rest.url, "http://localhost:8080");
    assert_eq!(rest.endpoints[0].name, "fileobject");
    assert_eq!(
        rest.endpoints[0]
            .versions
            .get("v1")
            .and_then(|v| v.path.as_deref()),
        Some("/api/v1/fileobject")
    );
}
