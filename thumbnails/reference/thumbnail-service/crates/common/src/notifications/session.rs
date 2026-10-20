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

//! The mechanics both durable consume loops need, in one place.
//!
//! There are two loops in this workspace — StorageEventQueue here in `streaming.rs` for
//! `omni.storage.*`, and WorkerEventQueue in `task-worker` for `omni.thumbnail.generate`
//! — and they are not being merged into one. Their acknowledgement policies
//! genuinely differ: StorageEventQueue converts and routes by storage event type, WorkerEventQueue
//! tracks withheld acknowledgements and recycles its stream to force redelivery.
//! Folding those into a single loop behind enough hooks to express both would be
//! harder to read than the two loops are.
//!
//! What is shared is everything underneath the policy: how a stream is opened,
//! how an acknowledgement is sent, and how long to wait before reconnecting.
//! Those were written twice, and the copies had already drifted — StorageEventQueue still
//! had an acknowledgement-ordering bug WorkerEventQueue had fixed. Two loops is a design
//! choice; duplicating the open/ack/backoff helpers was an accident.

use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::{Stream, StreamExt};

use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::{
    ConsumeDurableEventsWithAcksRequest, ConsumeDurableEventsWithAcksResponse,
};
use thumbnail_auth::AuthInterceptor;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::Status;
use tracing::warn;

/// Why a negative acknowledgement could not be delivered.
///
/// Callers use this to decide whether to fall back to the recycle (the stream
/// is broken and a NACK on it would be swallowed) or to surface the error to
/// an observer that can distinguish a rejected operation from a dead stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NackError {
    /// The ack channel is closed; the stream is already gone and the NACK
    /// cannot be sent. Callers SHOULD fall back to the recycle.
    ChannelClosed,
}

/// The consumer client both loops build: one channel, one auth interceptor.
pub type AckStreamClient = EventConsumerServiceClient<InterceptedService<Channel, AuthInterceptor>>;

/// The response half of a session.
///
/// Boxed rather than `Streaming<_>` because the call that produces the
/// `Streaming` is resolved lazily — see `open_ack_stream`. Both loops poll this
/// with `next()`, which `Streaming` already provided, so the substitution is
/// invisible to them.
pub type Deliveries =
    Pin<Box<dyn Stream<Item = Result<ConsumeDurableEventsWithAcksResponse, Status>> + Send>>;

/// An open durable-consume session.
///
/// The request half carries acknowledgements back; the response half carries
/// deliveries. Dropping `acks` ends the request stream, so hold it until every
/// in-flight handler has had its chance to acknowledge.
pub struct AckStream {
    pub acks: mpsc::Sender<ConsumeDurableEventsWithAcksRequest>,
    pub deliveries: Deliveries,
}

/// Why a session could not be opened.
///
/// Only one thing can now fail before the stream exists. Everything the server
/// can say about the call — an auth rejection, a refusal, a transport failure —
/// arrives as the first item of `deliveries` instead, because the call is no
/// longer awaited here. Both loops already classify those on the stream, which
/// is what makes moving them there safe.
///
/// There was a `TimedOut` variant, bounding how long the open could block. The
/// bound was real: an unbounded await hung a consumer for eight minutes in dev
/// with no log output. It is gone because the await is gone — this function no
/// longer blocks at all, so there is nothing left to bound.
#[derive(Debug)]
pub enum OpenError {
    /// The acknowledgement channel closed before the stream was established.
    ChannelClosed,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelClosed => {
                write!(f, "ack channel closed before the stream opened")
            }
        }
    }
}

/// Open a durable consume session for `queue_id`.
///
/// The first request on the channel names the queue and carries no
/// acknowledgements; every later one carries acknowledgements and no queue.
pub async fn open_ack_stream(
    client: &mut AckStreamClient,
    queue_id: &str,
    ack_buffer: usize,
) -> Result<AckStream, OpenError> {
    let (acks, ack_rx) = mpsc::channel::<ConsumeDurableEventsWithAcksRequest>(ack_buffer);

    if acks
        .send(ConsumeDurableEventsWithAcksRequest {
            queue_id: queue_id.to_string(),
            ack_delivery_tags: vec![],
            nack_delivery_tags: vec![],
        })
        .await
        .is_err()
    {
        return Err(OpenError::ChannelClosed);
    }

    // Deliberately not awaited here. `consume_durable_events_with_acks` resolves
    // when the server sends response headers, and the platform defers those
    // until it has a delivery to write — so on a queue with no traffic the call
    // never returns. Awaiting it made "the stream is established" mean "work has
    // already arrived", which is unobservable on an idle queue: the consumer sat
    // in the await until a bound elapsed, tore the stream down, and reconnected,
    // forever. StorageEventQueue hid this because storage events are constant; WorkerEventQueue is
    // idle at startup, so `task-worker` never reported ready and its first
    // rollout failed on the readiness gate.
    //
    // The request itself is already on the wire by the time this returns, and
    // the server has accepted it, so the session is genuinely open. Resolving
    // the call on first poll keeps every failure it can report — including the
    // auth rejection both loops classify — arriving as the stream's first item
    // instead of vanishing.
    // Cloned so the deferred call owns its client: the stream outlives this
    // function, and tonic clients are handles over a shared channel.
    let mut client = client.clone();
    let call = async move {
        client
            .consume_durable_events_with_acks(ReceiverStream::new(ack_rx))
            .await
    };
    let deliveries = futures_util::stream::once(call)
        .flat_map(|opened| match opened {
            Ok(response) => response.into_inner().boxed(),
            Err(status) => futures_util::stream::once(async move { Err(status) }).boxed(),
        })
        .boxed();

    Ok(AckStream { acks, deliveries })
}

