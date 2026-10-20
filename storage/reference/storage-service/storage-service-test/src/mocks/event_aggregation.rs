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

use std::{fmt::Display, net::SocketAddr, str, sync::Arc};

use parking_lot::Mutex;
use storage_service_protos::nvidia::omniverse::notifications::publisher::v1beta::{
    event_publishing_service_server::{EventPublishingService, EventPublishingServiceServer},
    BatchPublishEventsRequest, BatchPublishEventsResponse, Event, PublishEventRequest, PublishEventResponse,
    PublishingResult,
};
use tokio::net::TcpSocket;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::{CancellationToken, DropGuard};
use tonic::{async_trait, transport::Server, Request, Response, Status};
use tracing::info;

use crate::runtime::RUNTIME;

type PublishRequest = Request<PublishEventRequest>;
type BatchPublishRequest = Request<BatchPublishEventsRequest>;
type PublishResponse = Response<PublishEventResponse>;
type BatchPublishResponse = Response<BatchPublishEventsResponse>;
type PublishResult = Result<PublishResponse, Status>;
type BatchPublishResult = Result<BatchPublishResponse, Status>;

type BoxWhenFn = Box<dyn Fn(&PublishRequest) -> bool + Send + Sync>;
type BoxThenFn = Box<dyn Fn(PublishRequest) -> PublishResult + Send + Sync>;

pub struct Service {
    listen_addr: SocketAddr,
    state: Arc<Mutex<ServiceState>>,
    _drop_guard: DropGuard,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    /// Create a service running in the dedicated runtime.
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(ServiceState::default()));
        let cancel = CancellationToken::new();

        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        RUNTIME.spawn({
            let state = state.clone();
            let cancel = cancel.clone();
            async move {
                addr_tx.send(Self::run_service(state, cancel)).unwrap();
            }
        });

        let listen_addr = addr_rx.recv().unwrap();
        info!("Event Aggregation service mock is listening on {}", listen_addr);

        Self {
            state,
            listen_addr,
            _drop_guard: cancel.drop_guard(),
        }
    }

    /// Return the endpoint url for this service.
    pub fn endpoint_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    /// Create a publish request expectation.
    pub fn mock_publish_request<'a>(&'a self) -> PublishMockBuilder<'a> {
        PublishMockBuilder {
            state: &self.state,
            when: Default::default(),
            callbacks: Default::default(),
        }
    }

    /// Run the service in the dedicated runtime.
    fn run_service(state: Arc<Mutex<ServiceState>>, cancel: CancellationToken) -> SocketAddr {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();

        let listen_addr = socket.local_addr().unwrap();
        let listener = TcpListenerStream::new(socket.listen(512).unwrap());

        tokio::spawn(async move {
            let service = GrpcService { state };
            Server::builder()
                .add_service(EventPublishingServiceServer::new(service))
                .serve_with_incoming_shutdown(listener, cancel.cancelled_owned())
                .await
        });

        listen_addr
    }
}

pub struct PublishMockBuilder<'a> {
    state: &'a Mutex<ServiceState>,
    when: Vec<BoxWhenFn>,
    callbacks: Vec<Box<dyn Fn() + Send + Sync>>,
}

