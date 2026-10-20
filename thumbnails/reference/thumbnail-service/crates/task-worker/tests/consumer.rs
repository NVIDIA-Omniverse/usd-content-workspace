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

//! The WorkerEventQueue consume loop, against a scripted Notification double.
//!
//! What matters here is delivery bookkeeping rather than image work, so the
//! processor is a script: which deliveries get acknowledged, that the in-flight
//! bound actually bounds, that a non-generate event is not processed, that a
//! retryable failure sends a NACK without recycling the stream, and that a
//! broken ack channel (rejected NACK) falls back to a stream recycle.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_server::{
    EventConsumerService, EventConsumerServiceServer,
};
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::{
    ConsumeDurableEventsRequest, ConsumeDurableEventsResponse, ConsumeDurableEventsWithAcksRequest,
    ConsumeDurableEventsWithAcksResponse, ConsumeNonDurableEventsRequest,
    ConsumeNonDurableEventsResponse, CreateDurableQueueRequest, CreateDurableQueueResponse,
    DeleteDurableQueueRequest, DeleteDurableQueueResponse, Event, ManageDeadLetteredEventsRequest,
    ManageDeadLetteredEventsResponse, UpdateDurableQueueRequest, UpdateDurableQueueResponse,
};
use thumbnail_common::work_events::GENERATE_EVENT_TYPE;
use thumbnail_task_worker::consumer::Consumer;
use thumbnail_task_worker::health::HealthState;
use thumbnail_task_worker::metrics::WorkerMetrics;
use thumbnail_task_worker::outcome::{FailureClass, WorkOutcome};
use thumbnail_task_worker::pipeline::MessageProcessor;
use tokio::sync::watch;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

// --- Notification double ---------------------------------------------------

#[derive(Default, Debug)]
struct ConsumerLog {
    /// One entry per `ConsumeDurableEventsWithAcks` call, i.e. per session.
    sessions: usize,
    /// Delivery tags the worker acknowledged, across all sessions.
    acked: Vec<i64>,
    /// Delivery tags the worker negatively acknowledged (NACK), across all sessions.
    nacked: Vec<i64>,
    /// `CreateDurableQueue` calls, so a test can tell a re-provisioning recovery
    /// from a bare reconnect. Startup provisioning happens in `main`, not here,
    /// so any call recorded by this double came from the consume loop.
    queues_created: Vec<String>,
}

