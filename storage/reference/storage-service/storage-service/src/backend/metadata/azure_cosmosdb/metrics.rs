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

use storage_core_util::observability::metrics::{Attribute, BoundTimerMetric, TimerMetric, UIntCounter};
use storage_service_metrics::{
    call_latency_metric_base, response_count_metric_base, status::http::ResultMetrics, utils::BindExt,
};

/// CosmosDB metadata backend metrics.
pub struct Metrics {
    pub query_documents_latency: BoundTimerMetric,
    pub query_documents_count: ResultMetrics,
    pub create_document_latency: BoundTimerMetric,
    pub create_document_count: ResultMetrics,
    pub delete_document_latency: BoundTimerMetric,
    pub delete_document_count: ResultMetrics,
    pub get_document_latency: BoundTimerMetric,
    pub get_document_count: ResultMetrics,
}

impl Metrics {
    pub fn new(backend_id: &str) -> Self {
        let op_results = response_count_metric_base!(
            "azure_cosmosdb_metadata_operation_results",
            "Azure CosmosDB metadata backend operation results",
            { operation, backend },
        );
        let op_latency = call_latency_metric_base!(
            "azure_cosmosdb_metadata_operation_latency",
            "Azure CosmosDB metadata backend operation latency, seconds",
            { operation, backend },
        );

        Self {
            query_documents_latency: bind_op_latency_metric(op_latency, "query_documents", backend_id),
            query_documents_count: bind_op_result_metric(op_results, "query_documents", backend_id),
            create_document_latency: bind_op_latency_metric(op_latency, "create_document", backend_id),
            create_document_count: bind_op_result_metric(op_results, "create_document", backend_id),
            delete_document_latency: bind_op_latency_metric(op_latency, "delete_document", backend_id),
            delete_document_count: bind_op_result_metric(op_results, "delete_document", backend_id),
            get_document_latency: bind_op_latency_metric(op_latency, "get_document", backend_id),
            get_document_count: bind_op_result_metric(op_results, "get_document", backend_id),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

fn bind_op_latency_metric(base: &TimerMetric, operation: &'static str, backend: &str) -> BoundTimerMetric {
    base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("backend", backend.to_string()),
    ])
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
