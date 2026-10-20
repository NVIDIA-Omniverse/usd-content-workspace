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

use storage_core_util::observability::metrics::{metric_base, BoundTimerMetric, BoundUIntCounter, BoundUIntHistogram};
use storage_service_metrics::{
    active_call_count_metric, batch_size_metric, call_latency_metric,
    counter::UpDownCounter,
    grpc_operation_result_metrics,
    status::grpc::ResultMetrics,
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINE},
};

pub struct Metrics {
    pub(super) batch_publish_events_latency: BoundTimerMetric,
    pub(super) batch_publish_events_count: ResultMetrics,
    pub(super) batch_publish_events_active_requests: UpDownCounter,
    pub(super) batch_publish_events_batch_size: BoundUIntHistogram,
    pub(super) batch_publish_events_event_count: BoundUIntCounter,
}

impl Metrics {
    /// Create an instrument set.
    pub fn new() -> Self {
        metric_base!(
            static BATCH_PUBLISH_EVENTS_EVENT_COUNT: UIntCounter,
            name = "notification_service_batch_publish_events_event_count",
            description = "Number of events submitted to Notification Service BatchPublishEvents",
            { pod }
        );

        Self {
            batch_publish_events_latency: call_latency_metric!(
                "notification_service_batch_publish_events_latency",
                "Notification Service BatchPublishEvents operation latency",
            ),
            batch_publish_events_count: grpc_operation_result_metrics!(
                "notification_service_batch_publish_events_count",
                "Notification Service BatchPublishEvents operation results",
            ),
            batch_publish_events_active_requests: active_call_count_metric!(
                "notification_service_batch_publish_events_active_requests",
                "Notification Service BatchPublishEvents active request count",
            ),
            batch_publish_events_batch_size: batch_size_metric!(
                "notification_service_batch_publish_events_batch_size",
                "Notification Service BatchPublishEvents request size",
                &BATCH_SIZE_BUCKETS_FINE as &'static [f64],
            ),
            batch_publish_events_event_count: BATCH_PUBLISH_EVENTS_EVENT_COUNT.bind_default_attributes(),
        }
    }
}