/// Block until the recorded log satisfies `predicate`, or fail after 5s.
///
/// The double records acknowledgements from a detached reader task, so sleeping
/// for a fixed interval establishes only that time passed — not that the reader
/// has drained the request stream. Every *positive* assertion about what the
/// worker sent goes through here so a slow runner produces a slower pass rather
/// than an intermittent failure. Negative assertions ("nothing was NACKed") stay
/// as plain waits: no scheduling delay can conjure a message that was never sent.
async fn await_log(
    log: &Arc<Mutex<ConsumerLog>>,
    what: &str,
    predicate: impl Fn(&ConsumerLog) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        {
            let current = log.lock().expect("log must not be poisoned");
            if predicate(&current) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out after 5s waiting for {what}; log: {current:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// How the Notification double should behave for one test.
#[derive(Default)]
struct ConsumerScript {
    /// Deliveries emitted on each session, in order. A session past the end of
    /// the script emits nothing and stays open.
    sessions: VecDeque<Vec<(i64, Event)>>,
    /// Gap between deliveries within a session. Zero sends them as fast as the
    /// client reads, which is what most tests want; a non-zero value is how a
    /// test says "this stream is busy" rather than "this stream has gone quiet",
    /// a distinction the idle defense turns on.
    drip: Duration,
    /// Refuse sessions beyond this count, standing in for a consumer service
    /// that has become unreachable.
    accept_at_most: Option<usize>,
    /// Withhold response headers until there is a delivery to write, which is
    /// what the real Notification service does and what this double did not.
    ///
    /// Returning `Ok(Response::new(..))` promptly makes tonic flush headers
    /// immediately, so every test here saw a stream that established the instant
    /// it was opened. The platform does not behave that way on a quiet queue, and
    /// the difference is invisible until a queue is quiet: WorkerEventQueue is idle at
    /// startup, so the worker blocked in the open call, never reported ready, and
    /// its first dev rollout failed the readiness gate.
    defer_headers: bool,
    /// Statuses to answer the first sessions with, one per session, before the
    /// script above takes over. Drives the reconnect paths that depend on *why*
    /// a session failed.
    reject_first: VecDeque<tonic::Code>,
}

struct ScriptedConsumer {
    script: Mutex<VecDeque<Vec<(i64, Event)>>>,
    drip: Duration,
    accept_at_most: Option<usize>,
    reject_first: Mutex<VecDeque<tonic::Code>>,
    defer_headers: bool,
    log: Arc<Mutex<ConsumerLog>>,
}

#[tonic::async_trait]
impl EventConsumerService for ScriptedConsumer {
    type ConsumeDurableEventsWithAcksStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<ConsumeDurableEventsWithAcksResponse, Status>>
                + Send,
        >,
    >;

    async fn consume_durable_events_with_acks(
        &self,
        request: Request<Streaming<ConsumeDurableEventsWithAcksRequest>>,
    ) -> Result<Response<Self::ConsumeDurableEventsWithAcksStream>, Status> {
        let deliveries = self
            .script
            .lock()
            .expect("script must not be poisoned")
            .pop_front()
            .unwrap_or_default();

        let session = {
            let mut log = self.log.lock().expect("log must not be poisoned");
            log.sessions += 1;
            log.sessions
        };

        if let Some(code) = self
            .reject_first
            .lock()
            .expect("rejections must not be poisoned")
            .pop_front()
        {
            return Err(Status::new(code, "scripted session rejection"));
        }

        if self.accept_at_most.is_some_and(|limit| session > limit) {
            return Err(Status::unavailable("consumer service is unreachable"));
        }

        // Nothing to write and asked to behave like the platform: hold the
        // handler open so tonic has no reason to send headers. A client that
        // waits for them waits here forever.
        if self.defer_headers && deliveries.is_empty() {
            std::future::pending::<()>().await;
        }

        // Record the acknowledgements and negative acknowledgements the worker
        // sends back on this session.
        let log = self.log.clone();
        let mut inbound = request.into_inner();
        tokio::spawn(async move {
            while let Some(Ok(message)) = inbound.next().await {
                let mut log = log.lock().expect("log must not be poisoned");
                log.acked.extend(message.ack_delivery_tags);
                log.nacked.extend(message.nack_delivery_tags);
            }
        });

        // Sent over a channel rather than a ready-made iterator so the gap
        // between deliveries is under the test's control.
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let drip = self.drip;
        tokio::spawn(async move {
            for (delivery_tag, event) in deliveries {
                if !drip.is_zero() {
                    tokio::time::sleep(drip).await;
                }
                let response = ConsumeDurableEventsWithAcksResponse {
                    events: vec![event],
                    delivery_tag,
                };
                if tx.send(Ok(response)).await.is_err() {
                    return;
                }
            }
            // Hold the sender open past the script, exactly like a real queue
            // that has gone quiet. Waiting on the receiver rather than parking
            // forever means the task ends when the client goes away.
            tx.closed().await;
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    type ConsumeDurableEventsStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<ConsumeDurableEventsResponse, Status>> + Send>,
    >;

    async fn consume_durable_events(
        &self,
        _: Request<ConsumeDurableEventsRequest>,
    ) -> Result<Response<Self::ConsumeDurableEventsStream>, Status> {
        Err(Status::unimplemented(
            "deprecated; the worker uses WithAcks",
        ))
    }

    type ConsumeNonDurableEventsStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<ConsumeNonDurableEventsResponse, Status>> + Send,
        >,
    >;

    async fn consume_non_durable_events(
        &self,
        _: Request<Streaming<ConsumeNonDurableEventsRequest>>,
    ) -> Result<Response<Self::ConsumeNonDurableEventsStream>, Status> {
        Err(Status::unimplemented("the worker consumes durably"))
    }

    async fn create_durable_queue(
        &self,
        request: Request<CreateDurableQueueRequest>,
    ) -> Result<Response<CreateDurableQueueResponse>, Status> {
        let queue_id = request.into_inner().queue_id;
        self.log
            .lock()
            .expect("log must not be poisoned")
            .queues_created
            .push(queue_id.clone());
        Ok(Response::new(CreateDurableQueueResponse {
            queue_id,
            created: true,
        }))
    }

    async fn delete_durable_queue(
        &self,
        _: Request<DeleteDurableQueueRequest>,
    ) -> Result<Response<DeleteDurableQueueResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn update_durable_queue(
        &self,
        _: Request<UpdateDurableQueueRequest>,
    ) -> Result<Response<UpdateDurableQueueResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    type ManageDeadLetteredEventsStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<ManageDeadLetteredEventsResponse, Status>>
                + Send,
        >,
    >;

    async fn manage_dead_lettered_events(
        &self,
        _: Request<Streaming<ManageDeadLetteredEventsRequest>>,
    ) -> Result<Response<Self::ManageDeadLetteredEventsStream>, Status> {
        Err(Status::unimplemented("not used"))
    }
}

async fn start_consumer_service(
    sessions: VecDeque<Vec<(i64, Event)>>,
) -> (SocketAddr, Arc<Mutex<ConsumerLog>>) {
    start_scripted_consumer(ConsumerScript {
        sessions,
        ..ConsumerScript::default()
    })
    .await
}

async fn start_scripted_consumer(script: ConsumerScript) -> (SocketAddr, Arc<Mutex<ConsumerLog>>) {
    let log = Arc::new(Mutex::new(ConsumerLog::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock consumer must bind");
    let addr = listener.local_addr().expect("bound addr");

    let service = ScriptedConsumer {
        script: Mutex::new(script.sessions),
        drip: script.drip,
        accept_at_most: script.accept_at_most,
        reject_first: Mutex::new(script.reject_first),
        defer_headers: script.defer_headers,
        log: log.clone(),
    };
    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(EventConsumerServiceServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await;
    });

    (addr, log)
}

// --- scripted processor ----------------------------------------------------

/// A processor whose verdict is fixed, so a test can drive the acknowledgement
/// matrix without doing any image work.
struct ScriptedProcessor {
    retryable: bool,
    calls: Arc<AtomicUsize>,
    /// Held for the duration of each call, so a test can observe how many run at
    /// once.
    concurrent: Arc<AtomicUsize>,
    peak_concurrent: Arc<AtomicUsize>,
    hold: Duration,
}

#[async_trait::async_trait]
impl MessageProcessor for ScriptedProcessor {
    async fn process(&self, _message: &prost_types::Struct) -> WorkOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_concurrent.fetch_max(now, Ordering::SeqCst);

        tokio::time::sleep(self.hold).await;

        self.concurrent.fetch_sub(1, Ordering::SeqCst);
        if self.retryable {
            WorkOutcome::retryable(FailureClass::Upload, "dest", "storage unavailable")
        } else {
            WorkOutcome::Succeeded {
                destination: "dest".to_string(),
            }
        }
    }
}

// --- harness ---------------------------------------------------------------

fn generate_event() -> Event {
    Event {
        event_type: GENERATE_EVENT_TYPE.to_string(),
        principal_identity: "task-worker-tests".to_string(),
        occurred_at: None,
        published_at: None,
        // The scripted processor never parses it, so an empty message is enough
        // to exercise the delivery bookkeeping under test.
        message: Some(prost_types::Struct::default()),
    }
}

fn storage_event() -> Event {
    Event {
        event_type: "omni.storage.created".to_string(),
        ..generate_event()
    }
}

fn consumer(
    addr: SocketAddr,
    processor: Arc<dyn MessageProcessor>,
    configure: impl FnOnce(&mut thumbnail_task_worker::config::WorkerArgs),
) -> (Consumer, Arc<WorkerMetrics>) {
    let (consumer, metrics, _health) = consumer_with_health(addr, processor, configure);
    (consumer, metrics)
}

/// As `consumer`, but hands back the health state so a test can assert on
/// readiness rather than only on deliveries.
fn consumer_with_health(
    addr: SocketAddr,
    processor: Arc<dyn MessageProcessor>,
    configure: impl FnOnce(&mut thumbnail_task_worker::config::WorkerArgs),
) -> (Consumer, Arc<WorkerMetrics>, Arc<HealthState>) {
    let mut args = thumbnail_task_worker::config::WorkerArgs::for_tests();
    configure(&mut args);
    let provisioning = args
        .resolve_queue_provisioning()
        .expect("test args carry a default resource prefix");
    let queue_id = provisioning.queue_id;
    // Same groups startup provisions with, so a NOT_FOUND re-provision in the
    // consume loop recreates the subscription it actually had.
    let filter_groups = thumbnail_common::notifications::provisioning::build_generate_filter_groups(
        &provisioning.resource_prefixes,
    );

    let outbound = thumbnail_common::OutboundClients::for_tests();
    let metrics = Arc::new(WorkerMetrics::new_for_tests());
    let health = Arc::new(HealthState::default());
    // Instant restore rather than `TokenManager::refresh` against the fixture's
    // unreachable token URL — otherwise `TokenExpiring` hangs before reconnect.
    let refresher: Arc<dyn thumbnail_auth::TokenRefresher> =
        Arc::new(TestTokenRefresher(outbound.token_gate.clone()));

    (
        Consumer {
            args,
            queue_id,
            filter_groups: filter_groups.clone(),
            consumer_channel: tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
                .expect("valid endpoint")
                .connect_lazy(),
            interceptor: outbound.interceptor.clone(),
            refresher,
            token_gate: outbound.token_gate.clone(),
            pipeline: processor,
            metrics: metrics.clone(),
            health: health.clone(),
        },
        metrics,
        health,
    )
}

/// As `consumer_with_health`, but with an injected refresher (e.g. one that
/// cannot clear expiry).
fn consumer_with_sticky_refresher(
    addr: SocketAddr,
    processor: Arc<dyn MessageProcessor>,
    configure: impl FnOnce(&mut thumbnail_task_worker::config::WorkerArgs),
) -> (Consumer, Arc<WorkerMetrics>) {
    let mut args = thumbnail_task_worker::config::WorkerArgs::for_tests();
    configure(&mut args);
    let provisioning = args
        .resolve_queue_provisioning()
        .expect("test args carry a default resource prefix");
    let queue_id = provisioning.queue_id;
    let filter_groups = thumbnail_common::notifications::provisioning::build_generate_filter_groups(
        &provisioning.resource_prefixes,
    );

    let outbound = thumbnail_common::OutboundClients::for_tests();
    let metrics = Arc::new(WorkerMetrics::new_for_tests());
    let health = Arc::new(HealthState::default());
    let refresher: Arc<dyn thumbnail_auth::TokenRefresher> = Arc::new(StickyExpiringRefresher);

    (
        Consumer {
            args,
            queue_id,
            filter_groups: filter_groups.clone(),
            consumer_channel: tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
                .expect("valid endpoint")
                .connect_lazy(),
            interceptor: outbound.interceptor.clone(),
            refresher,
            token_gate: outbound.token_gate.clone(),
            pipeline: processor,
            metrics: metrics.clone(),
            health,
        },
        metrics,
    )
}

/// Restores a fresh bearer without talking to a token endpoint.
struct TestTokenRefresher(thumbnail_common::auth::TokenGate);

#[tonic::async_trait]
impl thumbnail_auth::TokenRefresher for TestTokenRefresher {
    async fn refresh(&self) {
        self.0.restore_fresh_token_for_tests();
    }
}

/// Leaves the gate unchanged — models a sick IdP or a sidecar file still inside
/// its skew window, where `refresh()` cannot clear `is_expiring_soon()`.
struct StickyExpiringRefresher;

#[tonic::async_trait]
impl thumbnail_auth::TokenRefresher for StickyExpiringRefresher {
    async fn refresh(&self) {}
}

fn scripted(
    retryable: bool,
    hold: Duration,
) -> (Arc<ScriptedProcessor>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(ScriptedProcessor {
        retryable,
        calls: calls.clone(),
        concurrent: Arc::new(AtomicUsize::new(0)),
        peak_concurrent: peak.clone(),
        hold,
    });
    (processor, calls, peak)
}

/// Run the consumer until `run_for` elapses, then shut it down and return.
async fn run_briefly(consumer: Consumer, run_for: Duration) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(run_for).await;
    shut_down(shutdown_tx, task).await;
}

/// Signal shutdown and assert the consumer actually stopped.
///
/// Failures to stop are the test's problem, not something to discard: a consumer
/// that ignores shutdown, or panics on the way out, would otherwise leave every
/// assertion above it passing on a worker that never terminated. Shared so a
/// test that drives its own lifecycle cannot quietly hold itself to a lower
/// standard than `run_briefly` does.
async fn shut_down(shutdown_tx: watch::Sender<bool>, task: tokio::task::JoinHandle<()>) {
    shutdown_tx
        .send(true)
        .expect("the consumer task should still be holding the shutdown receiver");

    match tokio::time::timeout(Duration::from_secs(5), task).await {
        Ok(Ok(())) => {}
        Ok(Err(join_error)) => panic!("the consumer task ended abnormally: {join_error}"),
        Err(_) => panic!("the consumer did not stop within 5s of the shutdown signal"),
    }
}

// --- tests -----------------------------------------------------------------

/// Readiness must not wait for work to arrive.
///
/// Notification withholds response headers on
/// `ConsumeDurableEventsWithAcks` until it has a delivery to write, so a client
/// that treats "the call returned" as "the stream is established" learns nothing
/// on an idle queue. WorkerEventQueue is idle at startup by definition — the first
/// generate event cannot arrive until something is uploaded — so the worker
/// reported not-ready indefinitely, failed its readiness gate, and the release
/// never completed. Every other test here passes either way, because the double
/// used to answer immediately.
#[tokio::test]
async fn the_worker_reports_ready_on_an_idle_queue() {
    let (addr, _log) = start_scripted_consumer(ConsumerScript {
        // No deliveries and headers withheld: a queue nobody has published to.
        defer_headers: true,
        ..ConsumerScript::default()
    })
    .await;
    let (processor, _calls, _) = scripted(false, Duration::ZERO);
    let (consumer, _metrics, health) = consumer_with_health(addr, processor, |_| {});

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let ready = health.is_ready();
    shut_down(shutdown_tx, task).await;

    assert!(
        ready,
        "a worker attached to an idle queue is consuming and must report ready; \
         waiting for the first delivery to prove it never terminates"
    );
}

#[tokio::test]
async fn a_successful_delivery_is_acknowledged() {
    let (addr, log) = start_consumer_service(VecDeque::from(vec![vec![
        (11, generate_event()),
        (12, generate_event()),
    ]]))
    .await;
    let (processor, calls, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |_| {});

    run_briefly(consumer, Duration::from_millis(600)).await;

    assert_eq!(calls.load(Ordering::SeqCst), 2, "both deliveries processed");
    let acked = log.lock().expect("log").acked.clone();
    assert!(
        acked.contains(&11) && acked.contains(&12),
        "acked: {acked:?}"
    );
}

/// Task 6.3 — retryable failure: NACK is sent; stream stays open.
///
/// With the old recycle-timer approach, the worker would withhold the
/// acknowledgement and arm a timer; after the interval it would tear the
/// stream down and reconnect. The new approach sends an explicit NACK so the
/// broker can redeliver at its own pace — the stream stays open and no recycle
/// metric is incremented.
///
/// The delay is set short so the NACK observed here is the one the redelivery
/// timer sent, not the teardown flush.
#[tokio::test]
async fn a_retryable_failure_sends_nack_without_recycling_the_stream() {
    let script = VecDeque::from(vec![vec![(21, generate_event())]]);
    let (addr, log) = start_consumer_service(script).await;
    let (processor, calls, _) = scripted(true, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        a.redelivery_recycle_interval = Duration::from_millis(100);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    await_log(
        &log,
        "delivery 21 to be NACKed once its delay elapsed",
        |l| l.nacked.contains(&21),
    )
    .await;

    {
        let log = log.lock().expect("log");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the event was processed exactly once"
        );
        assert!(log.acked.is_empty(), "a retryable failure must never ACK");
        assert_eq!(log.sessions, 1, "NACK must not recycle the stream");
    }
    let recycles = counter(&metrics.render(), "thumbnail_worker_stream_recycles_total");
    assert_eq!(
        recycles, 0,
        "NACK-based redelivery keeps the stream open; saw {recycles} recycle(s)"
    );

    shut_down(shutdown_tx, task).await;
}

/// Spaced, not spun. The broker requeues a negatively acknowledged delivery
/// immediately, so an undeferred NACK turns a persistently failing conversion
/// into a hot retry loop — a corrupt asset, or storage being down, retried as
/// fast as the network allows. The recycle timer this replaced supplied ~60s of
/// spacing as a side effect of reconnecting; nothing may be faster than that now.
///
/// The teardown assertion is the other half: holding a NACK must never mean
/// losing it, or the delivery stays in-flight until the broker's acknowledgement
/// timeout cancels the consumer, which is the stall this change exists to fix.
#[tokio::test]
async fn a_retryable_failure_is_spaced_and_still_flushed_on_teardown() {
    let script = VecDeque::from(vec![vec![(41, generate_event())]]);
    let (addr, log) = start_consumer_service(script).await;
    let (processor, calls, _) = scripted(true, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        // Far longer than this test runs, so any NACK seen below is unspaced.
        a.redelivery_recycle_interval = Duration::from_secs(3600);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_millis(800)).await;

    {
        let log = log.lock().expect("log");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one delivery, converted once"
        );
        assert!(
            log.nacked.is_empty(),
            "the NACK must wait out the redelivery delay rather than going straight \
             back to a broker that requeues immediately; saw nacked: {:?}",
            log.nacked,
        );
        assert_eq!(
            log.sessions, 1,
            "holding a NACK must not recycle the stream"
        );
    }
    assert_eq!(
        counter(&metrics.render(), "thumbnail_worker_stream_recycles_total"),
        0,
        "spacing is not implemented by reconnecting any more"
    );

    shut_down(shutdown_tx, task).await;

    await_log(
        &log,
        "the held NACK for delivery 41 to be flushed on teardown rather than \
         abandoned with the delivery still in flight",
        |l| l.nacked.contains(&41),
    )
    .await;
}

/// `thumbnail_worker_unacknowledged_deliveries` is incremented on receipt and
/// decremented only by an acknowledgement. A NACK ends the in-flight state just
/// as an ACK does, so it has to release the gauge — and now that the recycle is
/// rare, nothing else resets it, so a NACK that left it alone would make the
/// gauge climb monotonically under sustained retryable failure.
#[tokio::test]
async fn a_nacked_delivery_releases_the_unacknowledged_gauge() {
    let script = VecDeque::from(vec![vec![(51, generate_event())]]);
    let (addr, _log) = start_consumer_service(script).await;
    let (processor, _, _) = scripted(true, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        a.redelivery_recycle_interval = Duration::from_millis(100);
    });

    run_briefly(consumer, Duration::from_millis(600)).await;

    assert_eq!(
        metrics.unacknowledged(),
        0,
        "a NACKed delivery is no longer in flight, so the gauge must come back down"
    );
    let rendered = metrics.render();
    assert_eq!(
        labelled_counter_sum(&rendered, "thumbnail_worker_nacks_total"),
        1,
        "the retry stays visible as a NACK count: {rendered}"
    );
    // Attributed to the cause, not just counted: a conversion that failed is a
    // different operational condition from a redelivery arriving early, and the
    // unlabelled count could not tell them apart.
    assert!(
        rendered.contains("thumbnail_worker_nacks_total{reason=\"retryable_failure\"} 1"),
        "a failed conversion must be attributed to retryable_failure: {rendered}"
    );
    assert!(
        rendered.contains("thumbnail_worker_nacks_total{reason=\"early_redelivery\"} 0"),
        "nothing arrived early here, so that series must be present and zero: {rendered}"
    );
}

/// Sustained concurrent failure must not wedge the consume loop.
///
/// This is a regression test for a freeze observed in dev on 2026-08-31: with
/// `thumbnail-service` scaled to zero, one worker stopped dead — 16 retryable
/// failures but only 5 NACKs delivered, 12 deliveries in flight, and no log
/// output for eleven minutes. It never recovered; the pod had to be deleted.
///
/// The shape that matters is a conversion task handing its delivery tag to the
/// main loop *while still holding its semaphore permit*, when the main loop is
/// also the only thing that drains that handoff and is itself waiting for a
/// permit. Anything that makes the handoff block can therefore stop the loop
/// permanently. This drives far more concurrent failures than permits so the
/// handoff is under real pressure, and asserts the loop keeps settling them.
#[tokio::test]
async fn sustained_concurrent_failures_do_not_wedge_the_loop() {
    const DELIVERIES: i64 = 40;

    let script: Vec<(i64, Event)> = (1..=DELIVERIES).map(|t| (t, generate_event())).collect();
    let (addr, log) = start_consumer_service(VecDeque::from(vec![script])).await;
    // A conversion that takes a little time keeps permits held while further
    // deliveries arrive, which is what puts the handoff under pressure.
    let (processor, _calls, _) = scripted(true, Duration::from_millis(40));
    let (consumer, metrics) = consumer(addr, processor, |a| {
        a.max_concurrent_conversions = 4;
        // Short so the deferred NACKs come due inside the test, and well under
        // the deferral bound so both the held and the immediate paths are used.
        a.redelivery_recycle_interval = Duration::from_millis(500);
        a.nack_deferral_concurrency_limit = 10;
        // High enough that the circuit breaker does not fire and recycle the
        // session — a recycle rebuilds the loop and would mask a wedge.
        a.nack_early_redelivery_limit = 100_000;
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // Every delivery must be settled by a NACK. A wedged loop stops partway and
    // this times out, which is exactly how the dev freeze presented.
    await_log(
        &log,
        "every failing delivery to be negatively acknowledged",
        |l| {
            let mut seen: Vec<i64> = l.nacked.clone();
            seen.sort_unstable();
            seen.dedup();
            seen.len() as i64 >= DELIVERIES
        },
    )
    .await;

    assert_eq!(
        counter(&metrics.render(), "thumbnail_worker_stream_recycles_total"),
        0,
        "the loop must stay healthy under load, not recycle its way out: {}",
        metrics.render()
    );

    shut_down(shutdown_tx, task).await;
}

/// A deleted WorkerEventQueue is re-provisioned by the running worker.
///
/// Reconnecting cannot fix this on its own: a queue deleted underneath the
/// consumer stays deleted, so the generic backoff path reconnects forever against
/// a queue that is gone and only a pod restart recovers. That is the 2026-08-25
/// incident, which deleted five queues in one client namespace, and it was fixed
/// for StorageEventQueue while WorkerEventQueue still had it.
#[tokio::test]
async fn a_deleted_queue_is_reprovisioned_without_a_restart() {
    let (addr, log) = start_scripted_consumer(ConsumerScript {
        // First session fails NOT_FOUND, as the platform answers a consume
        // against a queue that no longer exists.
        reject_first: VecDeque::from(vec![tonic::Code::NotFound]),
        // The session after recovery carries real work, so the assertion is that
        // the worker consumes again rather than merely stops erroring.
        sessions: VecDeque::from(vec![vec![], vec![(71, generate_event())]]),
        ..ConsumerScript::default()
    })
    .await;
    let (processor, calls, _) = scripted(false, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        // Keep the post-recovery reconnect prompt; recovery itself takes no backoff.
        a.reconnect_base_backoff_seconds = 1;
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    await_log(
        &log,
        "the worker to re-provision its queue and then acknowledge real work",
        |l| !l.queues_created.is_empty() && l.acked.contains(&71),
    )
    .await;

    {
        let log = log.lock().expect("log");
        assert!(
            log.queues_created
                .iter()
                .all(|id| id.starts_with("thumbnail-service-worker-")),
            "re-provisioning must recreate the worker's own queue, got: {:?}",
            log.queues_created
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the delivery after recovery must be processed"
        );
    }
    assert_eq!(
        counter(&metrics.render(), "thumbnail_worker_stream_recycles_total"),
        0,
        "re-provisioning is not a recycle"
    );

    shut_down(shutdown_tx, task).await;
}

/// Read one unlabelled counter out of the exposition, so a test can assert on a
/// value rather than on the presence of a substring.
fn counter(rendered: &str, name: &str) -> u64 {
    rendered
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix(name)?.trim().parse().ok())
        .unwrap_or_else(|| panic!("{name} is missing from the exposition:\n{rendered}"))
}

/// Sum one labelled family across its label values, so a test can assert on a
/// total without restating every label the metric happens to carry.
fn labelled_counter_sum(rendered: &str, name: &str) -> u64 {
    let samples: Vec<u64> = rendered
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let rest = line.strip_prefix(name)?;
            // Labelled samples only: `name{...} value`.
            let value = rest.strip_prefix('{')?.split_once('}')?.1;
            value.trim().parse().ok()
        })
        .collect();
    assert!(
        !samples.is_empty(),
        "{name} has no labelled samples in the exposition:\n{rendered}"
    );
    samples.iter().sum()
}

fn expire_test_token(gate: &thumbnail_common::auth::TokenGate) {
    gate.clear_cached_token_for_tests();
}

/// A worker with all successful conversions must not churn its stream.
/// NACK-based redelivery and the recycle are both edge-triggered off retryable
/// failures; a quiet healthy stream stays open on a single session.
#[tokio::test]
async fn a_healthy_worker_does_not_recycle() {
    let (addr, log) =
        start_consumer_service(VecDeque::from(vec![vec![(31, generate_event())]])).await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |_| {});

    run_briefly(consumer, Duration::from_millis(1_000)).await;

    let log = log.lock().expect("log");
    assert_eq!(log.sessions, 1, "a healthy worker never recycles");
    assert!(
        log.nacked.is_empty(),
        "no retryable failures means no NACKs"
    );
}

/// WorkerEventQueue filters on the generate type. Anything else arriving is a
/// provisioning mistake, and processing it would be worse than ignoring it.
#[tokio::test]
async fn a_storage_event_is_ignored_rather_than_processed() {
    let (addr, log) =
        start_consumer_service(VecDeque::from(vec![vec![(41, storage_event())]])).await;
    let (processor, calls, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |_| {});

    run_briefly(consumer, Duration::from_millis(600)).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the worker must not process storage CREATE events"
    );
    assert!(
        log.lock().expect("log").acked.contains(&41),
        "an event this queue should not carry is acknowledged, not redelivered forever"
    );
}

/// Admission control has to actually bound in-flight work: without it the
/// unacknowledged set grows until the queue hits its maximum length and the
/// platform dead-letters the overflow.
#[tokio::test]
async fn concurrent_conversions_are_capped() {
    let deliveries: Vec<(i64, Event)> = (0..8).map(|n| (100 + n, generate_event())).collect();
    let (addr, _log) = start_consumer_service(VecDeque::from(vec![deliveries])).await;
    let (processor, calls, peak) = scripted(false, Duration::from_millis(150));
    let (consumer, _) = consumer(addr, processor, |a| a.max_concurrent_conversions = 2);

    run_briefly(consumer, Duration::from_millis(1_500)).await;

    assert!(
        calls.load(Ordering::SeqCst) > 2,
        "the cap should slow the worker down, not stop it"
    );
    assert!(
        peak.load(Ordering::SeqCst) <= 2,
        "at most 2 conversions may run at once, saw {}",
        peak.load(Ordering::SeqCst)
    );
}

/// A quiet healthy stream stays open. Silence alone must not force a reconnect
/// — that matches the Notification consumer already approved in RC. Recycle is
/// disarmed (nothing withheld); token stays fresh.
#[tokio::test]
async fn a_quiet_stream_is_not_rebuilt_for_silence() {
    let (addr, log) = start_consumer_service(VecDeque::new()).await;
    let (processor, calls, _) = scripted(false, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        // Far out of the way, so nothing here can be credited to the recycle.
        a.redelivery_recycle_interval = Duration::from_secs(3600);
    });

    // Longer than the former idle bound tests used (2s), so a regression that
    // reintroduced silence-based rebuild would open a second session here.
    run_briefly(consumer, Duration::from_millis(4_500)).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the stream never delivered anything"
    );
    assert_eq!(
        log.lock().expect("log").sessions,
        1,
        "a quiet healthy stream must stay open, saw {} session(s)",
        log.lock().expect("log").sessions
    );
    assert_eq!(
        counter(&metrics.render(), "thumbnail_worker_stream_recycles_total"),
        0,
        "nothing was withheld: {}",
        metrics.render()
    );
}

