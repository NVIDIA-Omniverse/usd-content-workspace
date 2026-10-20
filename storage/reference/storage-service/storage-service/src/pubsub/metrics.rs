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
    Attribute, BoundTimerMetric, BoundUIntHistogram, TimerMetric, UIntCounter, UIntGauge, UIntHistogram,
};
use storage_service_metrics::{counter::UpDownCounter, status, utils::BindExt as _};

pub fn bind_op_latency_metric(base: &TimerMetric, operation: &'static str, subscriber: &str) -> BoundTimerMetric {
    base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("subscriber", subscriber.to_string()),
    ])
}

pub fn bind_op_simple_result_metric(
    base: &UIntCounter,
    operation: &'static str,
    subscriber: &str,
) -> status::result::ResultMetrics {
    status::result::ResultMetrics::new_bind(|result| {
        base.bind_with_default_attributes([
            Attribute::new("operation", operation),
            Attribute::new("subscriber", subscriber.to_string()),
            Attribute::new("result", result),
        ])
    })
}

pub fn bind_op_http_result_metric(
    base: &UIntCounter,
    operation: &'static str,
    subscriber: &str,
) -> status::http::ResultMetrics {
    status::http::ResultMetrics::new_with_bind(|result| {
        base.bind_with_default_attributes([
            Attribute::new("operation", operation),
            Attribute::new("subscriber", subscriber.to_string()),
            Attribute::new("result", result),
        ])
    })
}

pub fn bind_op_active_requests_metric(base: &UIntGauge, operation: &'static str, subscriber: &str) -> UpDownCounter {
    UpDownCounter::new(base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("subscriber", subscriber.to_string()),
    ]))
}

pub fn bind_batch_size_metric(base: &UIntHistogram, operation: &'static str, subscriber: &str) -> BoundUIntHistogram {
    base.bind_with_default_attributes([
        Attribute::new("operation", operation),
        Attribute::new("subscriber", subscriber.to_string()),
    ])
}
