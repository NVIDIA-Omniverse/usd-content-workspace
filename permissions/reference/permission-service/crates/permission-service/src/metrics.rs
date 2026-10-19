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

use crate::cache::Caches;
use crate::config::ServiceConfig;
use clap::Args;
use opentelemetry::metrics::{
    Counter, Gauge, Histogram, Meter, MeterProvider, ObservableGauge, UpDownCounter,
};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

#[derive(Args, Clone, Debug)]
pub struct MetricsArgs {
    #[arg(long, env = "METRICS_ENDPOINT", default_value = "")]
    pub metrics_endpoint: String,
}

pub fn build_meter(config: &ServiceConfig) -> (SdkMeterProvider, Meter) {
    if config.metrics.metrics_endpoint.is_empty() {
        let meter_provider = SdkMeterProvider::default();
        let meter = meter_provider.meter("noop");
        return (meter_provider, meter);
    }

    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_protocol(Protocol::Grpc)
        .with_endpoint(config.metrics.metrics_endpoint.clone())
        .build()
        .unwrap();
    let reader = PeriodicReader::builder(exporter).build();

    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(config.opentelemetry_resource())
        .build();
    opentelemetry::global::set_meter_provider(meter_provider.clone());

    let meter = opentelemetry::global::meter_with_scope(config.opentelemetry_scope());
    (meter_provider, meter)
}

#[derive(Debug, Clone)]
pub struct Metrics {
    meter_provider: SdkMeterProvider,
    pub meter: Meter,
    pub authorization_allowed: Counter<u64>,
    pub authorization_denied: Counter<u64>,
    pub request_count: Counter<u64>,
    pub request_errors: Counter<u64>,
    pub http_requests_in_flight: UpDownCounter<i64>,
    pub rpc_request_count: Counter<u64>,
    pub rpc_request_duration: Histogram<u64>,
    pub rpc_request_errors: Counter<u64>,
    pub rpc_requests_in_flight: UpDownCounter<i64>,
    pub db_operation_duration: Histogram<u64>,
    pub db_operation_errors: Counter<u64>,
    pub notification_publish_count: Counter<u64>,
    pub notification_publish_duration: Histogram<u64>,
    pub notification_consume_count: Counter<u64>,
    pub notification_consume_errors: Counter<u64>,
    pub notification_consume_reconnect_count: Counter<u64>,
    pub notification_consume_connected: Gauge<u64>,
    pub notification_consume_delivery_lag: Histogram<u64>,
    pub policy_evaluation_depth: Histogram<u64>,
    pub policy_evaluation_groups_scanned: Histogram<u64>,
    pub policy_fetch_size: Histogram<u64>,
    pub policy_mutation_count: Counter<u64>,
    pub policy_mutation_batch_size: Histogram<u64>,
    pub service_build_info: Gauge<u64>,
    evaluation_cache_hits: Counter<u64>,
    evaluation_cache_misses: Counter<u64>,
    metadata_cache_hits: Counter<u64>,
    metadata_cache_misses: Counter<u64>,
    policy_set_cache_hits: Counter<u64>,
    policy_set_cache_misses: Counter<u64>,
    _evaluation_cache_size: ObservableGauge<u64>,
    _metadata_cache_size: ObservableGauge<u64>,
    _policy_set_cache_size: ObservableGauge<u64>,
    _token_cache_size: ObservableGauge<u64>,
    _invalid_token_cache_size: ObservableGauge<u64>,
}

