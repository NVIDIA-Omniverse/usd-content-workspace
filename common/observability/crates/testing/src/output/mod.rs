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

pub(super) mod metric;
pub(super) mod span;

pub use metric::{Metric, MetricData, MetricValue, OutputMetrics};
use opentelemetry::trace::SpanContext;
use otel_asserts::types::{common, metrics, trace};
pub use span::{Event, OutputSpans, SpanName, SpanRef};

fn attrs(resource: &otel_tools::resource::Resource) -> Vec<common::KeyValue> {
    resource
        .iter()
        .map(|(k, v)| common::KeyValue::new(k.as_str(), common::Value::from(v.clone())))
        .collect()
}

#[derive(Debug)]
pub struct Output<T> {
    pub root: SpanContext,
    pub server_uri: String,
    pub metrics: metrics::OutputMetrics,
    pub spans: trace::OutputSpans,
    pub res: T,
}

impl<T> Output<T> {
    pub fn response(&self) -> &T {
        &self.res
    }
}
