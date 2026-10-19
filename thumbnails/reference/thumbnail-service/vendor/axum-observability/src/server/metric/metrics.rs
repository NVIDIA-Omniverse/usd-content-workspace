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

use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry_semantic_conventions::metric as sem;

const HTTP_SERVER_REQUEST_STARTED: &str = "http.server.request.started";

/// See https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverrequestduration
const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

#[derive(Debug, Clone)]
pub(super) struct RequestMetrics {
    pub(super) request_started: Counter<u64>,
    pub(super) request_size: Histogram<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct ResponseMetrics {
    pub(super) request_duration: Histogram<f64>,
    pub(super) response_size: Histogram<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct Metrics {
    pub(super) req_metrics: RequestMetrics,
    pub(super) resp_metrics: ResponseMetrics,
}

impl Metrics {
    pub(super) fn new(meter: &Meter) -> Self {
        // custom metric; this mirrors the gRPC A66 `grpc.server.call.started`
        // metric.
        let request_started = meter
            .u64_counter(HTTP_SERVER_REQUEST_STARTED)
            .with_unit("{request}")
            .build();

        // See https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverrequestbodysize
        let request_size = meter
            .u64_histogram(sem::HTTP_SERVER_REQUEST_BODY_SIZE)
            .with_unit("By")
            .build();

        let req_metrics = RequestMetrics {
            request_started,
            request_size,
        };

        // See https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverrequestduration
        // the HTTP metrics really do not consider streaming; the only guidance
        // is:
        // > the metric value SHOULD be the same as the HTTP server span
        // > duration.
        let request_duration = meter
            .f64_histogram(sem::HTTP_SERVER_REQUEST_DURATION)
            .with_unit("s")
            .with_boundaries(DURATION_BUCKETS.to_vec())
            .build();

        // See https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverresponsebodysize
        let response_size = meter
            .u64_histogram(sem::HTTP_SERVER_RESPONSE_BODY_SIZE)
            .with_unit("By")
            .build();

        let resp_metrics = ResponseMetrics {
            request_duration,
            response_size,
        };

        Self {
            req_metrics,
            resp_metrics,
        }
    }
}
