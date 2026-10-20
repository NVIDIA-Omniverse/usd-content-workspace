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

//! In-process `EventPublishingService` gRPC server that records what the service
//! published, so tests can assert on events as observable output.
//!
//! Association events are fire-and-forget: `publish_event` is spawned and its result
//! is only logged, so "exactly one `omni.thumbnail.disassociated` event per successful
//! removal" and "no event for a failure" are invisible to a test unless something is
//! listening. This records every published event and lets tests wait for them.
//!
//! Unlike [`super::mock_storage_server`], this is per-test rather than per-binary: the
//! publishing channel comes from the `OutboundClients` a test passes in, not from
//! process-wide config, so each test can own its own recorder.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notifications_proto::nvidia::omniverse::notifications::publisher::v1beta::event_publishing_service_server::{
    EventPublishingService, EventPublishingServiceServer,
};
use notifications_proto::nvidia::omniverse::notifications::publisher::v1beta::{
    BatchPublishEventsRequest, BatchPublishEventsResponse, Event, PublishEventRequest,
    PublishEventResponse, PublishingResult,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};

/// One event the service published.
#[derive(Clone, Debug)]
pub struct PublishedEvent {
    pub event_type: String,
    pub resource_id: String,
    pub message: serde_json::Value,
    /// The W3C `traceparent` that arrived in the request metadata, if any.
    ///
    /// Recorded because how the client was *built* is not evidence that the trace
    /// layer and the auth interceptor composed in the order that works: the layer
    /// has to sit outside the interceptor, and only the received header shows it.
    pub traceparent: Option<String>,
    /// The `authorization` header that arrived, if any. Recorded for the same
    /// reason — without it, deleting the interceptor leaves the suite green.
    pub authorization: Option<String>,
}

impl PublishedEvent {
    /// A string out of the event message by JSON pointer, e.g.
    /// `/thumbnail/resource_address`. The message is the nested ADR 0002 shape, so
    /// most fields of interest are a level down.
    pub fn message_str(&self, pointer: &str) -> Option<&str> {
        self.message
            .pointer(pointer)
            .and_then(|value| value.as_str())
    }
}

/// How the recorder answers a publish, so tests can exercise the failure paths
/// that generate dispatch (unlike association publishing) propagates.
#[derive(Clone, Debug)]
pub enum PublisherBehavior {
    /// Record the event and report success.
    Succeed,
    /// Record the event, then fail the RPC with this status code.
    FailWith(tonic::Code),
    /// Record the event, then return `success = false` with this reason.
    Reject(String),
    /// Succeed for the first `n` publishes, then reject every later one.
    ///
    /// Injects a partial publisher failure independently of request validation.
    /// Dispatch is sequential, so the split is deterministic.
    RejectAfter { successes: usize, reason: String },
    /// Record the event, hold the RPC open for this long, then report success.
    ///
    /// For the association publishes, which are spawned and detached: a slow
    /// publisher is the condition under which a detached task that shares the
    /// request's span makes the request's exported span duration wrong.
    SucceedAfter(Duration),
    /// Fail the first publish with `Unauthenticated`, then succeed.
    ///
    /// Drives the refresh-and-reissue branch, which is the only place the layered
    /// `TonicTraceService<InterceptedService<…>>` stack is driven twice on one
    /// client: the interceptor is re-invoked with a refreshed token and
    /// `poll_ready` runs again on a service that already errored.
    UnauthenticatedThenSucceed,
}

struct RecordingPublisher {
    events: Arc<Mutex<Vec<PublishedEvent>>>,
    behavior: PublisherBehavior,
    /// Publishes answered so far, for the count-dependent behaviors.
    answered: Arc<Mutex<usize>>,
}

/// A header from the request metadata, as a `String` a test can compare.
fn header(metadata: &tonic::metadata::MetadataMap, name: &str) -> Option<String> {
    metadata
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

impl RecordingPublisher {
    fn record(&self, event: Option<Event>, metadata: &tonic::metadata::MetadataMap) {
        let Some(event) = event else {
            return;
        };
        let message = event
            .message
            .as_ref()
            .map(|message| {
                crate::utils::prost_to_json_value(&prost_types::Value {
                    kind: Some(prost_types::value::Kind::StructValue(message.clone())),
                })
            })
            .unwrap_or(serde_json::Value::Null);

        self.events
            .lock()
            .expect("published-event log must not be poisoned")
            .push(PublishedEvent {
                event_type: event.event_type,
                resource_id: event
                    .resource
                    .map(|resource| resource.resource_id)
                    .unwrap_or_default(),
                message,
                traceparent: header(metadata, "traceparent"),
                authorization: header(metadata, "authorization"),
            });
    }
}

/// The publisher's success result. The service only reads `success`, and echoing the
/// event back would just re-serialize what the caller already sent.
fn succeeded() -> PublishingResult {
    PublishingResult {
        event: None,
        success: true,
        failure_reason: String::new(),
    }
}

impl RecordingPublisher {
    fn result(&self) -> Result<PublishingResult, Status> {
        match &self.behavior {
            PublisherBehavior::Succeed => Ok(succeeded()),
            PublisherBehavior::FailWith(code) => {
                Err(Status::new(*code, "mock publisher: injected failure"))
            }
            PublisherBehavior::Reject(reason) => Ok(PublishingResult {
                event: None,
                success: false,
                failure_reason: reason.clone(),
            }),
            PublisherBehavior::RejectAfter { successes, reason } => {
                let mut answered = self.answered.lock().expect("answered lock");
                let index = *answered;
                *answered += 1;
                if index < *successes {
                    Ok(succeeded())
                } else {
                    Ok(PublishingResult {
                        event: None,
                        success: false,
                        failure_reason: reason.clone(),
                    })
                }
            }
            // The delay is applied by the handler, which is where `await` is
            // available; by the time this runs it has already elapsed.
            PublisherBehavior::SucceedAfter(_) => Ok(succeeded()),
            PublisherBehavior::UnauthenticatedThenSucceed => {
                let mut answered = self.answered.lock().expect("answered lock");
                let index = *answered;
                *answered += 1;
                if index == 0 {
                    Err(Status::new(
                        tonic::Code::Unauthenticated,
                        "mock publisher: token rejected once",
                    ))
                } else {
                    Ok(succeeded())
                }
            }
        }
    }
}

#[tonic::async_trait]
impl EventPublishingService for RecordingPublisher {
    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        let (metadata, _, body) = request.into_parts();
        self.record(body.event, &metadata);

        if let PublisherBehavior::SucceedAfter(delay) = &self.behavior {
            tokio::time::sleep(*delay).await;
        }

        Ok(Response::new(PublishEventResponse {
            result: Some(self.result()?),
        }))
    }

    async fn batch_publish_events(
        &self,
        request: Request<BatchPublishEventsRequest>,
    ) -> Result<Response<BatchPublishEventsResponse>, Status> {
        let (metadata, _, body) = request.into_parts();
        let events = body.events;
        let event_count = events.len();
        for event in events {
            self.record(Some(event), &metadata);
        }

        if let PublisherBehavior::FailWith(code) = &self.behavior {
            return Err(Status::new(*code, "mock publisher: injected failure"));
        }

        let results = (0..event_count)
            .map(|_| self.result())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Response::new(BatchPublishEventsResponse { results }))
    }
}

