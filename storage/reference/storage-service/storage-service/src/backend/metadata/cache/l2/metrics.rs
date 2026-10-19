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

use std::fmt::Debug;

use storage_core_util::observability::metrics::{
    metric_base, BoundTimerMetric, BoundUIntCounter, TimerMetric, UIntCounter,
};
use storage_service_metrics::{
    call_latency_metric_base, response_count_metric_base,
    types::{AsObservable, Observe},
    utils::BindExt,
};

pub struct Metrics {
    pub cache_access: BoundUIntCounter,
    pub cache_miss: BoundUIntCounter,
    pub write_operation_results: Results,
    pub write_operation_latency: BoundTimerMetric,
    pub invalidate_operation_results: Results,
    pub invalidate_operation_latency: BoundTimerMetric,
    pub get_or_claim_operation_results: Results,
    pub get_or_claim_operation_latency: BoundTimerMetric,
}

impl Metrics {
    pub fn new() -> Self {
        metric_base!(
            static CACHE_ACCESS_BASE: UIntCounter,
            name = "metadata_cache_l2_access",
            description = "Metadata cache access count, Redis backend",
            { pod }
        );
        metric_base!(
            static CACHE_MISS_BASE: UIntCounter,
            name = "metadata_cache_l2_miss",
            description = "Metadata cache miss count, Redis backend",
            { pod }
        );

        let op_results_base = response_count_metric_base!(
            "metadata_cache_l2_operation_results",
            "Metadata Cache operation results, Redis backend",
            { operation },
        );
        let op_latency_base = call_latency_metric_base!(
            "metadata_cache_l2_operation_latency",
            "Metadata Cache operation latency, seconds, Redis backend",
            { operation },
        );

        Self {
            cache_access: CACHE_ACCESS_BASE.bind_default_attributes(),
            cache_miss: CACHE_MISS_BASE.bind_default_attributes(),
            write_operation_results: bind_op_result_metric(op_results_base, "write"),
            write_operation_latency: bind_op_latency_metric(op_latency_base, "write"),
            invalidate_operation_results: bind_op_result_metric(op_results_base, "invalidate"),
            invalidate_operation_latency: bind_op_latency_metric(op_latency_base, "invalidate"),
            get_or_claim_operation_results: bind_op_result_metric(op_results_base, "get_or_claim"),
            get_or_claim_operation_latency: bind_op_latency_metric(op_latency_base, "get_or_claim"),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub enum Status {
    Ok,
    Err(fred::error::ErrorKind),
}

pub struct Results {
    ok: BoundUIntCounter,
    auth_error: BoundUIntCounter,
    io_error: BoundUIntCounter,
    routing_error: BoundUIntCounter,
    invalid_argument_error: BoundUIntCounter,
    invalid_command_error: BoundUIntCounter,
    url_error: BoundUIntCounter,
    protocol_error: BoundUIntCounter,
    unknown_error: BoundUIntCounter,
    cancelled_error: BoundUIntCounter,
    cluster_error: BoundUIntCounter,
    timeout_error: BoundUIntCounter,
    tls_error: BoundUIntCounter,
    config_error: BoundUIntCounter,
    parse_error: BoundUIntCounter,
    sentinel_error: BoundUIntCounter,
    not_found_error: BoundUIntCounter,
    backpressure_error: BoundUIntCounter,
}

impl Results {
    fn new_with_bind(bind: impl Fn(&'static str) -> BoundUIntCounter) -> Self {
        Self {
            ok: bind("Ok"),
            auth_error: bind("AuthError"),
            io_error: bind("IOError"),
            routing_error: bind("RoutingError"),
            invalid_argument_error: bind("InvalidArgumentError"),
            invalid_command_error: bind("InvalidCommandError"),
            url_error: bind("UrlError"),
            protocol_error: bind("ProtocolError"),
            unknown_error: bind("UnknownError"),
            cancelled_error: bind("CancelledError"),
            cluster_error: bind("ClusterError"),
            timeout_error: bind("TimeoutError"),
            tls_error: bind("TlsError"),
            config_error: bind("ConfigError"),
            parse_error: bind("ParseError"),
            sentinel_error: bind("SentinelError"),
            not_found_error: bind("NotFoundError"),
            backpressure_error: bind("BackpressureError"),
        }
    }

    fn get_counter_for_kind(&self, value: &Status) -> &BoundUIntCounter {
        use fred::error::ErrorKind::*;

        if let Status::Err(error) = value {
            match error {
                Auth => &self.auth_error,
                IO => &self.io_error,
                Routing => &self.routing_error,
                InvalidArgument => &self.invalid_argument_error,
                InvalidCommand => &self.invalid_command_error,
                Url => &self.url_error,
                Protocol => &self.protocol_error,
                Unknown => &self.unknown_error,
                Canceled => &self.cancelled_error,
                Cluster => &self.cluster_error,
                Timeout => &self.timeout_error,
                Tls => &self.tls_error,
                Config => &self.config_error,
                Parse => &self.parse_error,
                Sentinel => &self.sentinel_error,
                NotFound => &self.not_found_error,
                Backpressure => &self.backpressure_error,
            }
        } else {
            &self.ok
        }
    }
}

impl Observe for &Results {
    type Value = Status;

    fn observe(self, value: &Status) {
        self.get_counter_for_kind(value).add(1);
    }
}

impl AsObservable<Status> for Status {
    fn as_observable(&self) -> Status {
        self.clone()
    }
}

impl AsObservable<Status> for fred::error::Error {
    fn as_observable(&self) -> Status {
        Status::Err(self.kind().clone())
    }
}

fn bind_op_latency_metric(base: &TimerMetric, operation: &'static str) -> BoundTimerMetric {
    base.bind_with_default_attributes([("operation", operation)])
}

fn bind_op_result_metric(base: &UIntCounter, operation: &'static str) -> Results {
    Results::new_with_bind(|result| base.bind_with_default_attributes([("operation", operation), ("result", result)]))
}
