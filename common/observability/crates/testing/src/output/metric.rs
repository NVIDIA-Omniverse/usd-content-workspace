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

use super::{Output, attrs};
use otel_asserts::types::{common, metrics, resource};

impl<T> Output<T> {
    pub fn metrics<'a>(&'a self, resource: &otel_tools::resource::Resource) -> OutputMetrics<'a> {
        let resource_metric = match &self.metrics.resource_metrics[..] {
            [one] => one,
            _ => panic!(
                "expected 1 metrics export, but got {}: {:?}",
                self.metrics.resource_metrics.len(),
                self.metrics.resource_metrics
            ),
        };

        let expected_resource = Some(resource::Resource::new(attrs(resource)));
        let expected_scope = Some(common::InstrumentationScope::new("test", "", Vec::new()));

        let mut metrics = Vec::new();

        assert_eq!(
            resource_metric.resource, expected_resource,
            "resource_metric.resource"
        );
        for scope_metric in &resource_metric.scope_metrics {
            assert_eq!(scope_metric.scope, expected_scope, "scope_metric.scope");

            for metric in &scope_metric.metrics {
                metrics.push(metric);
            }
        }

        OutputMetrics {
            server_uri: &self.server_uri,
            raw: metrics,
        }
    }
}

#[derive(Debug)]
pub struct OutputMetrics<'a> {
    pub server_uri: &'a str,
    pub raw: Vec<&'a metrics::Metric>,
}

impl OutputMetrics<'_> {
    #[track_caller]
    pub fn assert(&self, expected: Vec<Metric<'_>>) {
        let actual = self.normalized_actual();

        pretty_assertions::assert_eq!(Self::normalize(expected), actual);
    }

    /// Assert the actual metrics match one of several accepted shapes.
    ///
    /// Use this when a scenario is genuinely racy and more than one outcome is
    /// correct -- for example a client that aborts a request mid-flight, where
    /// the server may or may not get far enough to record the call. Asserting a
    /// single shape there produces a flaky test; asserting that the outcome is
    /// one of the shapes the system is allowed to produce does not.
    ///
    /// Prefer [`Self::assert`]. Only reach for this when the non-determinism is
    /// in the system under test rather than in the test.
    ///
    /// On failure the diff is shown against the first candidate.
    #[track_caller]
    pub fn assert_one_of(&self, candidates: Vec<Vec<Metric<'_>>>) {
        assert!(!candidates.is_empty(), "no candidate metric sets given");

        let actual = self.normalized_actual();
        let candidates = candidates
            .into_iter()
            .map(Self::normalize)
            .collect::<Vec<_>>();

        if candidates.contains(&actual) {
            return;
        }

        eprintln!(
            "actual metrics matched none of the {} accepted shapes; \
             showing the diff against the first",
            candidates.len()
        );
        let first = candidates.into_iter().next().expect("checked non-empty");
        pretty_assertions::assert_eq!(first, actual);
    }

    fn normalize(mut expected: Vec<Metric<'_>>) -> Vec<Metric<'_>> {
        for metric in &mut expected {
            metric.attrs.sort_by(|a, b| a.key().cmp(b.key()));
        }
        expected.sort_by(|a, b| a.name.cmp(b.name));

        expected
    }

    fn normalized_actual(&self) -> Vec<Metric<'_>> {
        let mut actual = self
            .raw
            .iter()
            .copied()
            .map(|metric| {
                Metric::transform(metric).unwrap_or_else(|msg| panic!("{msg}: {metric:#?}"))
            })
            .collect::<Vec<Metric<'_>>>();
        actual.sort_by(|a, b| a.name.cmp(b.name));

        actual
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metric<'a> {
    pub name: &'a str,
    pub unit: &'a str,
    pub attrs: Vec<common::KeyValue>,
    pub data: MetricData,
}

