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

use std::net::SocketAddr;

use auto_discovery_service::proto::capabilities_service_server::{
    CapabilitiesService, CapabilitiesServiceServer,
};
use auto_discovery_service::proto::{
    GrpcEndpoint, GrpcService, ListServicesRequest, ListServicesResponse, RestEndpoint,
    RestService, RestVersion, RestVersionDetail, ServiceEntry,
};
use tonic::{transport::Server, Request, Response, Status};

#[derive(Default)]
struct MockCapabilities;

#[tonic::async_trait]
impl CapabilitiesService for MockCapabilities {
    async fn list_services(
        &self,
        _request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let services = vec![
            ServiceEntry {
                service_name: "fileobject".to_string(),
                grpc_services: vec![GrpcService {
                    endpoints: vec![GrpcEndpoint {
                        service_name: "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
                            .to_string(),
                    }],
                }],
                rest_services: vec![RestService {
                    endpoints: vec![RestEndpoint {
                        name: "fileobject".to_string(),
                        versions: vec![RestVersion {
                            version: "v1".to_string(),
                            url_detail: Some(RestVersionDetail {
                                path: "/api/v1/fileobject".to_string(),
                            }),
                        }],
                    }],
                }],
            },
            ServiceEntry {
                service_name: "filefolder".to_string(),
                grpc_services: vec![GrpcService {
                    endpoints: vec![GrpcEndpoint {
                        service_name: "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
                            .to_string(),
                    }],
                }],
                rest_services: vec![],
            },
        ];

        Ok(Response::new(ListServicesResponse {
            services,
            next_page_token: String::new(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(50051);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!("mock capabilities service listening on {}", addr);

    Server::builder()
        .add_service(CapabilitiesServiceServer::new(MockCapabilities))
        .serve(addr)
        .await?;

    Ok(())
}