/// Readiness has to follow the stream rather than the process. Here the first
/// session is accepted, then the next establish is refused, so readiness must
/// clear rather than remaining true on the strength of the earlier session.
///
/// The tear-down trigger is token near expiry (peer-aligned), not silence.
#[tokio::test]
async fn readiness_does_not_survive_a_stream_that_cannot_be_rebuilt() {
    let (addr, _log) = start_scripted_consumer(ConsumerScript {
        accept_at_most: Some(1),
        ..ConsumerScript::default()
    })
    .await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |a| {
        a.redelivery_recycle_interval = Duration::from_secs(3600);
        // Long enough that the worker is still backing off from the refused
        // rebuild — and so still not consuming — when readiness is sampled.
        a.reconnect_base_backoff_seconds = 30;
    });
    let health = consumer.health.clone();
    let token_gate = consumer.token_gate.clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        health.is_ready(),
        "the first session was accepted, so the worker should report Ready"
    );

    // Force the peer-aligned reopen path: clear the cached bearer so the next
    // housekeeping tick tears the session down for token expiry.
    expire_test_token(&token_gate);

    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        !health.is_ready(),
        "the stream was torn down for token expiry and could not be rebuilt, so the \
         replica must not still be counted as consuming"
    );
    assert!(
        health.is_live(),
        "liveness is deliberately weaker: restarting the pod abandons in-flight \
         work and does not fix an unreachable dependency"
    );

    shut_down(shutdown_tx, task).await;
}

