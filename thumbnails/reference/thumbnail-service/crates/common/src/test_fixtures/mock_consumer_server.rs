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

//! In-process, scriptable `EventConsumerService` gRPC server for exercising the
//! stateful I/O glue that the pure-decision unit tests in `provisioning_tests.rs`
//! deliberately cannot cover.
//!
//! `ensure_durable_queue` and the runtime supervisor's provisioning-failure
//! fallback are the "logic tested, glue untested" seams: the retry/refresh
//! *decisions* are pure and exhaustively unit-tested, but the code that threads
//! them through the real tonic client, interceptor, per-request timeout, and
//! token-refresh call only runs against a live server. This helper stands up a
//! real (loopback) gRPC server whose `CreateDurableQueue` responses are scripted
//! per attempt, so those glue paths get direct coverage without a dev cluster.
//!
//! Only `create_durable_queue` carries scripted behavior; the streaming RPCs
//! return `Unimplemented` (the consume loop that hits them in the supervisor
//! test is torn down immediately via shutdown and only needs the connection to
//! exist).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_server::{
    EventConsumerService, EventConsumerServiceServer,
};
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::{
    ConsumeDurableEventsRequest, ConsumeDurableEventsResponse, ConsumeDurableEventsWithAcksRequest,
    ConsumeDurableEventsWithAcksResponse, ConsumeNonDurableEventsRequest,
    ConsumeNonDurableEventsResponse, CreateDurableQueueRequest, CreateDurableQueueResponse,
    DeleteDurableQueueRequest, DeleteDurableQueueResponse, ManageDeadLetteredEventsRequest,
    ManageDeadLetteredEventsResponse, UpdateDurableQueueRequest, UpdateDurableQueueResponse,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};
use tonic::{Request, Response, Status, Streaming};

/// What the mock should do for a single `CreateDurableQueue` attempt.
#[derive(Clone, Copy, Debug)]
pub enum CreateBehavior {
    /// Respond OK, echoing the request's `queue_id` with `response.created` set
    /// to this flag.
    Created(bool),
    /// Fail the call with this status code (message is a fixed test string).
    Fail(tonic::Code),
    /// Never respond within any sane deadline: sleep effectively forever so the
    /// client-side per-request timeout is what ends the attempt. The sleeping
    /// handler is aborted when the server shuts down.
    Hang,
}

struct ScriptedConsumer {
    /// Per-attempt behavior. Attempts past the end reuse the last entry so a
    /// single-entry script can express "always do X".
    script: Vec<CreateBehavior>,
    create_calls: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl EventConsumerService for ScriptedConsumer {
    type ConsumeDurableEventsStream =
        futures_util::stream::Empty<Result<ConsumeDurableEventsResponse, Status>>;

    async fn consume_durable_events(
        &self,
        _request: Request<ConsumeDurableEventsRequest>,
    ) -> Result<Response<Self::ConsumeDurableEventsStream>, Status> {
        Err(Status::unimplemented("mock: consume_durable_events"))
    }

    type ConsumeDurableEventsWithAcksStream =
        futures_util::stream::Empty<Result<ConsumeDurableEventsWithAcksResponse, Status>>;

    async fn consume_durable_events_with_acks(
        &self,
        _request: Request<Streaming<ConsumeDurableEventsWithAcksRequest>>,
    ) -> Result<Response<Self::ConsumeDurableEventsWithAcksStream>, Status> {
        Err(Status::unimplemented(
            "mock: consume_durable_events_with_acks",
        ))
    }

    type ConsumeNonDurableEventsStream =
        futures_util::stream::Empty<Result<ConsumeNonDurableEventsResponse, Status>>;

    async fn consume_non_durable_events(
        &self,
        _request: Request<Streaming<ConsumeNonDurableEventsRequest>>,
    ) -> Result<Response<Self::ConsumeNonDurableEventsStream>, Status> {
        Err(Status::unimplemented("mock: consume_non_durable_events"))
    }

    async fn create_durable_queue(
        &self,
        request: Request<CreateDurableQueueRequest>,
    ) -> Result<Response<CreateDurableQueueResponse>, Status> {
        let attempt = self.create_calls.fetch_add(1, Ordering::SeqCst);
        let behavior = self
            .script
            .get(attempt)
            .or_else(|| self.script.last())
            .copied()
            .unwrap_or(CreateBehavior::Created(true));
        let queue_id = request.into_inner().queue_id;

        match behavior {
            CreateBehavior::Created(created) => Ok(Response::new(CreateDurableQueueResponse {
                created,
                queue_id,
            })),
            CreateBehavior::Fail(code) => Err(Status::new(code, "mock provisioning failure")),
            CreateBehavior::Hang => {
                // Far longer than any test's per-request timeout; the server
                // shutdown aborts this task at teardown.
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(Response::new(CreateDurableQueueResponse {
                    created: true,
                    queue_id,
                }))
            }
        }
    }

    async fn delete_durable_queue(
        &self,
        _request: Request<DeleteDurableQueueRequest>,
    ) -> Result<Response<DeleteDurableQueueResponse>, Status> {
        Err(Status::unimplemented("mock: delete_durable_queue"))
    }

    async fn update_durable_queue(
        &self,
        _request: Request<UpdateDurableQueueRequest>,
    ) -> Result<Response<UpdateDurableQueueResponse>, Status> {
        Err(Status::unimplemented("mock: update_durable_queue"))
    }

    type ManageDeadLetteredEventsStream =
        futures_util::stream::Empty<Result<ManageDeadLetteredEventsResponse, Status>>;

    async fn manage_dead_lettered_events(
        &self,
        _request: Request<Streaming<ManageDeadLetteredEventsRequest>>,
    ) -> Result<Response<Self::ManageDeadLetteredEventsStream>, Status> {
        Err(Status::unimplemented("mock: manage_dead_lettered_events"))
    }
}

/// A running mock `EventConsumerService` bound to an ephemeral loopback port.
///
/// Drop or [`MockConsumerServer::shutdown`] to stop it. Dropping signals the
/// server to stop and detaches the task; `shutdown().await` additionally waits
/// for it to finish, which is the tidy path for tests.
pub struct MockConsumerServer {
    pub addr: SocketAddr,
    create_calls: Arc<AtomicUsize>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: JoinHandle<()>,
}

impl MockConsumerServer {
    /// Start a server whose `CreateDurableQueue` responses follow `script`
    /// (attempt `i` uses `script[i]`, clamping to the last entry afterwards).
    pub async fn start(script: Vec<CreateBehavior>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock consumer server must bind an ephemeral loopback port");
        let addr = listener
            .local_addr()
            .expect("bound listener must expose its local addr");

        let create_calls = Arc::new(AtomicUsize::new(0));
        let svc = ScriptedConsumer {
            script,
            create_calls: create_calls.clone(),
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(EventConsumerServiceServer::new(svc))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        MockConsumerServer {
            addr,
            create_calls,
            shutdown_tx: Some(shutdown_tx),
            server_task,
        }
    }

    /// Connect a fresh `Channel` to this server.
    pub async fn channel(&self) -> Channel {
        tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .expect("mock server endpoint url must parse")
            .connect()
            .await
            .expect("mock server must accept a client connection")
    }

    /// Number of `CreateDurableQueue` attempts the server has observed.
    pub fn create_call_count(&self) -> usize {
        self.create_calls.load(Ordering::SeqCst)
    }

    /// Stop the server and wait for its task to finish.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = (&mut self.server_task).await;
    }
}

impl Drop for MockConsumerServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.server_task.abort();
    }
}
