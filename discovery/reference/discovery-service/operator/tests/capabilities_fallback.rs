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

// Verifies the client falls back to the deprecated v2alpha API when a service does
// not implement v2beta, and keeps using v2alpha for later pages. Runs against an
// in-process v2alpha-only server that returns two pages.

#[allow(deprecated)]
mod v2alpha {
    tonic::include_proto!("nvidia.omniverse.discovery.capabilities.v2alpha");
}

use std::net::SocketAddr;

use auto_discovery_service::client::capabilities;
use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use v2alpha::capabilities_service_server::{CapabilitiesService, CapabilitiesServiceServer};
use v2alpha::{GrpcEndpoint, GrpcService, ListServicesRequest, ListServicesResponse, ServiceEntry};

struct V2AlphaOnly;

fn grpc_entry(service_name: &str, grpc_service: &str) -> ServiceEntry {
    ServiceEntry {
        service_name: service_name.to_string(),
        grpc_services: vec![GrpcService {
            endpoints: vec![GrpcEndpoint {
                service_name: grpc_service.to_string(),
            }],
        }],
        rest_services: vec![],
    }
}

#[tonic::async_trait]
impl CapabilitiesService for V2AlphaOnly {
    async fn list_services(
        &self,
        request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let response = match request.into_inner().page_token.as_str() {
            "" => ListServicesResponse {
                services: vec![grpc_entry(
                    "fileobject",
                    "nvidia.omniverse.storage.fileobject.v1.FileObjectService",
                )],
                next_page_token: "page-2".to_string(),
            },
            "page-2" => ListServicesResponse {
                services: vec![grpc_entry(
                    "filefolder",
                    "nvidia.omniverse.storage.filefolder.v1.FileFolderService",
                )],
                next_page_token: String::new(),
            },
            token => return Err(Status::invalid_argument(format!("unknown token {token}"))),
        };
        Ok(Response::new(response))
    }
}

async fn spawn_v2alpha_server() -> SocketAddr {
    let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let addr = incoming.local_addr().expect("local addr");
    tokio::spawn(
        Server::builder()
            .add_service(CapabilitiesServiceServer::new(V2AlphaOnly))
            .serve_with_incoming(incoming),
    );
    addr
}

#[tokio::test]
async fn falls_back_to_v2alpha_when_v2beta_is_unimplemented() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let addr = spawn_v2alpha_server().await;

    let entries = capabilities::query_capabilities(&format!("http://{addr}"))
        .await
        .expect("query capabilities via v2alpha fallback");

    let names: Vec<&str> = entries.iter().map(|e| e.service_name.as_str()).collect();
    assert_eq!(names, ["filefolder", "fileobject"]);
}