/// Refreshing does not always help — a sick token endpoint (or a sidecar file
/// still inside its skew window) must not turn the expiry check into a 1 Hz
/// teardown loop.
#[tokio::test]
async fn a_refresh_that_does_not_clear_expiry_backs_off() {
    let (addr, log) = start_consumer_service(VecDeque::new()).await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, metrics) = consumer_with_sticky_refresher(addr, processor, |a| {
        a.redelivery_recycle_interval = Duration::from_secs(3600);
        // Long enough that a second reopen after a failed refresh cannot be
        // mistaken for immediate reconnect: without the guard the 1s tick would
        // open several sessions in this window.
        a.reconnect_base_backoff_seconds = 5;
    });
    let token_gate = consumer.token_gate.clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(log.lock().expect("log").sessions, 1);

    expire_test_token(&token_gate);
    tokio::time::sleep(Duration::from_millis(3_500)).await;

    let sessions = log.lock().expect("log").sessions;
    shut_down(shutdown_tx, task).await;

    assert!(
        sessions <= 2,
        "a refresh that leaves the token expiring must back off rather than \
         reopen once a second; saw {sessions} session(s)"
    );
    assert_eq!(
        counter(
            &metrics.render(),
            "thumbnail_worker_stream_token_reconnects_total"
        ),
        0,
        "failed refreshes must not count as successful token reconnects: {}",
        metrics.render()
    );
}