impl<'a> Metric<'a> {
    fn transform(metric: &'a metrics::Metric) -> Result<Self, &'static str> {
        let name = metric.name.as_str();
        let unit = metric.unit.as_str();

        let (attrs, data) = match &metric.data {
            None => return Err("metric has no data"),
            Some(metrics::Data::Gauge(gauge)) => match &gauge.data_points[..] {
                [one] => {
                    let attrs = &one.attributes;
                    let value = one.value.ok_or("metric has no value")?;
                    let data = MetricData::Gauge { value };
                    (attrs, data)
                }
                _ => return Err("metric has several data points"),
            },
            Some(metrics::Data::Sum(sum)) => {
                assert_eq!(
                    sum.aggregation_temporality,
                    Ok(metrics::AggregationTemporality::Cumulative)
                );

                match &sum.data_points[..] {
                    [one] => {
                        let attrs = &one.attributes;
                        let value = one.value.ok_or("metric has no value")?;
                        let data = MetricData::Sum { value };
                        (attrs, data)
                    }
                    _ => return Err("metric has several data points"),
                }
            }
            Some(metrics::Data::Summary(summary)) => match &summary.data_points[..] {
                [one] => {
                    let attrs = &one.attributes;
                    let data = MetricData::Summary {
                        count: one.count,
                        sum: one.sum,
                    };
                    (attrs, data)
                }
                _ => return Err("metric has several data points"),
            },
            Some(metrics::Data::Histogram(histogram)) => {
                assert_eq!(
                    histogram.aggregation_temporality,
                    Ok(metrics::AggregationTemporality::Cumulative)
                );

                match &histogram.data_points[..] {
                    [one] => {
                        let attrs = &one.attributes;
                        let data = MetricData::Histogram {
                            count: one.count,
                            sum: one.sum.into(),
                        };
                        (attrs, data)
                    }
                    _ => return Err("metric has several data points"),
                }
            }
            Some(metrics::Data::ExponentialHistogram(histogram)) => {
                assert_eq!(
                    histogram.aggregation_temporality,
                    Ok(metrics::AggregationTemporality::Cumulative)
                );

                match &histogram.data_points[..] {
                    [one] => {
                        let attrs = &one.attributes;
                        let data = MetricData::ExpHistogram {
                            count: one.count,
                            sum: one.sum,
                        };
                        (attrs, data)
                    }
                    _ => return Err("metric has several data points"),
                }
            }
        };

        let mut attrs = attrs.clone();
        attrs.sort_by(|a, b| a.key().cmp(b.key()));

        Ok(Self {
            name,
            unit,
            attrs,
            data,
        })
    }
}

#[derive(Debug, Clone)]
pub enum MetricValue {
    None,
    Some(f64),
    GreaterThanZero,
}

impl PartialEq for MetricValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Some(a), Self::Some(b)) => a.eq(b),
            (Self::GreaterThanZero, Self::GreaterThanZero) => false,
            (Self::GreaterThanZero, Self::Some(v)) | (Self::Some(v), Self::GreaterThanZero) => {
                *v > 0.0
            }
            _unused => false,
        }
    }
}

impl From<Option<f64>> for MetricValue {
    fn from(value: Option<f64>) -> Self {
        match value {
            Some(v) => Self::Some(v),
            None => Self::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetricData {
    Gauge {
        value: metrics::NumberDataPointValue,
    },
    Sum {
        value: metrics::NumberDataPointValue,
    },
    Summary {
        count: u64,
        sum: f64,
    },
    Histogram {
        count: u64,
        sum: MetricValue,
    },
    ExpHistogram {
        count: u64,
        sum: Option<f64>,
    },
}

#[macro_export]
macro_rules! expected_grpc_metrics {
    // Same as the client-only `build` below, for a server and a client block.
    (
        build $actual:expr => $rpc:literal,
        {
            $(
                $grpc_name:expr => $grpc_unit:literal: $grpc_type:tt { $($grpc_args:tt)* }
                $(=> {
                    $($grpc_key:expr => $grpc_val:expr,)+
                })?
                ,
            )*
        },
        {
            $(
                $client_name:expr => $client_unit:literal: $client_type:tt { $($client_args:tt)* }
                $(=> {
                    $($client_key:expr => $client_val:expr,)+
                })?
                ,
            )*
        },
    ) => {{
        let actual: &$crate::output::OutputMetrics<'_> = &$actual;
        vec![
            // A66 server metrics
            $(
            $crate::output::Metric {
                name: $grpc_name,
                unit: $grpc_unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("grpc.method", $rpc),
                    $($(otel_asserts::types::common::KeyValue::new($grpc_key, $grpc_val),)+)?
                ],
                data: $crate::expected_grpc_metrics!($grpc_type => $($grpc_args)*),
            },
            )*
            // A66 client metrics
            $(
            $crate::output::Metric {
                name: $client_name,
                unit: $client_unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("grpc.method", $rpc),
                    otel_asserts::types::common::KeyValue::new("grpc.target", actual.server_uri),
                    $($(otel_asserts::types::common::KeyValue::new($client_key, $client_val),)+)?
                ],
                data: $crate::expected_grpc_metrics!($client_type => $($client_args)*),
            },
            )*
        ]
    }};
    // Build a client-only expectation set instead of asserting it, so a test can
    // offer several accepted shapes to [`OutputMetrics::assert_one_of`] when the
    // non-determinism is in a metric's value rather than in whether a whole
    // block is emitted.
    (
        build $actual:expr => $rpc:literal,
        {
            $(
                $client_name:expr => $client_unit:literal: $client_type:tt { $($client_args:tt)* }
                $(=> {
                    $($client_key:expr => $client_val:expr,)+
                })?
                ,
            )*
        },
    ) => {{
        let actual: &$crate::output::OutputMetrics<'_> = &$actual;
        vec![
            $(
            $crate::output::Metric {
                name: $client_name,
                unit: $client_unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("grpc.method", $rpc),
                    otel_asserts::types::common::KeyValue::new("grpc.target", actual.server_uri),
                    $($(otel_asserts::types::common::KeyValue::new($client_key, $client_val),)+)?
                ],
                data: $crate::expected_grpc_metrics!($client_type => $($client_args)*),
            },
            )*
        ]
    }};
    (
        $actual:expr => $rpc:literal,
        {
            $(
                $grpc_name:expr => $grpc_unit:literal: $grpc_type:tt { $($grpc_args:tt)* }
                $(=> {
                    $($grpc_key:expr => $grpc_val:expr,)+
                })?
                ,
            )*
        },
        {
            $(
                $client_name:expr => $client_unit:literal: $client_type:tt { $($client_args:tt)* }
                $(=> {
                    $($client_key:expr => $client_val:expr,)+
                })?
                ,
            )*
        },
    ) => {{
        let actual: &$crate::output::OutputMetrics<'_> = &$actual;
        actual.assert(vec![
            // A66 server metrics
            $(
            $crate::output::Metric {
                name: $grpc_name,
                unit: $grpc_unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("grpc.method", $rpc),
                    $($(otel_asserts::types::common::KeyValue::new($grpc_key, $grpc_val),)+)?
                ],
                data: $crate::expected_grpc_metrics!($grpc_type => $($grpc_args)*),
            },
            )*
            // A66 client metrics
            $(
            $crate::output::Metric {
                name: $client_name,
                unit: $client_unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("grpc.method", $rpc),
                    otel_asserts::types::common::KeyValue::new("grpc.target", actual.server_uri),
                    $($(otel_asserts::types::common::KeyValue::new($client_key, $client_val),)+)?
                ],
                data: $crate::expected_grpc_metrics!($client_type => $($client_args)*),
            },
            )*
        ])
    }};
    (gauge => $value:expr) => {
        $crate::output::MetricData::Gauge { value: $value.into() }
    };
    (sum => $value:expr) => {
        $crate::output::MetricData::Sum { value: $value.into() }
    };
    (summary => $count:literal x $value:expr) => {
        $crate::output::MetricData::Summary { count: $count, value: $value }
    };
    (histogram => $count:literal x > 0.0) => {
        $crate::output::MetricData::Histogram { count: $count, sum: $crate::output::MetricValue::GreaterThanZero }
    };
    (histogram => $count:literal x $sum:expr) => {
        $crate::output::MetricData::Histogram { count: $count, sum: $crate::output::MetricValue::Some($sum) }
    };
    (exp_histogram => $count:literal x $sum:expr) => {
        $crate::output::MetricData::ExpHistogram { count: $count, sum: $crate::output::MetricValue::Some($sum) }
    };
}

