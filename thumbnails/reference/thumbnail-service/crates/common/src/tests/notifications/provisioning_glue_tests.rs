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

//! Glue tests for the stateful I/O in [`ensure_durable_queue`] — the thin async
//! wrapper that threads the pure decision core (unit-tested in
//! `provisioning_tests.rs`) through the real tonic client, interceptor,
//! per-request timeout, and reactive token refresh.
//!
//! These use a live in-process gRPC server ([`super::mock_consumer_server`]) to
//! cover the two "logic tested, glue untested" seams a manager review called
//! out:
//! - `Unauthenticated` -> refresh-token-once -> retry succeeds (the auth-refresh
//!   path, where a silent regression would be most costly).
//! - a server that accepts the request but never responds collapses into the
//!   per-request timeout (the timeout-wrapping that keeps a hung server from
//!   blocking startup forever).

use std::sync::Arc;

use serial_test::serial;
use thumbnail_auth::TokenRefresher;
use tokio::sync::watch;
use tonic::Code;

use crate::config::StreamRuntimeArgs;
use crate::errors::NotificationError;
use crate::notifications::provisioning::{
    build_storage_filter_groups, ensure_durable_queue, EnsureQueueOutcome,
};
use crate::test_fixtures::mock_consumer_server::{CreateBehavior, MockConsumerServer};
use crate::OutboundClients;

const QUEUE_ID: &str = "test-queue-id";
const RESOURCE_PREFIX: &str = "omniverse://storage/projects/my-project/";