/// Near-expiry must reopen without waiting for an on-wire UNAUTHENTICATED.
#[tokio::test]
async fn token_near_expiry_reopens_the_stream() {
    let (addr, log) = start_consumer_service(VecDeque::new()).await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, metrics) = consumer(addr, processor, |a| {
        a.redelivery_recycle_interval = Duration::from_secs(3600);
    });
    let token_gate = consumer.token_gate.clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        log.lock().expect("log").sessions,
        1,
        "first session should be established before expiry is forced"
    );

    expire_test_token(&token_gate);

    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        log.lock().expect("log").sessions >= 2,
        "token near expiry must reopen the stream, saw {} session(s)",
        log.lock().expect("log").sessions
    );
    assert!(
        counter(
            &metrics.render(),
            "thumbnail_worker_stream_token_reconnects_total"
        ) >= 1,
        "a successful near-expiry reopen must be counted: {}",
        metrics.render()
    );

    shut_down(shutdown_tx, task).await;
}

/// An expired token is the ordinary reason a stream dies, and the refresh that
/// follows has already fixed it — so the retry does not wait. Sleeping through a
/// backoff here extends an outage whose cause is gone.
#[tokio::test]
async fn a_refreshed_token_reconnects_without_waiting_out_a_backoff() {
    let (addr, log) = start_scripted_consumer(ConsumerScript {
        reject_first: VecDeque::from(vec![tonic::Code::Unauthenticated]),
        ..ConsumerScript::default()
    })
    .await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |a| {
        // Far longer than the run below, so a second session can only mean the
        // backoff was skipped rather than waited out.
        a.reconnect_base_backoff_seconds = 30;
    });

    run_briefly(consumer, Duration::from_millis(1_500)).await;

    assert!(
        log.lock().expect("log").sessions >= 2,
        "a refreshed token should be retried immediately, saw {} session(s)",
        log.lock().expect("log").sessions
    );
}