/// A running recorder bound to an ephemeral loopback port.
pub struct MockPublisherServer {
    pub addr: SocketAddr,
    events: Arc<Mutex<Vec<PublishedEvent>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: JoinHandle<()>,
}

impl MockPublisherServer {
    pub async fn start() -> Self {
        Self::start_with(PublisherBehavior::Succeed).await
    }

    pub async fn start_with(behavior: PublisherBehavior) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock publisher server must bind an ephemeral loopback port");
        let addr = listener
            .local_addr()
            .expect("bound listener must expose its local addr");

        let events = Arc::new(Mutex::new(Vec::new()));
        let service = RecordingPublisher {
            answered: Arc::new(Mutex::new(0)),
            events: events.clone(),
            behavior,
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .add_service(EventPublishingServiceServer::new(service))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        MockPublisherServer {
            addr,
            events,
            shutdown_tx: Some(shutdown_tx),
            server_task,
        }
    }

    /// An `OutboundClients` whose event publishing reaches this recorder.
    pub fn outbound(&self) -> Arc<crate::outbound::OutboundClients> {
        Arc::new(crate::outbound::OutboundClients::for_tests_publishing_to(
            self.addr,
        ))
    }

    /// As [`Self::outbound`], but a token refresh plants a different token.
    ///
    /// For the reissue tests: with the default no-op refresher both attempts carry
    /// the same bearer, so a test cannot tell a reissue that refreshed from one that
    /// resent the credential the server just rejected.
    pub fn outbound_with_token_refresh(&self) -> Arc<crate::outbound::OutboundClients> {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .expect("mock publisher endpoint url should parse")
            .connect_lazy();
        Arc::new(crate::outbound::OutboundClients {
            event_publishing_channel: Some(channel),
            ..crate::outbound::OutboundClients::for_tests_with_token_refresh()
        })
    }

    /// Every event recorded so far.
    pub fn events(&self) -> Vec<PublishedEvent> {
        self.events
            .lock()
            .expect("published-event log must not be poisoned")
            .clone()
    }

    /// Wait until `count` events have arrived, then return them.
    ///
    /// Publishing is spawned, so a test that asserted immediately would race the
    /// publish task. Panics on timeout rather than returning a short list, so a
    /// missing event fails loudly instead of as a confusing length mismatch.
    pub async fn await_events(&self, count: usize, timeout: Duration) -> Vec<PublishedEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let events = self.events();
            if events.len() >= count {
                return events;
            }
            if Instant::now() >= deadline {
                panic!(
                    "expected {count} published event(s) within {timeout:?}, got {}",
                    events.len()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Give any spawned publish a chance to arrive, then report what did.
    ///
    /// For asserting that *nothing* was published: there is no event to wait for, so
    /// this waits out the window in which one could plausibly appear.
    pub async fn settle(&self, window: Duration) -> Vec<PublishedEvent> {
        tokio::time::sleep(window).await;
        self.events()
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = (&mut self.server_task).await;
    }
}

impl Drop for MockPublisherServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.server_task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failing_batch_records_every_event_before_failing_the_rpc() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            answered: Arc::new(Mutex::new(0)),
            events: events.clone(),
            behavior: PublisherBehavior::FailWith(tonic::Code::Unavailable),
        };
        let request = BatchPublishEventsRequest {
            events: ["first", "second"]
                .into_iter()
                .map(|event_type| Event {
                    event_type: event_type.to_string(),
                    ..Event::default()
                })
                .collect(),
        };

        let status = publisher
            .batch_publish_events(Request::new(request))
            .await
            .expect_err("the injected failure applies to the batch RPC");

        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(
            events
                .lock()
                .expect("published-event log must not be poisoned")
                .len(),
            2
        );
    }
}
