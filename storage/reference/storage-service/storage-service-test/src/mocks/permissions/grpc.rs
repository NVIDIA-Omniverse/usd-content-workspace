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

use storage_service_protos::nvidia::omniverse::permission::v1beta::{
    permission_service_server::{PermissionService, PermissionServiceServer},
    CheckPermissionBatch, CheckPermissionBatchRequest, CheckPermissionBatchResponse, CheckPermissionRequest,
    CheckPermissionResponse, Decision as DecisionResponse, ResourceActionDecision, ResourceActionDecisionBatch,
};
use tokio::net::TcpSocket;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::{CancellationToken, DropGuard};
use tonic::{async_trait, transport::Server, Request, Response, Status};
use tracing::info;

use crate::{
    mocks::permissions::{Context, Decision},
    runtime::RUNTIME,
};

/// Permission service GRPC interface mock.
#[derive(Debug)]
pub struct Service {
    listen_addr: SocketAddr,
    _drop_guard: DropGuard,
}

impl Service {
    /// Create a GRPC service with the specified permission context.
    pub fn new(context: Context) -> Self {
        let cancel = CancellationToken::new();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        RUNTIME.spawn({
            let cancel = cancel.clone();
            async move {
                addr_tx.send(Self::start_service(context, cancel)).unwrap();
            }
        });

        let listen_addr = addr_rx.recv().unwrap();
        info!("Permission service GRPC mock is listening on {}", listen_addr);

        Self {
            listen_addr,
            _drop_guard: cancel.drop_guard(),
        }
    }

    /// Return the service endpoint url.
    pub fn endpoint_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    /// Start a GRPC service.
    fn start_service(context: Context, cancel: CancellationToken) -> SocketAddr {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();

        let listen_addr = socket.local_addr().unwrap();
        let listener = TcpListenerStream::new(socket.listen(512).unwrap());

        tokio::spawn(async move {
            let service = GrpcService { context };
            Server::builder()
                .add_service(PermissionServiceServer::new(service))
                .serve_with_incoming_shutdown(listener, cancel.cancelled_owned())
                .await
        });

        listen_addr
    }
}

#[derive(Debug)]
struct GrpcService {
    context: Context,
}

impl GrpcService {
    /// Evaluate a permission batch request.
    fn evaluate_batch(&self, batch: CheckPermissionBatch) -> ResourceActionDecisionBatch {
        let resource = batch.resource.expect("resource is missing from a batch request");
        ResourceActionDecisionBatch {
            results: batch
                .actions
                .into_iter()
                .map(|action| {
                    let (decision, reason) =
                        match self
                            .context
                            .evaluate_access(&resource.r#type, &resource.id, &action.name)
                        {
                            Decision::Allow => (DecisionResponse::Allow, None),
                            Decision::Deny => (DecisionResponse::Deny, Some("explicit deny")),
                            Decision::Missing => (DecisionResponse::Deny, None),
                        };

                    ResourceActionDecision {
                        action: action.name,
                        service: action.service,
                        decision: decision as i32,
                        reason: reason.map(String::from),
                    }
                })
                .collect(),
        }
    }
}

#[async_trait]
impl PermissionService for GrpcService {
    async fn check_permission(
        &self,
        _request: Request<CheckPermissionRequest>,
    ) -> Result<Response<CheckPermissionResponse>, Status> {
        // Mock implementation - always allow for testing
        Ok(Response::new(CheckPermissionResponse {
            decision: Decision::Allow as i32,
            reason: None,
        }))
    }

    async fn check_permission_batch(
        &self,
        request: Request<CheckPermissionBatchRequest>,
    ) -> Result<Response<CheckPermissionBatchResponse>, Status> {
        let (metadata, _extensions, request) = request.into_parts();
        let Some(authorization) = metadata.get("authorization") else {
            return Err(Status::unauthenticated("access token is missing"));
        };

        if authorization.as_bytes() != b"Bearer valid.token" {
            return Err(Status::unauthenticated("access token is invalid"));
        }

        self.context.record_request(
            metadata.into_headers(),
            request
                .batches
                .iter()
                .filter_map(|batch| batch.resource.as_ref().map(|resource| resource.id.clone()))
                .collect(),
        );

        Ok(Response::new(CheckPermissionBatchResponse {
            decisions: request
                .batches
                .into_iter()
                .map(|batch| self.evaluate_batch(batch))
                .collect(),
            ..Default::default()
        }))
    }
}