/// Acknowledge one delivery, reporting whether the acknowledgement was sent.
///
/// A `false` means the request stream is already gone, so the delivery stays
/// unacknowledged and will be redelivered — worth a warning, not a failure.
pub async fn send_ack(
    acks: &mpsc::Sender<ConsumeDurableEventsWithAcksRequest>,
    delivery_tag: i64,
) -> bool {
    let sent = acks
        .send(ConsumeDurableEventsWithAcksRequest {
            // Empty: the queue was named when the session opened.
            queue_id: String::new(),
            ack_delivery_tags: vec![delivery_tag],
            nack_delivery_tags: vec![],
        })
        .await
        .is_ok();

    if !sent {
        warn!(
            delivery_tag,
            "Ack channel closed before the acknowledgement was sent; the delivery will come back"
        );
    }
    sent
}

/// Negatively acknowledge one delivery, requesting it be requeued for redelivery.
///
/// Returns `Ok(())` if the request was sent, or `Err(NackError::ChannelClosed)` if
/// the request stream is already gone — which means the session is broken and
/// the caller should fall back to the recycle to force redelivery.
///
/// Unlike [`send_ack`], which is called on success paths and whose failure is
/// merely a warning, a failed NACK must be surfaced as a distinct error so the
/// caller can choose the fallback recycle rather than silently leaving the
/// delivery unacknowledged.
pub async fn send_nack(
    acks: &mpsc::Sender<ConsumeDurableEventsWithAcksRequest>,
    delivery_tag: i64,
) -> Result<(), NackError> {
    let sent = acks
        .send(ConsumeDurableEventsWithAcksRequest {
            // Empty: the queue was named when the session opened.
            queue_id: String::new(),
            ack_delivery_tags: vec![],
            nack_delivery_tags: vec![delivery_tag],
        })
        .await
        .is_ok();

    if !sent {
        warn!(
            delivery_tag,
            "Nack channel closed before the negative acknowledgement was sent; \
             falling back to the recycle to force redelivery"
        );
        return Err(NackError::ChannelClosed);
    }
    Ok(())
}

/// How long to wait before the next reconnect attempt.
///
/// `attempt` is 1-based: the first retry waits `base_seconds`, and each one after
/// that doubles. Both callers increment their counter before asking, so treating
/// the argument as an exponent directly made the *first* retry wait twice the
/// configured base — a setting named "base backoff" that was never used.
///
/// The exponent is capped before the shift so a long outage cannot overflow it,
/// and the result is capped at `max_seconds`.
pub fn reconnect_backoff(
    attempt: u32,
    base_seconds: u64,
    exponent_cap: u32,
    max_seconds: u64,
) -> Duration {
    let exponent = attempt.saturating_sub(1).min(exponent_cap);
    let factor = 2u64.saturating_pow(exponent);
    let ceiling = Duration::from_secs(base_seconds.saturating_mul(factor).min(max_seconds));
    jittered(ceiling)
}

