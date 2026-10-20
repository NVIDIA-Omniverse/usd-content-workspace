// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// Epoch seconds of the last successful sync cycle completion.
/// Read by the `directory.snapshot.age_seconds` observable gauge callback.
static LAST_SYNC_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn record_sync_completed() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    LAST_SYNC_EPOCH.store(now, Ordering::Release);
}

fn snapshot_age_seconds() -> f64 {
    let last = LAST_SYNC_EPOCH.load(Ordering::Acquire);
    if last == 0 {
        return 0.0;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (now.saturating_sub(last)) as f64
}

/// All OpenTelemetry metric instruments used by the service.
pub static METRICS: LazyLock<Metrics> = LazyLock::new(|| {
    let meter = opentelemetry::global::meter("microsoft-graph-userinfo-service");
    Metrics::new(&meter)
});

pub struct Metrics {
    // -- HTTP request metrics (RED) --
    pub http_request_duration: Histogram<f64>,
    pub http_request_count: Counter<u64>,
    pub http_request_error_count: Counter<u64>,

    // -- gRPC request metrics (RED) --
    pub rpc_duration: Histogram<f64>,
    pub rpc_request_count: Counter<u64>,
    pub rpc_request_error_count: Counter<u64>,

    // -- Graph client metrics --
    pub graph_request_duration: Histogram<f64>,
    pub graph_request_count: Counter<u64>,
    pub graph_retry_count: Counter<u64>,

    // -- Permission client metrics --
    pub permission_request_duration: Histogram<f64>,
    pub permission_request_count: Counter<u64>,

    // -- Sync metrics --
    pub sync_duration: Histogram<f64>,
    pub sync_count: Counter<u64>,

    // -- Snapshot gauges --
    pub snapshot_users: Gauge<u64>,
    pub snapshot_groups: Gauge<u64>,
    pub snapshot_memberships: Gauge<u64>,

    // -- Permission cache --
    pub permission_cache_hits: Counter<u64>,
    pub permission_cache_misses: Counter<u64>,

    // -- Postgres persistence --
    pub postgres_load_duration: Histogram<f64>,
    pub postgres_commit_duration: Histogram<f64>,
}

impl Metrics {
    fn new(meter: &Meter) -> Self {
        meter
            .f64_observable_gauge("directory.snapshot.age_seconds")
            .with_description("Seconds since the last successful sync cycle completed")
            .with_unit("s")
            .with_callback(|gauge| {
                gauge.observe(snapshot_age_seconds(), &[]);
            })
            .build();

        Self {
            http_request_duration: meter
                .f64_histogram("http.server.request.duration")
                .with_description("Duration of REST request handling")
                .with_unit("s")
                .build(),
            http_request_count: meter
                .u64_counter("http.server.request.count")
                .with_description("Total REST requests received")
                .build(),
            http_request_error_count: meter
                .u64_counter("http.server.request.error.count")
                .with_description("REST requests that resulted in a 4xx or 5xx status")
                .build(),

            rpc_duration: meter
                .f64_histogram("rpc.server.duration")
                .with_description("Duration of gRPC request handling")
                .with_unit("s")
                .build(),
            rpc_request_count: meter
                .u64_counter("rpc.server.request.count")
                .with_description("Total gRPC requests received")
                .build(),
            rpc_request_error_count: meter
                .u64_counter("rpc.server.request.error.count")
                .with_description("gRPC requests that resulted in a non-OK status code")
                .build(),

            graph_request_duration: meter
                .f64_histogram("graph.client.request.duration")
                .with_description("Duration of outgoing Microsoft Graph API calls")
                .with_unit("s")
                .build(),
            graph_request_count: meter
                .u64_counter("graph.client.request.count")
                .with_description("Total Graph API calls, including retries")
                .build(),
            graph_retry_count: meter
                .u64_counter("graph.client.retry.count")
                .with_description("Graph call retries, labeled with reason")
                .build(),

            permission_request_duration: meter
                .f64_histogram("permission.client.request.duration")
                .with_description("Duration of outgoing Permission Service calls")
                .with_unit("s")
                .build(),
            permission_request_count: meter
                .u64_counter("permission.client.request.count")
                .with_description("Total Permission Service calls")
                .build(),

            sync_duration: meter
                .f64_histogram("directory.sync.duration")
                .with_description("Duration of each sync cycle")
                .with_unit("s")
                .build(),
            sync_count: meter
                .u64_counter("directory.sync.count")
                .with_description("Total sync cycles completed")
                .build(),

            snapshot_users: meter
                .u64_gauge("directory.snapshot.users")
                .with_description("Number of users in the current in-memory snapshot")
                .build(),
            snapshot_groups: meter
                .u64_gauge("directory.snapshot.groups")
                .with_description("Number of groups in the current in-memory snapshot")
                .build(),
            snapshot_memberships: meter
                .u64_gauge("directory.snapshot.memberships")
                .with_description("Number of membership edges in the current snapshot")
                .build(),

            permission_cache_hits: meter
                .u64_counter("permission.cache.hit_count")
                .with_description("Permission decision cache hits")
                .build(),
            permission_cache_misses: meter
                .u64_counter("permission.cache.miss_count")
                .with_description("Permission decision cache misses")
                .build(),

            postgres_load_duration: meter
                .f64_histogram("snapshot.postgres.load.duration")
                .with_description("Duration of snapshot load from PostgreSQL at startup")
                .with_unit("s")
                .build(),
            postgres_commit_duration: meter
                .f64_histogram("snapshot.postgres.commit.duration")
                .with_description("Duration of snapshot commit after a sync cycle")
                .with_unit("s")
                .build(),
        }
    }
}

/// Helper: record HTTP request metrics from the trace middleware.
pub fn record_http_request(method: &str, route: &str, status: u16, duration_secs: f64) {
    let m = &*METRICS;
    let attrs = [
        KeyValue::new("method", method.to_owned()),
        KeyValue::new("route", route.to_owned()),
        KeyValue::new("status", i64::from(status)),
    ];
    m.http_request_duration.record(duration_secs, &attrs);
    m.http_request_count.add(1, &attrs);
    if status >= 400 {
        m.http_request_error_count.add(1, &attrs);
    }
}

/// Helper: record gRPC request metrics.
pub fn record_rpc_request(service: &str, method: &str, status_code: i32, duration_secs: f64) {
    let m = &*METRICS;
    let attrs = [
        KeyValue::new("rpc.service", service.to_owned()),
        KeyValue::new("rpc.method", method.to_owned()),
        KeyValue::new("rpc.grpc.status_code", i64::from(status_code)),
    ];
    m.rpc_duration.record(duration_secs, &attrs);
    m.rpc_request_count.add(1, &attrs);
    if status_code != 0 {
        m.rpc_request_error_count.add(1, &attrs);
    }
}

/// Helper: record a Graph client request.
pub fn record_graph_request(operation: &str, status: &str, duration_secs: f64) {
    let m = &*METRICS;
    let attrs = [
        KeyValue::new("operation", operation.to_owned()),
        KeyValue::new("status", status.to_owned()),
    ];
    m.graph_request_duration.record(duration_secs, &attrs);
    m.graph_request_count.add(1, &attrs);
}

/// Helper: record a Graph client retry.
pub fn record_graph_retry(operation: &str, reason: &str) {
    METRICS.graph_retry_count.add(
        1,
        &[
            KeyValue::new("operation", operation.to_owned()),
            KeyValue::new("reason", reason.to_owned()),
        ],
    );
}

/// Helper: record a Permission Service request.
pub fn record_permission_request(status: &str, duration_secs: f64) {
    let m = &*METRICS;
    let attrs = [KeyValue::new("status", status.to_owned())];
    m.permission_request_duration.record(duration_secs, &attrs);
    m.permission_request_count.add(1, &attrs);
}

/// Helper: record a sync cycle outcome.
pub fn record_sync_cycle(outcome: &str, duration_secs: f64) {
    let m = &*METRICS;
    let attrs = [KeyValue::new("outcome", outcome.to_owned())];
    m.sync_duration.record(duration_secs, &attrs);
    m.sync_count.add(1, &attrs);
}

/// Helper: update snapshot gauge values.
pub fn record_snapshot_size(users: u64, groups: u64, memberships: u64) {
    let m = &*METRICS;
    m.snapshot_users.record(users, &[]);
    m.snapshot_groups.record(groups, &[]);
    m.snapshot_memberships.record(memberships, &[]);
}