impl Metrics {
    pub fn new(meter_provider: SdkMeterProvider, meter: Meter, caches: &Caches) -> Metrics {
        let authorization_allowed = meter
            .u64_counter("authorization.request.allowed")
            .with_description("Total amount of allowed requests")
            .with_unit("requests")
            .build();
        let authorization_denied = meter
            .u64_counter("authorization.request.denied")
            .with_description(
                "Total amount of denied requests. `deny_reason` categorizes why: \
                 explicit_forbid | implicit | cross_principal_denied | evaluation_error",
            )
            .with_unit("requests")
            .build();
        let request_errors = meter
            .u64_counter("http.server.request.errors")
            .with_description("Total amount of requests that led to an error (HTTP4XX-HTTP5XX)")
            .with_unit("errors")
            .build();
        let request_count = meter
            .u64_counter("http.server.request.count")
            .with_description("Total amount of requests")
            .with_unit("requests")
            .build();
        let http_requests_in_flight = meter
            .i64_up_down_counter("http.server.request.active")
            .with_description("Number of HTTP requests currently being handled")
            .with_unit("requests")
            .build();
        let rpc_request_count = meter
            .u64_counter("rpc.server.request.count")
            .with_description("Total gRPC requests received")
            .with_unit("requests")
            .build();
        let rpc_request_duration = meter
            .u64_histogram("rpc.server.duration")
            .with_description("Duration of gRPC request handling")
            .with_unit("ms")
            .build();
        let rpc_request_errors = meter
            .u64_counter("rpc.server.request.error.count")
            .with_description("gRPC requests that resulted in a non-OK status code")
            .with_unit("requests")
            .build();
        let rpc_requests_in_flight = meter
            .i64_up_down_counter("rpc.server.request.active")
            .with_description("Number of gRPC requests currently being handled")
            .with_unit("requests")
            .build();
        let db_operation_duration = meter
            .u64_histogram("db.client.operation.duration")
            .with_description("Duration of a storage operation against the database")
            .with_unit("ms")
            .build();
        let db_operation_errors = meter
            .u64_counter("db.client.operation.errors")
            .with_description("Storage operations that failed with an internal database error")
            .with_unit("errors")
            .build();
        let notification_publish_count = meter
            .u64_counter("notification.publish.count")
            .with_description(
                "Outcome of each change-event publish attempt. `event_type` distinguishes \
                 policy-change from metadata-change events.",
            )
            .with_unit("events")
            .build();
        let notification_publish_duration = meter
            .u64_histogram("notification.publish.duration")
            .with_description(
                "Wall-clock time from the first publish attempt to the final outcome, \
                 including any retry",
            )
            .with_unit("ms")
            .build();
        let notification_consume_count = meter
            .u64_counter("notification.consume.count")
            .with_description(
                "Change events consumed from the Notification Service that triggered a local \
                 cache invalidation. `event_type` distinguishes policy-change from \
                 metadata-change events; `outcome` records how the caches were invalidated.",
            )
            .with_unit("events")
            .build();
        let notification_consume_errors = meter
            .u64_counter("notification.consume.errors")
            .with_description(
                "Consumer subscription stream terminations, by `reason` \
                 (unauthenticated | transport | stream_closed).",
            )
            .with_unit("errors")
            .build();
        let notification_consume_reconnect_count = meter
            .u64_counter("notification.consume.reconnect.count")
            .with_description(
                "Consumer subscription (re)connect attempts, by `reason` \
                 (initial | token_expiry | unauthenticated | error | stream_end).",
            )
            .with_unit("reconnects")
            .build();
        let notification_consume_connected = meter
            .u64_gauge("notification.consume.connected")
            .with_description(
                "1 while this replica has an active consumer subscription, 0 otherwise. \
                 A replica stuck at 0 silently goes stale until cache TTL.",
            )
            .build();
        let notification_consume_delivery_lag = meter
            .u64_histogram("notification.consume.delivery_lag")
            .with_description(
                "Delay between an event being published and this replica consuming it, \
                 measured as now - published_at.",
            )
            .with_unit("ms")
            .build();
        let policy_evaluation_depth = meter
            .u64_histogram("authorization.evaluation.policies_evaluated")
            .with_description(
                "Number of policies passed into the evaluator for a single action evaluation",
            )
            .with_unit("policies")
            .build();
        let policy_evaluation_groups_scanned = meter
            .u64_histogram("authorization.evaluation.order_groups_scanned")
            .with_description(
                "Number of distinct order groups iterated before a decision was reached",
            )
            .with_unit("groups")
            .build();
        let policy_fetch_size = meter
            .u64_histogram("authorization.policy_fetch.size")
            .with_description("Number of policies returned by a single storage fetch")
            .with_unit("policies")
            .build();
        let policy_mutation_count = meter
            .u64_counter("policy.mutation.count")
            .with_description("Every policy create/update/delete attempt, by outcome")
            .with_unit("policies")
            .build();
        let policy_mutation_batch_size = meter
            .u64_histogram("policy.mutation.batch_size")
            .with_description(
                "Number of policy items submitted in a single PUT /v1beta/policies/batch/ call",
            )
            .with_unit("policies")
            .build();
        let service_build_info = meter
            .u64_gauge("service.build_info")
            .with_description(
                "Always 1. Presence plus the version attribute lets dashboards join other \
                 metrics by deployed version.",
            )
            .build();
        service_build_info.record(1, &[KeyValue::new("version", env!("CARGO_PKG_VERSION"))]);

        let evaluation_cache_hits = meter
            .u64_counter("evaluation.cache.hit")
            .with_description("Evaluation-cache lookups that found a cached decision")
            .with_unit("hits")
            .build();
        let evaluation_cache_misses = meter
            .u64_counter("evaluation.cache.miss")
            .with_description("Evaluation-cache lookups that required a fresh policy evaluation")
            .with_unit("misses")
            .build();
        let metadata_cache_hits = meter
            .u64_counter("metadata.cache.hit")
            .with_description("Metadata-cache lookups that found cached metadata")
            .with_unit("hits")
            .build();
        let metadata_cache_misses = meter
            .u64_counter("metadata.cache.miss")
            .with_description("Metadata-cache lookups that required a storage fetch")
            .with_unit("misses")
            .build();
        let policy_set_cache_hits = meter
            .u64_counter("policy_set.cache.hit")
            .with_description("PolicySet-cache lookups that found a parsed Cedar AST")
            .with_unit("hits")
            .build();
        let policy_set_cache_misses = meter
            .u64_counter("policy_set.cache.miss")
            .with_description("PolicySet-cache lookups that required parsing policy text")
            .with_unit("misses")
            .build();

        let evaluation_cache_size = {
            let evaluation = caches.evaluation.clone();
            meter
                .u64_observable_gauge("evaluation.cache.size")
                .with_description("Total amount of evaluation cache entries")
                .with_callback(move |observer| {
                    observer.observe(evaluation.entry_count(), &[]);
                })
                .build()
        };
        let metadata_cache_size = {
            let metadata = caches.metadata.clone();
            meter
                .u64_observable_gauge("metadata.cache.size")
                .with_description("Total amount of metadata cache entries")
                .with_callback(move |observer| {
                    observer.observe(metadata.entry_count(), &[]);
                })
                .build()
        };
        let policy_set_cache_size = {
            let policy_set = caches.policy_set.clone();
            meter
                .u64_observable_gauge("policy_set.cache.size")
                .with_description("Total amount of cached parsed PolicySet ASTs")
                .with_callback(move |observer| {
                    observer.observe(policy_set.entry_count(), &[]);
                })
                .build()
        };
        let token_cache_size = {
            let auth = caches.auth.clone();
            meter
                .u64_observable_gauge("token.cache.size")
                .with_description("Total amount of cached tokens")
                .with_callback(move |observer| {
                    observer.observe(auth.valid_token_count(), &[]);
                })
                .build()
        };
        let invalid_token_cache_size = {
            let auth = caches.auth.clone();
            meter
                .u64_observable_gauge("token.cache.invalid.size")
                .with_description(
                    "Total amount of entries in the negative (invalid-token) cache. \
                     Persistent saturation indicates an attacker or misbehaving client \
                     hammering the auth path with rejected bearers.",
                )
                .with_callback(move |observer| {
                    observer.observe(auth.invalid_token_count(), &[]);
                })
                .build()
        };

        Metrics {
            meter_provider,
            meter,
            authorization_allowed,
            authorization_denied,
            request_count,
            request_errors,
            http_requests_in_flight,
            rpc_request_count,
            rpc_request_duration,
            rpc_request_errors,
            rpc_requests_in_flight,
            db_operation_duration,
            db_operation_errors,
            notification_publish_count,
            notification_publish_duration,
            notification_consume_count,
            notification_consume_errors,
            notification_consume_reconnect_count,
            notification_consume_connected,
            notification_consume_delivery_lag,
            policy_evaluation_depth,
            policy_evaluation_groups_scanned,
            policy_fetch_size,
            policy_mutation_count,
            policy_mutation_batch_size,
            service_build_info,
            evaluation_cache_hits,
            evaluation_cache_misses,
            metadata_cache_hits,
            metadata_cache_misses,
            policy_set_cache_hits,
            policy_set_cache_misses,
            _evaluation_cache_size: evaluation_cache_size,
            _metadata_cache_size: metadata_cache_size,
            _policy_set_cache_size: policy_set_cache_size,
            _token_cache_size: token_cache_size,
            _invalid_token_cache_size: invalid_token_cache_size,
        }
    }

    pub fn record_evaluation_cache(&self, hit: bool) {
        if hit {
            self.evaluation_cache_hits.add(1, &[]);
        } else {
            self.evaluation_cache_misses.add(1, &[]);
        }
    }

    pub fn record_metadata_cache(&self, hit: bool) {
        if hit {
            self.metadata_cache_hits.add(1, &[]);
        } else {
            self.metadata_cache_misses.add(1, &[]);
        }
    }

    pub fn record_policy_set_cache(&self, hit: bool) {
        if hit {
            self.policy_set_cache_hits.add(1, &[]);
        } else {
            self.policy_set_cache_misses.add(1, &[]);
        }
    }
}

impl Drop for Metrics {
    fn drop(&mut self) {
        self.meter_provider.shutdown().unwrap()
    }
}
