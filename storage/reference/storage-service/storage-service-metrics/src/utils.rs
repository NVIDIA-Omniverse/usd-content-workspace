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

use std::sync::LazyLock;

use storage_core_util::observability::metrics::{Attribute, MetricBinder};

use crate::types::{AsObservable, Observe};

/// Metric value buckets for fine batch size measurements. Value scale factor is 1.5.
pub static BATCH_SIZE_BUCKETS_FINE: [f64; 23] = [
    0., 1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 15., 23., 35., 53., 80., 120., 180., 270., 405., 607., 911., 1367.,
];

/// Metric value buckets for the finest batch size measurements.
pub static BATCH_SIZE_BUCKETS_FINEST: [f64; 23] = [
    0., 1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11., 12., 13., 14., 15., 16., 17., 18., 19., 20., 21., 22.,
];

pub trait BindExt {
    type Bound;

    fn bind_default_attributes(&self) -> Self::Bound {
        self.bind_with_default_attributes([] as [Attribute; 0])
    }

    fn bind_with_default_attributes(&self, attributes: impl IntoIterator<Item = impl Into<Attribute>>) -> Self::Bound;
}

impl<T: MetricBinder> BindExt for T {
    type Bound = T::Bound;

    fn bind_with_default_attributes(&self, attributes: impl IntoIterator<Item = impl Into<Attribute>>) -> Self::Bound {
        static POD_NAME: LazyLock<String> =
            LazyLock::new(|| std::env::var("STORAGE_SERVICE_POD_NAME").unwrap_or("pod".into()));

        self.bind(
            [("pod", POD_NAME.as_str())]
                .into_iter()
                .map(|(key, value)| Attribute::new(key, value))
                .chain(attributes.into_iter().map(Into::into)),
        )
    }
}

#[macro_export]
macro_rules! call_latency_metric_base {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::call_latency_metric_base!($name, $description, {})
    };
    ($name:literal, $description:literal, { $($label:ident),* } $(,)?) => {
        {
            ::storage_core_util::observability::metrics::metric_base!(
                static INSTANCE: TimerMetric,
                name = $name,
                { pod, $($label),* }
            );
            &INSTANCE
        }
    };
}

#[macro_export]
macro_rules! call_latency_metric {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::__bind_empty!(::storage_service_metrics::call_latency_metric_base!(
            $name,
            $description
        ))
    };
}

#[macro_export]
macro_rules! response_count_metric_base {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::response_count_metric_base!($name, $description, {})
    };
    ($name:literal, $description:literal, { $($label:ident),* } $(,)?) => {
        {
            ::storage_core_util::observability::metrics::metric_base!(
                static INSTANCE: UIntCounter,
                name = $name,
                description = $description,
                { pod, result, $($label),* }
            );
            &INSTANCE
        }
    };
}

#[macro_export]
macro_rules! response_count_metric {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::__bind_empty!(::storage_service_metrics::response_count_metric_base!(
            $name,
            $description
        ))
    };
}

#[macro_export]
macro_rules! batch_size_metric_base {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::batch_size_metric_base!(
            $name,
            $description,
            &::storage_service_metrics::utils::BATCH_SIZE_BUCKETS_FINE as &'static [f64],
            {},
        )
    };
    ($name:literal, $description:literal, $buckets:expr, $(,)?) => {
        ::storage_service_metrics::batch_size_metric_base!(
            $name,
            $description,
            $buckets,
            {},
        )
    };
    ($name:literal, $description:literal, $buckets:expr, { $($label:ident),* } $(,)?) => {
        {
            ::storage_core_util::observability::metrics::metric_base!(
                static INSTANCE: UIntHistogram,
                name = $name,
                description = $description,
                buckets = $buckets,
                { pod, $($label),* }
            );
            &INSTANCE
        }
    };
}

#[macro_export]
macro_rules! batch_size_metric {
    ($name:literal, $description:literal, $buckets:expr $(,)?) => {
        ::storage_service_metrics::__bind_empty!(::storage_service_metrics::batch_size_metric_base!(
            $name,
            $description,
            $buckets,
        ))
    };
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::__bind_empty!(::storage_service_metrics::batch_size_metric_base!(
            $name,
            $description
        ),)
    };
}

#[macro_export]
macro_rules! active_call_count_metric_base {
    ($name:literal, $description:literal $(,)?) => {
        ::storage_service_metrics::active_call_count_metric_base!($name, $description, {})
    };
    ($name:literal, $description:literal, { $($label:ident),* } $(,)?) => {
        {
            ::storage_core_util::observability::metrics::metric_base!(
                static INSTANCE: UIntGauge,
                name = $name,
                description = $description,
                { pod, $($label),* }
            );
            &INSTANCE
        }
    };
}

#[macro_export]
macro_rules! active_call_count_metric {
    ($name:literal, $description:literal $(,)?) => {{
        let instrument = ::storage_service_metrics::__bind_empty!(
            ::storage_service_metrics::active_call_count_metric_base!($name, $description)
        );
        ::storage_service_metrics::counter::UpDownCounter::new(instrument)
    }};
}

#[macro_export]
macro_rules! http_operation_result_metrics {
    ($name:literal, $description:literal $(,)?) => {{
        ::storage_service_metrics::status::http::ResultMetrics::new(
            ::storage_service_metrics::response_count_metric_base!($name, $description),
        )
    }};
}

#[macro_export]
macro_rules! grpc_operation_result_metrics {
    ($name:literal, $description:literal $(,)?) => {{
        ::storage_service_metrics::status::grpc::ResultMetrics::new(
            ::storage_service_metrics::response_count_metric_base!($name, $description),
        )
    }};
}

#[macro_export]
macro_rules! operation_result_metrics {
    ($name:literal, $description:literal $(,)?) => {{
        ::storage_service_metrics::status::result::ResultMetrics::new(
            ::storage_service_metrics::response_count_metric_base!($name, $description),
        )
    }};
}

#[macro_export]
macro_rules! __bind_empty {
    ($val:expr $(,)?) => {{
        use ::storage_service_metrics::utils::BindExt;
        ($val).bind_default_attributes()
    }};
}

pub fn record_metric_map<'a, M, V, Map>(metric: M, map: Map) -> impl 'a + FnOnce(&V)
where
    M: 'a + Observe,
    Map: 'a + FnOnce(&V) -> M::Value,
{
    move |result| {
        metric.observe(&map(result));
    }
}

pub fn record_metric_value<'a, M, V, R>(metric: M, value: V) -> impl 'a + FnOnce(&R)
where
    M: 'a + Observe,
    V: 'a + AsObservable<M::Value>,
    M::Value: 'static,
{
    record_metric_map(metric, move |_| value.as_observable())
}

pub fn record_metric<'a, M, V>(metric: M) -> impl 'a + FnOnce(&V)
where
    M: 'a + Observe,
    V: AsObservable<M::Value>,
    M::Value: 'static,
{
    record_metric_map(metric, |result: &V| result.as_observable())
}