impl PublishMockBuilder<'_> {
    pub fn when(mut self, func: impl Fn(&PublishRequest) -> bool + Send + Sync + 'static) -> Self {
        self.when.push(Box::new(func));
        self
    }

    pub fn when_bearer_authorization(self, value: impl Display) -> Self {
        self.when_authorization(format!("Bearer {}", value))
    }

    pub fn when_authorization(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.when(move |request| {
            request.metadata().get("authorization").map(|v| v.as_bytes()) == Some(value.as_bytes())
        })
    }

    pub fn when_no_authorization(self) -> Self {
        self.when(|request| request.metadata().get("authorization").is_none())
    }

    pub fn when_event(self, event: impl Into<Event>) -> Self {
        let event = event.into();
        self.when(move |request| request.get_ref().event.as_ref() == Some(&event))
    }

    pub fn then_call(mut self, func: impl Fn() + Send + Sync + 'static) -> Self {
        self.callbacks.push(Box::new(func));
        self
    }

    pub fn then(self, func: impl Fn(PublishRequest) -> PublishResult + Send + Sync + 'static) {
        self.state.lock().mocks.push(PublishMock {
            when: Box::new({
                let when_callbacks = self.when;
                move |request| when_callbacks.iter().all(|c| c(request))
            }),
            then: Box::new({
                let then_callbacks = self.callbacks;
                move |request| {
                    for callback in then_callbacks.iter() {
                        callback();
                    }

                    func(request)
                }
            }),
        });
    }

    pub fn then_reply_with_ok(self) {
        self.then(|request| {
            let request = request.into_inner();
            Ok(Response::new(PublishEventResponse {
                result: Some(PublishingResult {
                    event: request.event,
                    success: true,
                    ..Default::default()
                }),
            }))
        });
    }

    pub fn then_reply_with_publish_failure(self, reason: impl Into<String>) {
        let reason = reason.into();
        self.then(move |request| {
            let request = request.into_inner();
            Ok(Response::new(PublishEventResponse {
                result: Some(PublishingResult {
                    event: request.event,
                    success: false,
                    failure_reason: reason.clone(),
                }),
            }))
        });
    }
}

struct PublishMock {
    when: BoxWhenFn,
    then: BoxThenFn,
}

struct GrpcService {
    state: Arc<Mutex<ServiceState>>,
}

impl GrpcService {
    fn get_publish_response(&self, request: PublishRequest) -> Option<PublishResult> {
        let mocks = &mut self.state.lock().mocks;
        for (idx, mock) in mocks.iter_mut().rev().enumerate() {
            if (mock.when)(&request) {
                let response = (mock.then)(request);
                mocks.remove(mocks.len() - idx - 1);

                return Some(response);
            }
        }

        None
    }
}

#[async_trait]
impl EventPublishingService for GrpcService {
    async fn publish_event(&self, _request: PublishRequest) -> PublishResult {
        Err(Status::unimplemented("PublishEvent is not supported"))
    }

    async fn batch_publish_events(&self, request: BatchPublishRequest) -> BatchPublishResult {
        let (metadata, extensions, request) = request.into_parts();
        let events = request.events;
        let mut results = Vec::with_capacity(events.len());

        for event in events {
            let request = Request::from_parts(
                metadata.clone(),
                extensions.clone(),
                PublishEventRequest {
                    event: Some(event.clone()),
                },
            );
            let response = self
                .get_publish_response(request)
                .unwrap_or_else(|| Err(Status::invalid_argument("unexpected batch publish request")));
            results.push(into_publishing_result(event, response));
        }

        Ok(Response::new(BatchPublishEventsResponse { results }))
    }
}

#[derive(Default)]
struct ServiceState {
    mocks: Vec<PublishMock>,
}

/// A publish entry that was received by [`RecordingService`], exposed to tests for assertions.
#[derive(Clone, Debug)]
pub struct ReceivedPublish {
    pub authorization: Option<String>,
    pub event: Option<Event>,
}

type BoxPublishHandler = Arc<dyn Fn(PublishRequest) -> PublishResult + Send + Sync>;

/// A non-intrusive event-aggregation mock that records every publish entry it receives instead of
/// matching per-request expectations like [`Service`]. Responses come from the handler installed
/// via [`RecordingService::set_publish_handler`]. Meant for tests that assert over the set of
/// published events after the fact, such as the synthetic directory events feature.
pub struct RecordingService {
    listen_addr: SocketAddr,
    state: Arc<Mutex<RecordingState>>,
    _drop_guard: DropGuard,
}

#[derive(Default)]
struct RecordingState {
    received_publishes: Vec<ReceivedPublish>,
    publish_handler: Option<BoxPublishHandler>,
}

