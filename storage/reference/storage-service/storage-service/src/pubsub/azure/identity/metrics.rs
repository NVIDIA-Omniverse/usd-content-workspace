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

use storage_core_util::observability::metrics::{
    BoundTimerMetric, BoundUIntHistogram, TimerMetric, UIntCounter, UIntHistogram,
};
use storage_service_metrics::{
    batch_size_metric_base, call_latency_metric_base, response_count_metric_base,
    status::{http::ResultMetrics, result::ResultMetrics as SimpleResultMetrics},
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINE},
};

pub struct Metrics {
    pub resolve_identity_latency: BoundTimerMetric,
    pub resolve_identity_results: SimpleResultMetrics,
    pub get_properties_latency: BoundTimerMetric,
    pub get_properties_results: ResultMetrics,
    pub list_blobs_latency: BoundTimerMetric,
    pub list_blobs_results: ResultMetrics,
    pub list_blobs_response_size: BoundUIntHistogram,
}

impl Metrics {
    pub fn new() -> Self {
        let op_latency = call_latency_metric_base!(
            "azure_blob_identity_resolver_operation_latency",
            "Azure Blob identity resolver operation latency, seconds",
            { operation },
        );
        let op_results = response_count_metric_base!(
            "azure_blob_identity_resolver_operation_results",
            "Azure Blob identity resolver operation results",
            { operation },
        );
        let op_batch_size = batch_size_metric_base!(
            "azure_blob_identity_resolver_operation_batch_size",
            "Azure Blob identity resolver operation batch size",
            BATCH_SIZE_BUCKETS_FINE,
            { operation },
        );

        Self {
            resolve_identity_latency: bind_op_latency_metric(op_latency, "resolve"),
            resolve_identity_results: bind_op_simple_results_metric(op_results, "resolve"),
            get_properties_latency: bind_op_latency_metric(op_latency, "get_blob_properties"),
            get_properties_results: bind_op_results_metric(op_results, "get_blob_properties"),
            list_blobs_latency: bind_op_latency_metric(op_latency, "list_blobs"),
            list_blobs_results: bind_op_results_metric(op_results, "list_blobs"),
            list_blobs_response_size: bind_batch_size_metric(op_batch_size, "list_blobs"),
        }
    }
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

fn bind_op_latency_metric(base: &TimerMetric, operation: &'static str) -> BoundTimerMetric {
    base.bind_with_default_attributes([("operation", operation)])
}

fn bind_op_results_metric(base: &UIntCounter, operation: &'static str) -> ResultMetrics {
    ResultMetrics::new_with_bind(|result| {
        base.bind_with_default_attributes([("result", result), ("operation", operation)])
    })
}

fn bind_op_simple_results_metric(base: &UIntCounter, operation: &'static str) -> SimpleResultMetrics {
    SimpleResultMetrics::new_bind(|result| {
        base.bind_with_default_attributes([("result", result), ("operation", operation)])
    })
}

fn bind_batch_size_metric(base: &UIntHistogram, operation: &'static str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([("operation", operation)])
}
