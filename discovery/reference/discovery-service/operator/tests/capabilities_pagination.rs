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

// Verifies the client follows ListServices pagination until next_page_token is
// empty, and rejects servers whose tokens would page forever. Runs against an
// in-process v2beta server.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use auto_discovery_service::client::capabilities;
use auto_discovery_service::proto::capabilities_service_server::{
    CapabilitiesService, CapabilitiesServiceServer,
};
use auto_discovery_service::proto::{ListServicesRequest, ListServicesResponse, ServiceEntry};
use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

/// How the test server chooses `next_page_token` for each request.
#[derive(Clone, Copy)]
enum Paging {
    /// Serves `count` pages, one service each, using tokens `page-1`..`page-{count-1}`.
    Pages(usize),
    /// Always returns the token it was sent (or `loop` on the first page).
    RepeatToken,
    /// Always returns a new token.
    Endless,
}

struct PagingServer {
    paging: Paging,
    tokens_received: Arc<Mutex<Vec<String>>>,
}

fn entry(name: &str) -> ServiceEntry {
    ServiceEntry {
        service_name: name.to_string(),
        grpc_services: vec![],
        rest_services: vec![],
    }
}

#[tonic::async_trait]
impl CapabilitiesService for PagingServer {
    async fn list_services(
        &self,
        request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let token = request.into_inner().page_token;
        let mut received = self.tokens_received.lock().unwrap();
        received.push(token.clone());
        let call = received.len();

        let response = match self.paging {
            Paging::Pages(count) => {
                let index = match token.strip_prefix("page-") {
                    None if token.is_empty() => 0,
                    Some(n) => n
                        .parse()
                        .map_err(|_| Status::invalid_argument("bad token"))?,
                    None => return Err(Status::invalid_argument("bad token")),
                };
                ListServicesResponse {
                    services: vec![entry(&format!("service-{index}"))],
                    next_page_token: if index + 1 < count {
                        format!("page-{}", index + 1)
                    } else {
                        String::new()
                    },
                }
            }
            Paging::RepeatToken => ListServicesResponse {
                services: vec![entry("repeat")],
                next_page_token: if token.is_empty() {
                    "loop".to_string()
                } else {
                    token
                },
            },
            Paging::Endless => ListServicesResponse {
                services: vec![entry(&format!("endless-{call}"))],
                next_page_token: format!("next-{call}"),
            },
        };
        Ok(Response::new(response))
    }
}

async fn spawn_server(paging: Paging) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let tokens_received = Arc::new(Mutex::new(Vec::new()));
    let server = PagingServer {
        paging,
        tokens_received: tokens_received.clone(),
    };
    let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let addr = incoming.local_addr().expect("local addr");
    tokio::spawn(
        Server::builder()
            .add_service(CapabilitiesServiceServer::new(server))
            .serve_with_incoming(incoming),
    );
    (addr, tokens_received)
}

#[tokio::test]
async fn collects_services_from_every_page() {
    let (addr, tokens) = spawn_server(Paging::Pages(3)).await;

    let entries = capabilities::query_capabilities(&format!("http://{addr}"))
        .await
        .expect("query capabilities");

    let names: Vec<&str> = entries.iter().map(|e| e.service_name.as_str()).collect();
    assert_eq!(names, ["service-0", "service-1", "service-2"]);
    assert_eq!(*tokens.lock().unwrap(), ["", "page-1", "page-2"]);
}

#[tokio::test]
async fn single_page_makes_one_request() {
    let (addr, tokens) = spawn_server(Paging::Pages(1)).await;

    let entries = capabilities::query_capabilities(&format!("http://{addr}"))
        .await
        .expect("query capabilities");

    assert_eq!(entries.len(), 1);
    assert_eq!(*tokens.lock().unwrap(), [""]);
}

#[tokio::test]
async fn repeated_page_token_is_an_error() {
    let (addr, tokens) = spawn_server(Paging::RepeatToken).await;

    let error = capabilities::query_capabilities(&format!("http://{addr}"))
        .await
        .expect_err("repeated token must fail");

    assert!(
        error.to_string().contains("same page token"),
        "unexpected error: {error}"
    );
    assert_eq!(*tokens.lock().unwrap(), ["", "loop"]);
}

#[tokio::test]
async fn endless_pagination_is_capped() {
    let (addr, tokens) = spawn_server(Paging::Endless).await;

    let error = capabilities::query_capabilities(&format!("http://{addr}"))
        .await
        .expect_err("endless pagination must fail");

    assert!(
        error.to_string().contains("more than 100 pages"),
        "unexpected error: {error}"
    );
    assert_eq!(tokens.lock().unwrap().len(), 100);
}

#[tokio::test]
async fn paginates_over_a_single_portforward_stream() {
    let (addr, tokens) = spawn_server(Paging::Pages(3)).await;
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");

    let entries = capabilities::query_capabilities_via_stream(stream, "portforward-test")
        .await
        .expect("query capabilities via stream");

    assert_eq!(entries.len(), 3);
    assert_eq!(*tokens.lock().unwrap(), ["", "page-1", "page-2"]);
}