/// A [`TokenRefresher`] that counts how many times `refresh()` was invoked, so a
/// test can assert the reactive refresh-once contract fired exactly once.
struct CountingRefresher {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[tonic::async_trait]
impl TokenRefresher for CountingRefresher {
    async fn refresh(&self) {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Build a `StreamRuntimeArgs` from defaults, then override only the knobs these
/// tests depend on. Parsing from an empty argv gives every field its documented
/// default without reaching into private constructors.
fn stream_args(stream_timeout_secs: u64, provisioning_max_retries: u32) -> StreamRuntimeArgs {
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        stream: StreamRuntimeArgs,
    }

    // `try_parse_from` reads (and validates) `#[arg(env = ...)]` values from the
    // process-global env at parse time — before the two knobs below are
    // overridden. If another test has transiently set an invalid stream env var
    // (e.g. `test_stream_runtime_invalid_values_are_rejected` sets
    // `EVENT_TASK_SUBMISSION_MAX_RETRY_AGE=0s`), this parse would reject it and
    // panic. The callers are therefore `#[serial]`, so they never overlap the
    // env-mutating `#[serial]` config tests and always parse a clean env.
    let mut args = Wrapper::try_parse_from(["test"])
        .expect("StreamRuntimeArgs must parse from defaults")
        .stream;
    args.stream_timeout_secs = stream_timeout_secs;
    args.notifications_provisioning_max_retries = provisioning_max_retries;
    args
}

/// `Unauthenticated` on the first `CreateDurableQueue` must trigger exactly one
/// token refresh and a single retry of the identical request, and the retried
/// call's success must be surfaced — the reactive refresh-once glue in
/// `ensure_durable_queue`.
#[tokio::test]
#[serial]
async fn unauthenticated_refreshes_token_once_then_retry_succeeds() {
    let server = MockConsumerServer::start(vec![
        CreateBehavior::Fail(Code::Unauthenticated),
        CreateBehavior::Created(true),
    ])
    .await;
    let channel = server.channel().await;

    let interceptor = OutboundClients::for_tests().interceptor;
    let refresh_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let refresher = Arc::new(CountingRefresher {
        calls: refresh_calls.clone(),
    });

    // Generous per-request timeout (no timeout path here); zero transient
    // retries so only the auth-refresh retry can drive a second attempt.
    let backoff = stream_args(30, 0);
    let (_tx, mut shutdown) = watch::channel(false);

    let outcome = ensure_durable_queue(
        channel,
        interceptor,
        refresher,
        QUEUE_ID,
        build_storage_filter_groups(&[RESOURCE_PREFIX.to_string()]),
        &mut shutdown,
        &backoff,
    )
    .await
    .expect("refresh-once retry must yield a successful provisioning outcome");

    assert_eq!(
        outcome,
        EnsureQueueOutcome::Created,
        "the post-refresh retry responded created=true"
    );
    assert_eq!(
        refresh_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "token must be refreshed exactly once on Unauthenticated"
    );
    assert_eq!(
        server.create_call_count(),
        2,
        "the identical request must be reissued exactly once after refresh"
    );

    server.shutdown().await;
}

/// A persistent `Unauthenticated` (even after the one refresh) must not loop
/// forever: it surfaces as `QueueProvisioningFailed` carrying the
/// `Unauthenticated` code, and the request is attempted exactly twice (initial +
/// one post-refresh retry).
#[tokio::test]
#[serial]
async fn unauthenticated_that_persists_after_refresh_fails_without_looping() {
    let server = MockConsumerServer::start(vec![CreateBehavior::Fail(Code::Unauthenticated)]).await;
    let channel = server.channel().await;

    let interceptor = OutboundClients::for_tests().interceptor;
    let refresh_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let refresher = Arc::new(CountingRefresher {
        calls: refresh_calls.clone(),
    });

    let backoff = stream_args(30, 0);
    let (_tx, mut shutdown) = watch::channel(false);

    let err = ensure_durable_queue(
        channel,
        interceptor,
        refresher,
        QUEUE_ID,
        build_storage_filter_groups(&[RESOURCE_PREFIX.to_string()]),
        &mut shutdown,
        &backoff,
    )
    .await
    .expect_err("a persistent Unauthenticated must fail, not succeed");

    match err {
        NotificationError::QueueProvisioningFailed { status, queue_id } => {
            assert_eq!(status.code(), Code::Unauthenticated);
            assert_eq!(queue_id, QUEUE_ID);
        }
        other => panic!("expected QueueProvisioningFailed, got {other:?}"),
    }
    assert_eq!(
        refresh_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "refresh must fire once and only once, never in a loop"
    );
    assert_eq!(
        server.create_call_count(),
        2,
        "initial attempt + exactly one post-refresh retry"
    );

    server.shutdown().await;
}

/// A server that accepts the request but never responds must be bounded by the
/// per-request deadline (`stream_timeout_secs`): the attempt collapses into a
/// timeout that the decision routes onto the transient-retry path, and with a
/// zero retry budget it fails fast as `DeadlineExceeded` rather than hanging.
#[tokio::test]
#[serial]
async fn unresponsive_server_is_bounded_by_the_per_request_timeout() {
    let server = MockConsumerServer::start(vec![CreateBehavior::Hang]).await;
    let channel = server.channel().await;

    let interceptor = OutboundClients::for_tests().interceptor;
    let refresher: Arc<dyn TokenRefresher> = Arc::new(CountingRefresher {
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });

    // 1s per-request deadline, no transient retries: one attempt times out and
    // the exhausted budget turns it into a fail-fast.
    let backoff = stream_args(1, 0);
    let (_tx, mut shutdown) = watch::channel(false);

    let started = std::time::Instant::now();
    let err = ensure_durable_queue(
        channel,
        interceptor,
        refresher,
        QUEUE_ID,
        build_storage_filter_groups(&[RESOURCE_PREFIX.to_string()]),
        &mut shutdown,
        &backoff,
    )
    .await
    .expect_err("an unresponsive server must not hang the caller forever");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the call must be bounded by the per-request timeout, not block indefinitely"
    );
    match err {
        NotificationError::QueueProvisioningFailed { status, queue_id } => {
            assert_eq!(
                status.code(),
                Code::DeadlineExceeded,
                "a timed-out attempt must surface as DeadlineExceeded"
            );
            assert_eq!(queue_id, QUEUE_ID);
        }
        other => panic!("expected QueueProvisioningFailed, got {other:?}"),
    }

    server.shutdown().await;
}