impl Default for RecordingService {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingService {
    /// Create a service running in the dedicated runtime.
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(RecordingState::default()));
        let cancel = CancellationToken::new();

        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        RUNTIME.spawn({
            let state = state.clone();
            let cancel = cancel.clone();
            async move {
                addr_tx.send(Self::run_service(state, cancel)).unwrap();
            }
        });

        let listen_addr = addr_rx.recv().unwrap();
        info!("Event Aggregation recording mock is listening on {}", listen_addr);

        Self {
            state,
            listen_addr,
            _drop_guard: cancel.drop_guard(),
        }
    }

    /// Return the endpoint url for this service.
    pub fn endpoint_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    /// Return all publish entries the mock has received so far.
    pub fn received_publishes(&self) -> Vec<ReceivedPublish> {
        self.state.lock().received_publishes.clone()
    }

    /// Install the handler invoked on every publish entry. Requests received before a
    /// handler is installed are rejected.
    pub fn set_publish_handler<F>(&self, handler: F)
    where
        F: Fn(PublishRequest) -> PublishResult + Send + Sync + 'static,
    {
        self.state.lock().publish_handler = Some(Arc::new(handler));
    }

    fn run_service(state: Arc<Mutex<RecordingState>>, cancel: CancellationToken) -> SocketAddr {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();

        let listen_addr = socket.local_addr().unwrap();
        let listener = TcpListenerStream::new(socket.listen(512).unwrap());

        tokio::spawn(async move {
            let service = RecordingGrpcService { state };
            Server::builder()
                .add_service(EventPublishingServiceServer::new(service))
                .serve_with_incoming_shutdown(listener, cancel.cancelled_owned())
                .await
        });

        listen_addr
    }
}

struct RecordingGrpcService {
    state: Arc<Mutex<RecordingState>>,
}

#[async_trait]
impl EventPublishingService for RecordingGrpcService {
    async fn publish_event(&self, _request: PublishRequest) -> PublishResult {
        Err(Status::unimplemented("PublishEvent is not supported"))
    }

    async fn batch_publish_events(&self, request: BatchPublishRequest) -> BatchPublishResult {
        let (metadata, extensions, request) = request.into_parts();
        let authorization = metadata
            .get("authorization")
            .and_then(|v| str::from_utf8(v.as_bytes()).ok())
            .map(str::to_owned);
        let events = request.events;
        let handler = {
            let mut state = self.state.lock();
            state
                .received_publishes
                .extend(events.iter().cloned().map(|event| ReceivedPublish {
                    authorization: authorization.clone(),
                    event: Some(event),
                }));
            state.publish_handler.clone()
        };
        let Some(handler) = handler else {
            return Err(Status::unimplemented("no publish handler installed"));
        };

        let mut results = Vec::with_capacity(events.len());
        for event in events {
            let request = Request::from_parts(
                metadata.clone(),
                extensions.clone(),
                PublishEventRequest {
                    event: Some(event.clone()),
                },
            );
            results.push(into_publishing_result(event, handler(request)));
        }

        Ok(Response::new(BatchPublishEventsResponse { results }))
    }
}

/// The surface the test harness needs from an event-aggregation mock: construct it and read back
/// the endpoint the storage-service child should publish to.
pub trait EventAggregationMock {
    fn start() -> Self;
    fn endpoint(&self) -> String;
}

impl EventAggregationMock for Service {
    fn start() -> Self {
        Self::new()
    }

    fn endpoint(&self) -> String {
        self.endpoint_url()
    }
}

impl EventAggregationMock for RecordingService {
    fn start() -> Self {
        Self::new()
    }

    fn endpoint(&self) -> String {
        self.endpoint_url()
    }
}

fn into_publishing_result(event: Event, response: PublishResult) -> PublishingResult {
    match response {
        Ok(response) => response.into_inner().result.unwrap_or_default(),
        Err(error) => PublishingResult {
            event: Some(event),
            success: false,
            failure_reason: error.to_string(),
        },
    }
}
