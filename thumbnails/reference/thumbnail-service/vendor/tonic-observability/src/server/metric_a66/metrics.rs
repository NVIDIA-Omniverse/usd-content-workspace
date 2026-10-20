// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};

/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments
const GRPC_SERVER_CALL_STARTED: &str = "grpc.server.call.started";
/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments
const GRPC_SERVER_CALL_DURATION: &str = "grpc.server.call.duration";
/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments
const GRPC_SERVER_CALL_TX_SIZE: &str = "grpc.server.call.sent_total_compressed_message_size";
/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments
const GRPC_SERVER_CALL_RX_SIZE: &str = "grpc.server.call.rcvd_total_compressed_message_size";

const GRPC_SERVER_HANDLER_ACTIVE: &str = "grpc.server.handler.active";
const GRPC_SERVER_REQUEST_ACTIVE: &str = "grpc.server.request.active";
const GRPC_SERVER_RESPONSE_ACTIVE: &str = "grpc.server.response.active";

/// OpenTelemetry buckets are always f64
macro_rules! buckets {
    ($($v:literal),+ $(,)?) => {
        &[$($v as f64,)+]
    };
}

/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#units
const LATENCY_BUCKETS: &[f64] = buckets![
    0, 0.00001, 0.00005, 0.0001, 0.0003, 0.0006, 0.0008, 0.001, 0.002, 0.003, 0.004, 0.005, 0.006,
    0.008, 0.01, 0.013, 0.016, 0.02, 0.025, 0.03, 0.04, 0.05, 0.065, 0.08, 0.1, 0.13, 0.16, 0.2,
    0.25, 0.3, 0.4, 0.5, 0.65, 0.8, 1, 2, 5, 10, 20, 50, 100,
];
/// See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#units
const SIZE_BUCKETS: &[f64] = buckets![
    0,
    1024,
    2048,
    4096,
    16384,
    65536,
    262144,
    1048576,
    4194304,
    16777216,
    67108864,
    268435456,
    1073741824,
    4294967296f64,
];

#[derive(Debug, Clone)]
pub(super) struct CallMetrics {
    pub(super) started: Counter<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct RequestMetrics {
    pub(super) request_size: Histogram<u64>,
    pub(super) active_requests: UpDownCounter<i64>,
}

#[derive(Debug, Clone)]
pub(super) struct ResponseMetrics {
    pub(super) response_size: Histogram<u64>,
    pub(super) active_responses: UpDownCounter<i64>,
    pub(super) call_duration: Histogram<f64>,
    pub(super) active_handlers: UpDownCounter<i64>,
}

#[derive(Debug, Clone)]
pub(super) struct Metrics {
    pub(super) call_metrics: CallMetrics,
    pub(super) req_metrics: RequestMetrics,
    pub(super) resp_metrics: ResponseMetrics,
}

impl Metrics {
    pub(super) fn new(meter: &Meter) -> Self {
        // See https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments

        let started = meter
            .u64_counter(GRPC_SERVER_CALL_STARTED)
            .with_description(
                "The total number of RPCs started, including those that have not completed.",
            )
            .with_unit("{call}")
            .build();

        let call_metrics = CallMetrics { started };

        let request_size = meter
            .u64_histogram(GRPC_SERVER_CALL_RX_SIZE)
            .with_description("Total bytes (compressed but not encrypted) received across all request messages (metadata excluded) per RPC; does not include grpc or transport framing bytes.")
            .with_unit("By")
            .with_boundaries(SIZE_BUCKETS.to_vec())
            .build();

        let active_requests = meter
            .i64_up_down_counter(GRPC_SERVER_REQUEST_ACTIVE)
            .with_unit("{request}")
            .build();

        let req_metrics = RequestMetrics {
            request_size,
            active_requests,
        };

        let response_size = meter
            .u64_histogram(GRPC_SERVER_CALL_TX_SIZE)
            .with_description("Total bytes (compressed but not encrypted) sent across all response messages (metadata excluded) per RPC; does not include grpc or transport framing bytes.")
            .with_unit("By")
            .with_boundaries(SIZE_BUCKETS.to_vec())
            .build();

        let active_responses = meter
            .i64_up_down_counter(GRPC_SERVER_RESPONSE_ACTIVE)
            .with_unit("{response}")
            .build();

        let call_duration = meter
            .f64_histogram(GRPC_SERVER_CALL_DURATION)
            .with_description("This metric aims to measure the end-to-end time an RPC takes from the server transport's perspective.")
            .with_unit("s")
            .with_boundaries(LATENCY_BUCKETS.to_vec())
            .build();

        let active_handlers = meter
            .i64_up_down_counter(GRPC_SERVER_HANDLER_ACTIVE)
            .with_unit("{handler}")
            .build();

        let resp_metrics = ResponseMetrics {
            response_size,
            active_responses,
            call_duration,
            active_handlers,
        };

        Self {
            call_metrics,
            req_metrics,
            resp_metrics,
        }
    }
}