#[macro_export]
macro_rules! expected_http_metrics {
    (
        $actual:expr => $http:ident, $method:literal => $path:literal,
        { $($name:expr => $unit:literal: $type:tt { $($args:tt)* } $(=> { $($key:expr => $val:expr),+ $(,)? })? ),+ $(,)? }
    ) => {{
        let actual: &$crate::output::OutputMetrics<'_> = &$actual;
        actual.assert(vec![
            $($crate::output::Metric {
                name: $name,
                unit: $unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("http.request.method", $method),
                    otel_asserts::types::common::KeyValue::new("http.route", $path),
                    otel_asserts::types::common::KeyValue::new("network.protocol.version", <$http>::otel()),
                    $($(otel_asserts::types::common::KeyValue::new($key, $val),)+)?
                ],
                data: $crate::expected_http_metrics!($type => $($args)*),
            },)+
        ])
    }};
    (
        $actual:expr => $http:ident, $method:literal => None,
        { $($name:expr => $unit:literal: $type:tt { $($args:tt)* } $(=> { $($key:expr => $val:expr),+ $(,)? })? ),+ $(,)? }
    ) => {{
        let actual: &$crate::output::OutputMetrics<'_> = &$actual;
        actual.assert(vec![
            $($crate::output::Metric {
                name: $name,
                unit: $unit,
                attrs: vec![
                    otel_asserts::types::common::KeyValue::new("http.request.method", $method),
                    otel_asserts::types::common::KeyValue::new("network.protocol.version", <$http>::otel()),
                    $($(otel_asserts::types::common::KeyValue::new($key, $val),)+)?
                ],
                data: $crate::expected_http_metrics!($type => $($args)*),
            },)+
        ])
    }};
    (gauge => $value:expr) => {
        $crate::output::MetricData::Gauge { value: $value.into() }
    };
    (sum => $value:expr) => {
        $crate::output::MetricData::Sum { value: $value.into() }
    };
    (summary => $count:literal x $value:expr) => {
        $crate::output::MetricData::Summary { count: $count, value: $value }
    };
    (histogram => $count:literal x > 0.0) => {
        $crate::output::MetricData::Histogram { count: $count, sum: $crate::output::MetricValue::GreaterThanZero }
    };
    (histogram => $count:literal x $sum:expr) => {
        $crate::output::MetricData::Histogram { count: $count, sum: $crate::output::MetricValue::Some($sum) }
    };
    (exp_histogram => $count:literal x $sum:expr) => {
        $crate::output::MetricData::ExpHistogram { count: $count, sum: $crate::output::MetricValue::Some($sum) }
    };
}