/// Spread a computed backoff over the second half of its interval.
///
/// Without this the delay is a pure function of the attempt number, so every
/// replica that lost its stream to the same event retries at the same instants:
/// the dependency that just came back gets its reconnection load in synchronized
/// bursts rather than spread out, which is worst exactly when it is least able to
/// absorb it. The worker autoscales to twenty replicas, so this is not
/// hypothetical at load.
///
/// Equal jitter — `[ceiling/2, ceiling]` — rather than full jitter over
/// `[0, ceiling]`. Full jitter spreads over a wider window but can return zero,
/// turning a retry into an immediate one; keeping half the interval as a floor
/// buys desynchronization without ever making the backoff disappear. Half of a
/// 60s ceiling is a 30s spread, which is ample for a replica count in the tens.
///
/// The ceiling is preserved: the draw is bounded above by the computed value.
pub(crate) fn jittered(ceiling: Duration) -> Duration {
    let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
    if millis == 0 {
        return ceiling;
    }
    let floor = millis / 2;
    Duration::from_millis(rand::random_range(floor..=millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The growth shape is asserted as a band, not an equality, because the delay
    /// is jittered: each attempt draws from the upper half of its computed
    /// interval. The ceiling per attempt is unchanged -- 5, 10, 20, 40, 60, 60 --
    /// so the assertion is that every draw lands inside `[ceiling/2, ceiling]`.
    ///
    /// The original point of this test survives: the *first* retry's ceiling is
    /// the configured base rather than double it. Both callers increment before
    /// asking, so an exponent taken straight from the attempt count skipped the
    /// base delay entirely.
    #[test]
    fn the_backoff_grows_from_the_configured_base_within_its_jitter_band() {
        // base 5s, exponent capped at 4, ceiling 60s.
        let ceilings = [(1, 5), (2, 10), (3, 20), (4, 40), (5, 60), (6, 60)];

        for (attempt, ceiling) in ceilings {
            // Repeated because a single draw could land anywhere in the band and
            // would not distinguish a correct implementation from a stuck one.
            for _ in 0..64 {
                let waited = reconnect_backoff(attempt, 5, 4, 60);
                assert!(
                    waited <= Duration::from_secs(ceiling),
                    "attempt {attempt} waited {waited:?}, over its {ceiling}s ceiling"
                );
                assert!(
                    waited >= Duration::from_secs(ceiling) / 2,
                    "attempt {attempt} waited {waited:?}, under half its {ceiling}s ceiling — \
                     jitter must not turn a retry into an immediate one"
                );
            }
        }
    }

    /// Jitter has to actually vary, or it is a no-op that still passes the band
    /// assertions above.
    #[test]
    fn the_backoff_is_not_the_same_value_every_time() {
        let draws: std::collections::HashSet<u128> = (0..256)
            .map(|_| reconnect_backoff(5, 5, 4, 60).as_millis())
            .collect();

        assert!(
            draws.len() > 1,
            "every draw returned the same delay, so replicas would still retry in \
             lock-step: {draws:?}"
        );
    }

    /// A long outage must not shift its way into an overflow panic or wrap
    /// around to a tiny backoff.
    #[test]
    fn a_very_long_outage_stays_at_the_ceiling() {
        for _ in 0..64 {
            let capped = reconnect_backoff(u32::MAX, 5, 4, 60);
            assert!(capped <= Duration::from_secs(60));
            assert!(capped >= Duration::from_secs(30));

            let saturated = reconnect_backoff(u32::MAX, u64::MAX, 16, 60);
            assert!(saturated <= Duration::from_secs(60));
            assert!(saturated >= Duration::from_secs(30));
        }
    }

    /// A zero ceiling stays zero rather than panicking on an empty draw range.
    #[test]
    fn a_zero_ceiling_is_left_alone() {
        assert_eq!(reconnect_backoff(1, 0, 4, 60), Duration::ZERO);
        assert_eq!(jittered(Duration::ZERO), Duration::ZERO);
    }

    /// Task 4.3: a rejected NACK (channel closed before the send) surfaces as
    /// `NackError::ChannelClosed` rather than being silently swallowed.
    #[tokio::test]
    async fn send_nack_returns_channel_closed_when_receiver_is_dropped() {
        let (tx, rx) = tokio::sync::mpsc::channel::<ConsumeDurableEventsWithAcksRequest>(1);
        // Drop the receiver so the channel is immediately closed.
        drop(rx);

        let result = send_nack(&tx, 42).await;
        assert_eq!(
            result,
            Err(NackError::ChannelClosed),
            "a send_nack on a closed channel must surface NackError::ChannelClosed"
        );
    }

    /// send_nack on an open channel succeeds and encodes the delivery tag in
    /// `nack_delivery_tags`, not `ack_delivery_tags`.
    #[tokio::test]
    async fn send_nack_encodes_tag_in_nack_delivery_tags() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ConsumeDurableEventsWithAcksRequest>(1);

        let result = send_nack(&tx, 99).await;
        assert_eq!(result, Ok(()), "send on an open channel must succeed");

        let msg = rx.try_recv().expect("NACK request must be buffered");
        assert!(
            msg.ack_delivery_tags.is_empty(),
            "a NACK must not also ack; got ack_delivery_tags={:?}",
            msg.ack_delivery_tags
        );
        assert_eq!(
            msg.nack_delivery_tags,
            vec![99i64],
            "the delivery tag must appear in nack_delivery_tags"
        );
    }
}
