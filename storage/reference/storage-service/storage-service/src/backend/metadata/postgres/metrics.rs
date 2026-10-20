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
    status::result::ResultMetrics,
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINE},
};

pub struct Metrics {
    pub connection_acquire_latency: BoundTimerMetric,
    pub connection_acquire_count: ResultMetrics,
    pub query_all_metadata_for_resource_latency: BoundTimerMetric,
    pub query_all_metadata_for_resource_count: ResultMetrics,
    pub query_all_metadata_for_resource_response_size: BoundUIntHistogram,
    pub query_metadata_batch_latency: BoundTimerMetric,
    pub query_metadata_batch_count: ResultMetrics,
    pub query_metadata_batch_batch_size: BoundUIntHistogram,
    pub query_metadata_batch_response_size: BoundUIntHistogram,
    pub upsert_metadata_batch_latency: BoundTimerMetric,
    pub upsert_metadata_batch_count: ResultMetrics,
    pub upsert_metadata_batch_size: BoundUIntHistogram,
    pub upsert_metadata_batch_response_size: BoundUIntHistogram,
    pub update_metadata_batch_latency: BoundTimerMetric,
    pub update_metadata_batch_count: ResultMetrics,
    pub update_metadata_batch_size: BoundUIntHistogram,
    pub update_metadata_batch_response_size: BoundUIntHistogram,
    pub delete_metadata_batch_latency: BoundTimerMetric,
    pub delete_metadata_batch_count: ResultMetrics,
    pub delete_metadata_batch_size: BoundUIntHistogram,
    pub delete_metadata_batch_response_size: BoundUIntHistogram,
}

impl Metrics {
    pub fn new(backend_id: &str) -> Self {
        let op_latency = call_latency_metric_base!(
            "postgres_metadata_query_latency",
            "Postgres metadata backend query latency, seconds",
            { query, backend },
        );
        let op_results = response_count_metric_base!(
            "postgres_metadata_query_results",
            "Postgres metadata backend query results",
            { query, backend },
        );
        let op_batch_size = batch_size_metric_base!(
            "postgres_metadata_query_batch_size",
            "Postgres metadata backend query batch size, items",
            BATCH_SIZE_BUCKETS_FINE,
            { query, backend },
        );
        let op_response_size = batch_size_metric_base!(
            "postgres_metadata_query_response_size",
            "Postgres metadata backend response size, items",
            BATCH_SIZE_BUCKETS_FINE,
            { query, backend },
        );

        Self {
            connection_acquire_latency: bind_op_latency(op_latency, "acquire", backend_id),
            connection_acquire_count: bind_op_results(op_results, "acquire", backend_id),
            query_all_metadata_for_resource_latency: bind_op_latency(
                op_latency,
                "query_all_metadata_for_resource",
                backend_id,
            ),
            query_all_metadata_for_resource_count: bind_op_results(
                op_results,
                "query_all_metadata_for_resource",
                backend_id,
            ),
            query_all_metadata_for_resource_response_size: bind_response_size(
                op_response_size,
                "query_all_metadata_for_resource",
                backend_id,
            ),
            query_metadata_batch_latency: bind_op_latency(op_latency, "query_metadata_batch", backend_id),
            query_metadata_batch_count: bind_op_results(op_results, "query_metadata_batch", backend_id),
            query_metadata_batch_batch_size: bind_batch_size(op_batch_size, "query_metadata_batch", backend_id),
            query_metadata_batch_response_size: bind_response_size(
                op_response_size,
                "query_metadata_batch",
                backend_id,
            ),
            upsert_metadata_batch_latency: bind_op_latency(op_latency, "upsert_metadata_batch", backend_id),
            upsert_metadata_batch_count: bind_op_results(op_results, "upsert_metadata_batch", backend_id),
            upsert_metadata_batch_size: bind_batch_size(op_batch_size, "upsert_metadata_batch", backend_id),
            upsert_metadata_batch_response_size: bind_response_size(
                op_response_size,
                "upsert_metadata_batch",
                backend_id,
            ),
            update_metadata_batch_count: bind_op_results(op_results, "update_metadata_batch", backend_id),
            update_metadata_batch_latency: bind_op_latency(op_latency, "update_metadata_batch", backend_id),
            update_metadata_batch_size: bind_batch_size(op_batch_size, "update_metadata_batch", backend_id),
            update_metadata_batch_response_size: bind_response_size(
                op_response_size,
                "update_metadata_batch",
                backend_id,
            ),
            delete_metadata_batch_count: bind_op_results(op_results, "delete_metadata_batch", backend_id),
            delete_metadata_batch_latency: bind_op_latency(op_latency, "delete_metadata_batch", backend_id),
            delete_metadata_batch_size: bind_batch_size(op_batch_size, "delete_metadata_batch", backend_id),
            delete_metadata_batch_response_size: bind_response_size(
                op_response_size,
                "delete_metadata_batch",
                backend_id,
            ),
        }
    }
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

fn bind_op_latency(base: &TimerMetric, query: &'static str, backend: &str) -> BoundTimerMetric {
    base.bind_with_default_attributes([
        Attribute::new("query", query),
        Attribute::new("backend", backend.to_string()),
    ])
}

fn bind_op_results(base: &UIntCounter, query: &'static str, backend: &str) -> ResultMetrics {
    ResultMetrics::new_bind(|result| {
        base.bind_with_default_attributes([
            Attribute::new("query", query),
            Attribute::new("backend", backend.to_string()),
            Attribute::new("result", result),
        ])
    })
}

fn bind_batch_size(base: &UIntHistogram, query: &'static str, backend: &str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([
        Attribute::new("query", query),
        Attribute::new("backend", backend.to_string()),
    ])
}

fn bind_response_size(base: &UIntHistogram, query: &'static str, backend: &str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([
        Attribute::new("query", query),
        Attribute::new("backend", backend.to_string()),
    ])
}
