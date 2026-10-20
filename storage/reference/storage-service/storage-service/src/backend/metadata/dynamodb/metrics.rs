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
    Attribute, BoundTimerMetric, BoundUIntHistogram, TimerMetric, UIntCounter, UIntHistogram,
};
use storage_service_metrics::{
    batch_size_metric_base, call_latency_metric_base, response_count_metric_base,
    status::http::ResultMetrics,
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINE},
};

pub struct Metrics {
    pub put_item_latency: BoundTimerMetric,
    pub put_item_count: ResultMetrics,
    pub delete_item_latency: BoundTimerMetric,
    pub delete_item_count: ResultMetrics,
    pub batch_get_item_latency: BoundTimerMetric,
    pub batch_get_item_count: ResultMetrics,
    pub batch_get_item_request_items: BoundUIntHistogram,
    pub query_latency: BoundTimerMetric,
    pub query_count: ResultMetrics,
}

impl Metrics {
    pub fn new(backend_id: &str) -> Self {
        let op_results = response_count_metric_base!(
            "aws_dynamodb_metadata_operation_results",
            "AWS DynamoDB metadata backend operation results",
            { operation, backend }
        );
        let op_latency = call_latency_metric_base!(
            "aws_dynamodb_metadata_operation_latency",
            "AWS DynamoDB metadata backend operation latency, seconds",
            { operation, backend },
        );
        let op_batch_size = batch_size_metric_base!(
            "aws_dynamodb_metadata_operation_batch_size",
            "AWS DynamoDB metadata backend request batch size, items",
            BATCH_SIZE_BUCKETS_FINE,
            { operation, backend },
        );

        Self {
            put_item_latency: bind_op_latency_metric(op_latency, "put_item", backend_id),
            put_item_count: bind_op_result_metric(op_results, "put_item", backend_id),
            delete_item_latency: bind_op_latency_metric(op_latency, "delete_item", backend_id),
            delete_item_count: bind_op_result_metric(op_results, "delete_item", backend_id),
            batch_get_item_latency: bind_op_latency_metric(op_latency, "batch_get_item", backend_id),
            batch_get_item_count: bind_op_result_metric(op_results, "batch_get_item", backend_id),
            batch_get_item_request_items: bind_op_batch_size_metric(op_batch_size, "batch_get_item", backend_id),
            query_latency: bind_op_latency_metric(op_latency, "query", backend_id),
            query_count: bind_op_result_metric(op_results, "query", backend_id),
        }
    }
}

fn bind_op_result_metric(base: &UIntCounter, operation: &'static str, backend: &str) -> ResultMetrics {
    ResultMetrics::new_with_bind(|result| {
        base.bind_with_default_attributes([
            Attribute::new("operation", operation),
            Attribute::new("backend", backend.to_string()),
            Attribute::new("result", result),
        ])
    })
}

fn bind_op_latency_metric(base: &TimerMetric, operation: &'static str, backend: &str) -> BoundTimerMetric {
    base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("backend", backend.to_string()),
    ])
}

fn bind_op_batch_size_metric(base: &UIntHistogram, operation: &'static str, backend: &str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("backend", backend.to_string()),
    ])
}
