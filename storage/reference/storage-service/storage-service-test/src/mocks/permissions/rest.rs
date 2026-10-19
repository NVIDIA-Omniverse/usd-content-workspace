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

use axum::{
    extract::{FromRequestParts, State},
    http::{header::AUTHORIZATION, request::Parts, HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use std::{collections::HashMap, net::SocketAddr};
use storage_service_permissions::client::rest::models::{
    ActionDecision, BatchRequest, BatchResponse, Decision as DecisionResponse, Request,
};
use tokio::net::TcpListener;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::info;

use crate::{
    mocks::permissions::{Context, Decision},
    runtime::RUNTIME,
};

/// Permission service fixture.
#[derive(Debug)]
pub struct Service {
    listen_addr: SocketAddr,
    _drop_guard: DropGuard,
}

/// REST permission service implementation.
impl Service {
    /// Create a service running in a dedicated runtime.
    pub fn new(context: Context) -> Self {
        let cancel = CancellationToken::new();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        RUNTIME.spawn({
            let cancel = cancel.clone();
            async move {
                addr_tx.send(Self::start_server(context, cancel).await).unwrap();
            }
        });

        let listen_addr = addr_rx.recv().unwrap();
        info!("Permission service REST mock is listening on {}", listen_addr);

        Self {
            listen_addr,
            _drop_guard: cancel.drop_guard(),
        }
    }

    /// Return the service endpoint url.
    pub fn endpoint_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    /// Run a service until the cancellation signal is triggered.
    async fn start_server(context: Context, cancel: CancellationToken) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();

        let router = Router::new()
            .route("/v1beta/authorization/batch/", post(Self::evaluate_batch_request))
            .with_state(context);

        tokio::spawn(async move {
            cancel
                .run_until_cancelled(async move {
                    axum::serve(listener, router).await.unwrap();
                })
                .await;
        });

        listen_addr
    }

    /// Evaluate a batch authorization request.
    async fn evaluate_batch_request(
        _: ValidAuthorizationHeader,
        State(context): State<Context>,
        headers: HeaderMap,
        Json(request): Json<BatchRequest>,
    ) -> Result<Json<BatchResponse>, StatusCode> {
        context.record_request(
            headers,
            request
                .batches
                .iter()
                .map(|request| request.resource.id.clone())
                .collect(),
        );

        Ok(Json(BatchResponse {
            decisions: request
                .batches
                .into_iter()
                .map(|request| Self::evaluate_request(&context, request))
                .collect(),
        }))
    }

    /// Evaluate a single request in a batch.
    fn evaluate_request(context: &Context, request: Request) -> HashMap<String, ActionDecision> {
        request
            .actions
            .into_iter()
            .map(|action| {
                let (decision, reason) = match context.evaluate_access(
                    &request.resource.resource_type,
                    &request.resource.id,
                    &action.name,
                ) {
                    Decision::Allow => (DecisionResponse::Allow, None),
                    Decision::Deny => (DecisionResponse::Deny, Some("explicit deny")),
                    Decision::Missing => (DecisionResponse::Deny, None),
                };

                (
                    format!("{}:{}", action.service, action.name),
                    ActionDecision {
                        decision,
                        action: action.name,
                        reason: reason.map(Into::into),
                    },
                )
            })
            .collect()
    }
}

struct ValidAuthorizationHeader;

impl<S: Send + Sync> FromRequestParts<S> for ValidAuthorizationHeader {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(value) = parts.headers.get(AUTHORIZATION) else {
            return Err(StatusCode::UNAUTHORIZED);
        };

        if value.as_bytes() != b"Bearer valid.token" {
            return Err(StatusCode::UNAUTHORIZED);
        }

        Ok(Self)
    }
}
