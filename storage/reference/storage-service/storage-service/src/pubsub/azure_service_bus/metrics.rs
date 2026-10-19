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
use storage_core_util::observability::metrics::{BoundTimerMetric, BoundUIntHistogram};
use storage_service_metrics::{
    active_call_count_metric_base, batch_size_metric_base, call_latency_metric_base, counter::UpDownCounter,
    response_count_metric_base, status::result::ResultMetrics, utils::BATCH_SIZE_BUCKETS_FINEST,
};

use crate::pubsub::metrics::{
    bind_batch_size_metric, bind_op_active_requests_metric, bind_op_latency_metric, bind_op_simple_result_metric,
};

/// Azure Service Bus subscriber metrics.
pub struct Metrics {
    pub(super) receive_messages_latency: BoundTimerMetric,
    pub(super) receive_messages_count: ResultMetrics,
    pub(super) receive_messages_batch_size: BoundUIntHistogram,
    pub(super) receive_messages_active_requests: UpDownCounter,
    pub(super) complete_message_latency: BoundTimerMetric,
    pub(super) complete_message_count: ResultMetrics,
    pub(super) complete_message_active_requests: UpDownCounter,
}

impl Metrics {
    /// Create an instrument set.
    pub fn new(subscriber_id: &str) -> Self {
        let op_results = response_count_metric_base!(
            "azure_service_bus_operation_results",
            "Azure Service Bus subscriber operation results",
            { operation, subscriber },
        );
        let op_latency = call_latency_metric_base!(
            "azure_service_bus_operation_latency",
            "Azure Service Bus subscriber operation latency, seconds",
            { operation, subscriber },
        );
        let op_active_requests = active_call_count_metric_base!(
            "azure_service_bus_operation_active_requests",
            "Azure Service Bus subscriber active request count",
            { operation, subscriber },
        );
        let op_batch_size = batch_size_metric_base!(
            "azure_service_bus_operation_batch_size",
            "Azure Service Bus operation batch size",
            &BATCH_SIZE_BUCKETS_FINEST as &'static [f64],
            { operation, subscriber },
        );

        Self {
            receive_messages_count: bind_op_simple_result_metric(op_results, "receive_messages", subscriber_id),
            receive_messages_latency: bind_op_latency_metric(op_latency, "receive_messages", subscriber_id),
            receive_messages_batch_size: bind_batch_size_metric(op_batch_size, "receive_messages", subscriber_id),
            receive_messages_active_requests: bind_op_active_requests_metric(
                op_active_requests,
                "receive_messages",
                subscriber_id,
            ),
            complete_message_count: bind_op_simple_result_metric(op_results, "complete_message", subscriber_id),
            complete_message_latency: bind_op_latency_metric(op_latency, "complete_message", subscriber_id),
            complete_message_active_requests: bind_op_active_requests_metric(
                op_active_requests,
                "complete_message",
                subscriber_id,
            ),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}