/// But only the first one. A refresh that keeps being rejected means a revoked
/// credential or a changed policy, and retrying flat out would hammer the token
/// endpoint and Notification for as long as that lasts.
#[tokio::test]
async fn repeated_auth_rejection_falls_back_to_backing_off() {
    let (addr, log) = start_scripted_consumer(ConsumerScript {
        reject_first: VecDeque::from(vec![
            tonic::Code::Unauthenticated,
            tonic::Code::Unauthenticated,
            tonic::Code::Unauthenticated,
        ]),
        ..ConsumerScript::default()
    })
    .await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |a| {
        a.reconnect_base_backoff_seconds = 30;
    });

    run_briefly(consumer, Duration::from_millis(1_500)).await;

    assert_eq!(
        log.lock().expect("log").sessions,
        2,
        "the first rejection retries at once and the second must then back off, \
         so exactly two sessions fit in the window"
    );
}

/// Dropping every sender is shutdown, not something to ignore. `changed()`
/// returns `Err` the moment the last one goes and forever after, so discarding
/// it left a `select!` arm permanently ready — the loop spinning while the last
/// observed value was still `false`, and the worker never stopping.
#[tokio::test]
async fn a_dropped_shutdown_sender_stops_the_consumer() {
    let (addr, _log) = start_consumer_service(VecDeque::new()).await;
    let (processor, _, _) = scripted(false, Duration::ZERO);
    let (consumer, _) = consumer(addr, processor, |_| {});

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(shutdown_tx);

    match tokio::time::timeout(Duration::from_secs(5), task).await {
        Ok(Ok(())) => {}
        Ok(Err(join_error)) => panic!("the consumer task ended abnormally: {join_error}"),
        Err(_) => panic!("a closed shutdown channel must stop the consumer, not spin it"),
    }
}

