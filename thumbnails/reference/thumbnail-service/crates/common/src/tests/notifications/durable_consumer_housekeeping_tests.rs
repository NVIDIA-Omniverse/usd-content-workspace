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

//! Regression tests for the durable-consumer `select!` housekeeping pattern.
//!
//! Historically idle-silence reconnect was implemented as
//! `tokio::time::timeout(stream_timeout_secs, stream.next())` inside a
//! `tokio::select!` that also raced a shorter recreated sleep. `select!`
//! recreates every branch each iteration, so the timeout never elapsed. The
//! durable loops now use a persistent `tokio::time::interval` for token-expiry
//! (and WorkerEventQueue recycle) checks — the same shape that fixed the heartbeat and
//! idle bugs. Idle-silence rebuild itself is gone (peer-aligned reconnect).
//!
//! These tests keep the select! shapes (scaled to milliseconds) without a real
//! gRPC client/server. Quiet-stream behaviour end-to-end lives in
//! `crates/task-worker/tests/consumer.rs`.

use futures_util::stream::{self, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thumbnail_auth::{Token, TokenManager};
use tokio::time::{self, MissedTickBehavior};

use crate::auth::TokenGate;

/// Token-expiry reopen must fire from a persistent interval even when the stream
/// never yields — silence alone is not a reconnect trigger, but expiry-soon is.
#[tokio::test]
async fn token_expiry_check_fires_via_persistent_interval_when_stream_never_yields() {
    let periodic_tick = Duration::from_millis(50);
    let expiring = Arc::new(AtomicBool::new(false));

    let mut dead_stream = stream::pending::<()>();

    let mut periodic_timer = time::interval(periodic_tick);
    periodic_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    periodic_timer.tick().await;

    let test_start = time::Instant::now();
    let mut detected_after: Option<Duration> = None;

    let overall_deadline = time::sleep(Duration::from_secs(5));
    tokio::pin!(overall_deadline);

    let flag = expiring.clone();
    tokio::spawn(async move {
        time::sleep(Duration::from_millis(200)).await;
        flag.store(true, Ordering::SeqCst);
    });

    loop {
        tokio::select! {
            _ = &mut overall_deadline => {
                break;
            }
            _ = dead_stream.next() => {
                unreachable!("dead_stream never yields anything");
            }
            _ = periodic_timer.tick() => {
                if expiring.load(Ordering::SeqCst) {
                    detected_after = Some(test_start.elapsed());
                    break;
                }
            }
        }
    }

    let detected_after =
        detected_after.expect("token-expiry reopen must fire once is_expiring_soon becomes true");

    assert!(
        detected_after >= Duration::from_millis(200),
        "fired before the expiry flag was set: {detected_after:?}"
    );
    assert!(
        detected_after < Duration::from_millis(200) + periodic_tick * 4,
        "took implausibly long to notice expiry: {detected_after:?}"
    );
}

/// A quiet stream with a fresh token must not reconnect solely for silence.
/// Driven from a real [`TokenGate`] so a hardcoded false cannot pass vacuously.
#[tokio::test]
async fn quiet_stream_with_fresh_token_does_not_reconnect_for_silence() {
    let periodic_tick = Duration::from_millis(50);
    let observe_for = Duration::from_millis(400);

    let manager = Arc::new(TokenManager::new(
        Arc::new(reqwest::Client::new()),
        "id".into(),
        "secret".into(),
        "scope".into(),
        "http://127.0.0.1/token".into(),
        Default::default(),
        Default::default(),
    ));
    {
        let handle = manager.get_token_handle();
        *handle.lock().unwrap() = Some(Token::new("tok".into(), 3600));
    }
    let gate = TokenGate::client_credentials(manager);

    let mut dead_stream = stream::pending::<()>();
    let mut periodic_timer = time::interval(periodic_tick);
    periodic_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    periodic_timer.tick().await;

    let mut reconnects = 0u32;
    let overall_deadline = time::sleep(observe_for);
    tokio::pin!(overall_deadline);

    loop {
        tokio::select! {
            _ = &mut overall_deadline => {
                break;
            }
            _ = dead_stream.next() => {
                unreachable!("dead_stream never yields anything");
            }
            _ = periodic_timer.tick() => {
                if gate.is_expiring_soon() {
                    reconnects += 1;
                    break;
                }
            }
        }
    }

    assert_eq!(
        reconnects, 0,
        "silence alone must not force a reconnect when the token is fresh"
    );
}

/// Demonstrates the bug the persistent interval replaced: wrapping
/// `stream.next()` in a `tokio::time::timeout` recreated every `select!`
/// iteration never times out when raced against a shorter recreated sleep.
#[tokio::test]
async fn naive_recreated_timeout_wrapper_never_fires_even_past_threshold() {
    let stream_read_timeout = Duration::from_millis(150);
    let recreated_periodic_sleep = Duration::from_millis(50);

    let mut dead_stream = stream::pending::<()>();
    let mut stream_timeout_fired_count = 0u32;

    let test_duration = Duration::from_secs(1);
    let overall_deadline = time::sleep(test_duration);
    tokio::pin!(overall_deadline);

    loop {
        tokio::select! {
            _ = &mut overall_deadline => {
                break;
            }
            stream_result = time::timeout(stream_read_timeout, dead_stream.next()) => {
                if stream_result.is_err() {
                    stream_timeout_fired_count += 1;
                }
            }
            _ = time::sleep(recreated_periodic_sleep) => {}
        }
    }

    assert_eq!(
        stream_timeout_fired_count, 0,
        "bug demonstration: the recreated timeout wrapper should never fire because \
         the shorter recreated sleep always wins the race first, but it fired {} times",
        stream_timeout_fired_count
    );
}
