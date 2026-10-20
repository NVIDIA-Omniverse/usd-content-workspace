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

use storage_core_util::observability::metrics::{Attribute, BoundTimerMetric, BoundUIntHistogram, UIntHistogram};
use storage_service_metrics::{
    active_call_count_metric_base, batch_size_metric_base, call_latency_metric_base,
    counter::UpDownCounter,
    response_count_metric_base,
    status::http::ResultMetrics,
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINEST},
};

use crate::pubsub::metrics::{
    bind_batch_size_metric, bind_op_active_requests_metric, bind_op_http_result_metric, bind_op_latency_metric,
};

/// SQS subscriber metrics.
pub struct Metrics {
    pub(super) delete_message_batch_latency: BoundTimerMetric,
    pub(super) delete_message_batch_count: ResultMetrics,
    pub(super) delete_message_batch_batch_size: BoundUIntHistogram,
    pub(super) delete_message_active_requests: UpDownCounter,
    pub(super) receive_message_latency: BoundTimerMetric,
    pub(super) receive_message_count: ResultMetrics,
    pub(super) receive_message_batch_size: BoundUIntHistogram,
    pub(super) receive_message_active_requests: UpDownCounter,
    pub(super) events_per_message: BoundUIntHistogram,
}

impl Metrics {
    /// Create an instrument set.
    pub fn new(subscriber_id: &str) -> Self {
        let op_results = response_count_metric_base!(
            "sqs_operation_results",
            "SQS subscriber operation results",
            { operation, subscriber },
        );
        let op_latency = call_latency_metric_base!(
            "sqs_operation_latency",
            "SQS subscriber operation latency, seconds",
            { operation, subscriber },
        );
        let op_active_requests = active_call_count_metric_base!(
            "sqs_operation_active_requests",
            "SQS subscriber active request count",
            { operation, subscriber },
        );
        let op_batch_size = batch_size_metric_base!(
            "sqs_operation_batch_size",
            "SQS subscriber operation batch size, items",
            &BATCH_SIZE_BUCKETS_FINEST as &'static [f64],
            { operation, subscriber },
        );
        let op_events_per_message = batch_size_metric_base!(
            "sqs_events_per_message",
            "SQS subscriber events per message",
            &BATCH_SIZE_BUCKETS_FINEST as &'static [f64],
            { subscriber },
        );

        Self {
            delete_message_batch_latency: bind_op_latency_metric(op_latency, "delete_message_batch", subscriber_id),
            delete_message_batch_count: bind_op_http_result_metric(op_results, "delete_message_batch", subscriber_id),
            delete_message_batch_batch_size: bind_batch_size_metric(
                op_batch_size,
                "delete_message_batch",
                subscriber_id,
            ),
            delete_message_active_requests: bind_op_active_requests_metric(
                op_active_requests,
                "delete_message_batch",
                subscriber_id,
            ),
            receive_message_latency: bind_op_latency_metric(op_latency, "receive_message", subscriber_id),
            receive_message_count: bind_op_http_result_metric(op_results, "receive_message", subscriber_id),
            receive_message_batch_size: bind_batch_size_metric(op_batch_size, "receive_message", subscriber_id),
            receive_message_active_requests: bind_op_active_requests_metric(
                op_active_requests,
                "receive_message",
                subscriber_id,
            ),
            events_per_message: bind_op_events_per_message_metric(op_events_per_message, subscriber_id),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

fn bind_op_events_per_message_metric(base: &UIntHistogram, subscriber: &str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([Attribute::new("subscriber", subscriber.to_string())])
}
