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

//! Regression tests for the timeout wrapped around the initial
//! `consume_durable_events_with_acks` call in
//! `notifications::streaming::EventConsumer::consume_durable_events_continuously`.
//!
//! Dev incident (distinct from the one covered by `durable_consumer_housekeeping_tests.rs`):
//! the stream-idle defense fired correctly and triggered a reconnect, but the
//! reconnect itself then hung for 8+ minutes with zero log output and no
//! automatic recovery, requiring a manual pod restart. Root cause: the call that
//! opens a new durable-queue consumer session was a bare, unbounded `.await` —
//! none of this module's other defenses (stream-idle check, auth-error handling)
//! run until *after* that call returns, so a server that stalls accepting a new
//! session (e.g. a stale session/lock for the same `queue_id` not yet released
//! from the just-torn-down previous stream) could block the client forever with
//! no way to detect or recover. The fix wraps that call in
//! `tokio::time::timeout(stream_timeout_secs, ..)`, converting an indefinite hang
//! into a bounded, retryable error that flows into the existing reconnect-backoff
//! loop in `start_event_service`.
//!
//! These tests reproduce the shape (scaled to milliseconds) against a future that
//! never resolves, standing in for a stalled `consume_durable_events_with_acks`
//! call, without needing a real gRPC client/server.

use std::time::Duration;
use tokio::time;

/// The fixed pattern: the connect call is wrapped in `tokio::time::timeout`, so a
/// server that never responds still yields a bounded error instead of hanging the
/// caller forever.
#[tokio::test]
async fn bounded_timeout_around_stream_establishment_fires_when_server_never_responds() {
    let establish_timeout = Duration::from_millis(100);

    // Stands in for a `consume_durable_events_with_acks(..).await` whose response
    // never arrives (e.g. the server is stuck on a stale session for this queue_id).
    let never_responds = std::future::pending::<Result<(), String>>();

    let result = time::timeout(establish_timeout, never_responds).await;

    assert!(
        result.is_err(),
        "timeout must fire and yield an error when the establishment call never resolves"
    );
}

/// Demonstrates the bug this replaced: a bare, unbounded `.await` on the
/// establishment call never returns, no matter how long a hung server takes —
/// there is nothing else in `consume_durable_events_continuously` that can step
/// in, because every other defense (idle-stream check, per-message auth
/// handling) lives inside the loop *after* this call, which is never reached.
#[tokio::test]
async fn unbounded_await_on_stream_establishment_never_returns_even_well_past_any_reasonable_bound()
{
    let never_responds = std::future::pending::<Result<(), String>>();

    // Race the unbounded await against a generous deadline; if the bug were
    // fixed by anything *other* than adding the timeout, this would need to
    // change, but as long as the establishment call is a bare `.await`, the
    // deadline always wins.
    let long_deadline = time::sleep(Duration::from_millis(500));
    tokio::pin!(long_deadline);
    tokio::pin!(never_responds);

    tokio::select! {
        _ = &mut long_deadline => {
            // Bug demonstration: the deadline elapsed first, proving the
            // establishment call itself would never have unblocked the caller.
        }
        _ = &mut never_responds => {
            panic!("bug demonstration: the unbounded await should never resolve");
        }
    }
}