/// A slow conversion must not hold up the acknowledgement of ones that already
/// finished — the ack stream is shared and must not be blocked behind it.
#[tokio::test]
async fn a_slow_conversion_does_not_block_other_acknowledgements() {
    let deliveries: Vec<(i64, Event)> = (0..4).map(|n| (200 + n, generate_event())).collect();
    let (addr, log) = start_consumer_service(VecDeque::from(vec![deliveries])).await;
    let (processor, _, _) = scripted(false, Duration::from_millis(100));
    let (consumer, _) = consumer(addr, processor, |a| a.max_concurrent_conversions = 4);

    run_briefly(consumer, Duration::from_millis(900)).await;

    let acked = log.lock().expect("log").acked.clone();
    assert_eq!(
        acked.len(),
        4,
        "every completed delivery should be acknowledged independently, saw {acked:?}"
    );
}

// --- liveness at the concurrency bound -------------------------------------
//
// Conversion capacity used to be acquired inside the `stream.next()` arm of the
// `select!`. `select!` runs the winning arm to completion, so once a delivery had
// been read and no permit was free, that await stopped every other arm from being
// polled: shutdown, the token-expiry check, and the deferred-NACK deadline all
// stopped happening until a conversion finished. Capacity is now reserved before
// a delivery is read, in an arm of its own.
//
// Each of these tests saturates the bound with a conversion far longer than the
// thing being asserted, so a regression shows up as a timeout rather than as a
// flake.

/// Shutdown is observed while the loop is waiting for capacity.
///
/// One permit, a three-second conversion, and more deliveries queued behind it,
/// so the loop is parked wanting capacity it cannot have. The drain budget is
/// deliberately tiny: it isolates *observing* the shutdown from *finishing* the
/// in-flight work, which is what would otherwise dominate the measurement.
#[tokio::test]
async fn shutdown_is_observed_while_saturated() {
    let script: Vec<(i64, Event)> = (1..=4).map(|t| (t, generate_event())).collect();
    let (addr, _log) = start_consumer_service(VecDeque::from(vec![script])).await;
    let (processor, calls, _) = scripted(false, Duration::from_secs(3));
    let (consumer, _) = consumer(addr, processor, |a| {
        a.max_concurrent_conversions = 1;
        a.shutdown_drain_timeout = Duration::from_millis(100);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // Let the first delivery take the only permit and park the loop.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "one permit means exactly one conversion in flight"
    );

    let signalled = std::time::Instant::now();
    shutdown_tx.send(true).expect("consumer holds the receiver");
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("shutdown must be observed without waiting for the conversion")
        .expect("the consumer task ended abnormally");

    assert!(
        signalled.elapsed() < Duration::from_millis(1_500),
        "shutdown took {:?}; the 3s conversion should not have gated it",
        signalled.elapsed()
    );
}

/// The token-expiry check still runs while the loop is waiting for capacity.
///
/// Expiry is what reopens the session, and the load that keeps every permit busy
/// is exactly when a stale bearer matters — so this check going quiet under
/// saturation is the worst possible time for it to do so.
#[tokio::test]
async fn token_expiry_is_checked_while_saturated() {
    let script: Vec<(i64, Event)> = (1..=6).map(|t| (t, generate_event())).collect();
    let (addr, log) = start_consumer_service(VecDeque::from(vec![script])).await;
    let (processor, _calls, _) = scripted(false, Duration::from_secs(5));
    let (consumer, _metrics) = consumer(addr, processor, |a| {
        a.max_concurrent_conversions = 1;
        a.shutdown_drain_timeout = Duration::from_millis(100);
        // Reopen promptly once expiry is seen, so the new session is observable
        // well inside the 5s conversion.
        a.reconnect_base_backoff_seconds = 1;
    });
    let token_gate = consumer.token_gate.clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(log.lock().expect("log").sessions, 1);

    expire_test_token(&token_gate);

    // The housekeeping tick is 1s. A second session inside 3s proves the tick was
    // polled while the only permit was held by a 5s conversion.
    await_log(
        &log,
        "the session to reopen on token expiry while saturated",
        |l| l.sessions >= 2,
    )
    .await;

    shut_down(shutdown_tx, task).await;
}

/// A deferred NACK comes due on time while the loop is waiting for capacity.
///
/// This is the redelivery path. If the deadline arm is not polled, a retryable
/// failure's NACK sits held until capacity frees, and the delivery is not
/// redelivered for as long as the worker stays busy.
#[tokio::test]
async fn a_deferred_nack_fires_while_saturated() {
    let script: Vec<(i64, Event)> = (1..=4).map(|t| (t, generate_event())).collect();
    let (addr, log) = start_consumer_service(VecDeque::from(vec![script])).await;
    // Retryable, so each conversion defers a NACK; long, so the permit stays held.
    let (processor, _calls, _) = scripted(true, Duration::from_secs(4));
    let (consumer, _) = consumer(addr, processor, |a| {
        a.max_concurrent_conversions = 1;
        a.shutdown_drain_timeout = Duration::from_millis(100);
        // Comes due well inside the 4s conversion that follows it.
        a.redelivery_recycle_interval = Duration::from_millis(300);
        a.nack_deferral_concurrency_limit = 10;
        a.nack_early_redelivery_limit = 100_000;
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // The first conversion finishes at ~4s and defers its NACK for 300ms. The
    // second delivery then holds the permit for another 4s. The NACK must be sent
    // during that, not after it.
    await_log(
        &log,
        "a deferred NACK to be sent while a conversion holds the permit",
        |l| !l.nacked.is_empty(),
    )
    .await;

    shut_down(shutdown_tx, task).await;
}

// --- shutdown drain and the panic path --------------------------------------

/// A processor whose verdict is fixed but which can be told to panic, or to hold
/// for a chosen duration, so the drain budget and the panic arm can be driven.
struct DrainProcessor {
    hold: Duration,
    panic_on_call: bool,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl MessageProcessor for DrainProcessor {
    async fn process(&self, _message: &prost_types::Struct) -> WorkOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.panic_on_call {
            panic!("scripted conversion panic");
        }
        tokio::time::sleep(self.hold).await;
        WorkOutcome::Succeeded {
            destination: "dest".to_string(),
        }
    }
}

/// A conversion that finishes inside the drain budget gets acknowledged.
///
/// The drain exists so acknowledgements for work that completed actually reach
/// the server before the stream drops. Without it, work that succeeded would come
/// back and be redone, which is waste rather than incorrectness — but it is
/// avoidable waste and the loop is written to avoid it.
#[tokio::test]
async fn a_conversion_finishing_during_drain_is_acknowledged() {
    let (addr, log) =
        start_consumer_service(VecDeque::from(vec![vec![(71, generate_event())]])).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(DrainProcessor {
        // Longer than the pause before shutdown, so the conversion is still in
        // flight when the signal arrives, and well inside the drain budget.
        hold: Duration::from_millis(600),
        panic_on_call: false,
        calls: calls.clone(),
    });
    let (consumer, _) = consumer(addr, processor, |a| {
        a.shutdown_drain_timeout = Duration::from_secs(5);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // Let the delivery start converting, then shut down mid-conversion.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the conversion must be in flight"
    );
    shut_down(shutdown_tx, task).await;

    let log = log.lock().expect("log");
    assert!(
        log.acked.contains(&71),
        "a conversion that finished inside the drain budget must be acknowledged, \
         or the work is redone for nothing; acked: {:?}",
        log.acked
    );
}

/// A conversion still running when the drain budget expires is abandoned
/// unacknowledged, and the worker still stops.
///
/// Unacknowledged is the correct outcome: the work did not finish, so it comes
/// back. What must not happen is the worker hanging on it — the budget is there to
/// bound shutdown, and a pod that ignores SIGTERM gets killed instead.
#[tokio::test]
async fn a_conversion_exceeding_the_drain_budget_is_abandoned_not_acknowledged() {
    let (addr, log) =
        start_consumer_service(VecDeque::from(vec![vec![(72, generate_event())]])).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(DrainProcessor {
        // Far longer than the drain budget below.
        hold: Duration::from_secs(10),
        panic_on_call: false,
        calls: calls.clone(),
    });
    let (consumer, _) = consumer(addr, processor, |a| {
        a.shutdown_drain_timeout = Duration::from_millis(200);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let signalled = std::time::Instant::now();
    // `shut_down` allows 5s, which is generous against a 200ms budget and a 10s
    // conversion: passing means the budget bounded it, not that the work finished.
    shut_down(shutdown_tx, task).await;
    assert!(
        signalled.elapsed() < Duration::from_secs(5),
        "the drain budget must bound shutdown; took {:?}",
        signalled.elapsed()
    );

    let log = log.lock().expect("log");
    assert!(
        !log.acked.contains(&72),
        "work that did not finish must not be acknowledged; acked: {:?}",
        log.acked
    );
}

/// A panicking conversion recycles the session rather than wedging the loop.
///
/// A panicked task leaves no delivery tag to NACK, so the redelivery has to come
/// from the reconnect. The arm that does this had no test: a regression would have
/// left the delivery unresolved with nothing to force it back.
#[tokio::test]
async fn a_panicking_conversion_recycles_the_session() {
    let (addr, log) = start_consumer_service(VecDeque::from(vec![
        vec![(73, generate_event())],
        vec![(74, generate_event())],
    ]))
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(DrainProcessor {
        hold: Duration::ZERO,
        panic_on_call: true,
        calls: calls.clone(),
    });
    let (consumer, metrics) = consumer(addr, processor, |a| {
        a.reconnect_base_backoff_seconds = 1;
        a.shutdown_drain_timeout = Duration::from_millis(100);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    await_log(
        &log,
        "the session to recycle after a panicked conversion",
        |l| l.sessions >= 2,
    )
    .await;

    let recycles = counter(&metrics.render(), "thumbnail_worker_stream_recycles_total");
    shut_down(shutdown_tx, task).await;

    assert!(
        recycles >= 1,
        "the panic must be counted as a recycle, not silently swallowed"
    );
    assert!(
        calls.load(Ordering::SeqCst) >= 1,
        "the conversion must have been attempted"
    );
}
