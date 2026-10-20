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

use super::{common, opt_into, proto, resource, span_context, vec_into};
use opentelemetry::{SpanId, TraceId};
pub use proto::metrics::AggregationTemporality;

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct OutputMetrics {
    pub resource_metrics: Vec<ResourceMetrics>,
}

impl From<Vec<proto::metrics::ResourceMetrics>> for OutputMetrics {
    fn from(value: Vec<proto::metrics::ResourceMetrics>) -> Self {
        let resource_metrics = vec_into!(value);
        Self { resource_metrics }
    }
}

impl OutputMetrics {
    pub fn assert_timestamps(&mut self) {
        let timestamp = common::TimestampRange::now();
        for resource_metric in &mut self.resource_metrics {
            for scope_metric in &mut resource_metric.scope_metrics {
                for metric in &mut scope_metric.metrics {
                    if let Some(data) = metric.data.as_ref() {
                        match data {
                            Data::Gauge(gauge) => {
                                for (i, dp) in gauge.data_points.iter().enumerate() {
                                    assert_eq!(
                                        timestamp,
                                        dp.start_time_unix_nano,
                                        "metric data point {} start time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.start_time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                    assert_eq!(
                                        timestamp,
                                        dp.time_unix_nano,
                                        "metric data point {} time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                }
                            }
                            Data::Sum(sum) => {
                                for (i, dp) in sum.data_points.iter().enumerate() {
                                    assert_eq!(
                                        timestamp,
                                        dp.start_time_unix_nano,
                                        "metric data point {} start time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.start_time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                    assert_eq!(
                                        timestamp,
                                        dp.time_unix_nano,
                                        "metric data point {} time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                }
                            }
                            Data::Histogram(histogram) => {
                                for (i, dp) in histogram.data_points.iter().enumerate() {
                                    assert_eq!(
                                        timestamp,
                                        dp.start_time_unix_nano,
                                        "metric data point {} start time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.start_time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                    assert_eq!(
                                        timestamp,
                                        dp.time_unix_nano,
                                        "metric data point {} time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                }
                            }
                            Data::ExponentialHistogram(histogram) => {
                                for (i, dp) in histogram.data_points.iter().enumerate() {
                                    assert_eq!(
                                        timestamp,
                                        dp.start_time_unix_nano,
                                        "metric data point {} start time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.start_time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                    assert_eq!(
                                        timestamp,
                                        dp.time_unix_nano,
                                        "metric data point {} time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                }
                            }
                            Data::Summary(summary) => {
                                for (i, dp) in summary.data_points.iter().enumerate() {
                                    assert_eq!(
                                        timestamp,
                                        dp.start_time_unix_nano,
                                        "metric data point {} start time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.start_time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                    assert_eq!(
                                        timestamp,
                                        dp.time_unix_nano,
                                        "metric data point {} time {} < {} < {}: {:#?}",
                                        i,
                                        timestamp.start,
                                        dp.time_unix_nano,
                                        timestamp.end,
                                        metric
                                    );
                                }
                            }
                        }
                    }

                    if let Some(data) = metric.data.as_mut() {
                        match data {
                            Data::Gauge(gauge) => {
                                for dp in &mut gauge.data_points {
                                    dp.start_time_unix_nano = 0;
                                    dp.time_unix_nano = 0;
                                }
                            }
                            Data::Sum(sum) => {
                                for dp in &mut sum.data_points {
                                    dp.start_time_unix_nano = 0;
                                    dp.time_unix_nano = 0;
                                }
                            }
                            Data::Histogram(histogram) => {
                                for dp in &mut histogram.data_points {
                                    dp.start_time_unix_nano = 0;
                                    dp.time_unix_nano = 0;
                                }
                            }
                            Data::ExponentialHistogram(histogram) => {
                                for dp in &mut histogram.data_points {
                                    dp.start_time_unix_nano = 0;
                                    dp.time_unix_nano = 0;
                                }
                            }
                            Data::Summary(summary) => {
                                for dp in &mut summary.data_points {
                                    dp.start_time_unix_nano = 0;
                                    dp.time_unix_nano = 0;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceMetrics {
    pub resource: Option<resource::Resource>,
    pub schema_url: String,
    pub scope_metrics: Vec<ScopeMetrics>,
}

impl From<proto::metrics::ResourceMetrics> for ResourceMetrics {
    fn from(value: proto::metrics::ResourceMetrics) -> Self {
        let proto::metrics::ResourceMetrics {
            resource,
            scope_metrics,
            schema_url,
        } = value;
        let resource = opt_into!(resource);
        let scope_metrics = vec_into!(scope_metrics);
        Self {
            resource,
            schema_url,
            scope_metrics,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeMetrics {
    pub scope: Option<common::InstrumentationScope>,
    pub schema_url: String,
    pub metrics: Vec<Metric>,
}

impl From<proto::metrics::ScopeMetrics> for ScopeMetrics {
    fn from(value: proto::metrics::ScopeMetrics) -> Self {
        let proto::metrics::ScopeMetrics {
            scope,
            metrics,
            schema_url,
        } = value;
        let scope = opt_into!(scope);
        let metrics = vec_into!(metrics);
        Self {
            scope,
            schema_url,
            metrics,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: String,
    pub description: String,
    pub unit: String,
    pub metadata: Vec<common::KeyValue>,
    pub data: Option<Data>,
}

impl Metric {
    pub fn new(name: &str, desc: &str, unit: &str, data: Data) -> Self {
        Self {
            name: name.to_string(),
            description: desc.to_string(),
            unit: unit.to_string(),
            metadata: Vec::new(),
            data: Some(data),
        }
    }
}

impl From<proto::metrics::Metric> for Metric {
    fn from(value: proto::metrics::Metric) -> Self {
        let proto::metrics::Metric {
            name,
            description,
            unit,
            metadata,
            data,
        } = value;
        let metadata = vec_into!(metadata);
        let data = opt_into!(data);
        Self {
            name,
            description,
            unit,
            metadata,
            data,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    Gauge(Gauge),
    Sum(Sum),
    Histogram(Histogram),
    ExponentialHistogram(ExponentialHistogram),
    Summary(Summary),
}

impl From<proto::metrics::metric::Data> for Data {
    fn from(value: proto::metrics::metric::Data) -> Self {
        use proto::metrics::metric::Data;
        match value {
            Data::Gauge(v) => Self::Gauge(v.into()),
            Data::Sum(v) => Self::Sum(v.into()),
            Data::Histogram(v) => Self::Histogram(v.into()),
            Data::ExponentialHistogram(v) => Self::ExponentialHistogram(v.into()),
            Data::Summary(v) => Self::Summary(v.into()),
        }
    }
}

impl Data {
    pub fn gauge(dp: NumberDataPoint) -> Self {
        Self::Gauge(Gauge {
            data_points: vec![dp],
        })
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    pub data_points: Vec<NumberDataPoint>,
}

impl From<proto::metrics::Gauge> for Gauge {
    fn from(value: proto::metrics::Gauge) -> Self {
        let proto::metrics::Gauge { data_points } = value;
        let data_points = vec_into!(data_points);
        Self { data_points }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct NumberDataPoint {
    pub attributes: Vec<common::KeyValue>,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
    pub exemplars: Vec<Exemplar>,
    pub flags: u32,
    pub value: Option<NumberDataPointValue>,
}

impl NumberDataPoint {
    pub fn new(value: impl Into<NumberDataPointValue>, attributes: Vec<common::KeyValue>) -> Self {
        Self {
            attributes,
            start_time_unix_nano: 0,
            time_unix_nano: 0,
            exemplars: Vec::new(),
            flags: 0,
            value: Some(value.into()),
        }
    }
}

impl From<proto::metrics::NumberDataPoint> for NumberDataPoint {
    fn from(value: proto::metrics::NumberDataPoint) -> Self {
        let proto::metrics::NumberDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            exemplars,
            flags,
            value,
        } = value;
        let attributes = vec_into!(attributes);
        let exemplars = vec_into!(exemplars);
        let value = opt_into!(value);
        Self {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            exemplars,
            flags,
            value,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberDataPointValue {
    AsDouble(f64),
    AsInt(i64),
}

impl From<f64> for NumberDataPointValue {
    #[inline]
    fn from(v: f64) -> Self {
        Self::AsDouble(v)
    }
}

impl From<i64> for NumberDataPointValue {
    #[inline]
    fn from(v: i64) -> Self {
        Self::AsInt(v)
    }
}

impl From<proto::metrics::number_data_point::Value> for NumberDataPointValue {
    fn from(value: proto::metrics::number_data_point::Value) -> Self {
        use proto::metrics::number_data_point::Value;
        match value {
            Value::AsDouble(v) => Self::AsDouble(v),
            Value::AsInt(v) => Self::AsInt(v),
        }
    }
}

impl From<proto::metrics::exemplar::Value> for NumberDataPointValue {
    fn from(value: proto::metrics::exemplar::Value) -> Self {
        use proto::metrics::exemplar::Value;
        match value {
            Value::AsDouble(v) => Self::AsDouble(v),
            Value::AsInt(v) => Self::AsInt(v),
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Exemplar {
    pub filtered_attributes: Vec<common::KeyValue>,
    pub time_unix_nano: u64,
    #[cfg_attr(feature = "with-serde", serde(with = "crate::serde::span_id"))]
    pub span_id: SpanId,
    #[cfg_attr(feature = "with-serde", serde(with = "crate::serde::trace_id"))]
    pub trace_id: TraceId,
    pub value: Option<NumberDataPointValue>,
}

impl From<proto::metrics::Exemplar> for Exemplar {
    fn from(value: proto::metrics::Exemplar) -> Self {
        let proto::metrics::Exemplar {
            filtered_attributes,
            time_unix_nano,
            span_id,
            trace_id,
            value,
        } = value;
        let filtered_attributes = vec_into!(filtered_attributes);
        let span_id = span_context::make_span_id(&span_id);
        let trace_id = span_context::make_trace_id(&trace_id);
        let value = opt_into!(value);
        Self {
            filtered_attributes,
            time_unix_nano,
            span_id,
            trace_id,
            value,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Sum {
    pub data_points: Vec<NumberDataPoint>,
    #[cfg_attr(feature = "with-serde", serde(with = "crate::serde::agg_temp"))]
    pub aggregation_temporality: Result<AggregationTemporality, i32>,
    pub is_monotonic: bool,
}

pub(crate) fn agg_temp(aggregation_temporality: i32) -> Result<AggregationTemporality, i32> {
    aggregation_temporality
        .try_into()
        .map_err(|_e| aggregation_temporality)
}

impl From<proto::metrics::Sum> for Sum {
    fn from(value: proto::metrics::Sum) -> Self {
        let proto::metrics::Sum {
            data_points,
            aggregation_temporality,
            is_monotonic,
        } = value;
        let data_points = vec_into!(data_points);
        let aggregation_temporality = agg_temp(aggregation_temporality);
        Self {
            data_points,
            aggregation_temporality,
            is_monotonic,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub data_points: Vec<SummaryDataPoint>,
}

impl From<proto::metrics::Summary> for Summary {
    fn from(value: proto::metrics::Summary) -> Self {
        let proto::metrics::Summary { data_points } = value;
        let data_points = vec_into!(data_points);
        Self { data_points }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryDataPoint {
    pub attributes: Vec<common::KeyValue>,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
    pub count: u64,
    pub sum: f64,
    pub quantile_values: Vec<ValueAtQuantile>,
    pub flags: u32,
}

impl From<proto::metrics::SummaryDataPoint> for SummaryDataPoint {
    fn from(value: proto::metrics::SummaryDataPoint) -> Self {
        let proto::metrics::SummaryDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            quantile_values,
            flags,
        } = value;
        let attributes = vec_into!(attributes);
        let quantile_values = vec_into!(quantile_values);
        Self {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            quantile_values,
            flags,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueAtQuantile {
    pub quantile: f64,
    pub value: f64,
}

impl From<proto::metrics::summary_data_point::ValueAtQuantile> for ValueAtQuantile {
    fn from(value: proto::metrics::summary_data_point::ValueAtQuantile) -> Self {
        let proto::metrics::summary_data_point::ValueAtQuantile { quantile, value } = value;
        Self { quantile, value }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub data_points: Vec<HistogramDataPoint>,
    #[cfg_attr(feature = "with-serde", serde(with = "crate::serde::agg_temp"))]
    pub aggregation_temporality: Result<AggregationTemporality, i32>,
}

impl From<proto::metrics::Histogram> for Histogram {
    fn from(value: proto::metrics::Histogram) -> Self {
        let proto::metrics::Histogram {
            data_points,
            aggregation_temporality,
        } = value;
        let data_points = vec_into!(data_points);
        let aggregation_temporality = agg_temp(aggregation_temporality);
        Self {
            data_points,
            aggregation_temporality,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramDataPoint {
    pub attributes: Vec<common::KeyValue>,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
    pub count: u64,
    pub sum: Option<f64>,
    pub bucket_counts: Vec<u64>,
    pub explicit_bounds: Vec<f64>,
    pub exemplars: Vec<Exemplar>,
    pub flags: u32,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl From<proto::metrics::HistogramDataPoint> for HistogramDataPoint {
    fn from(value: proto::metrics::HistogramDataPoint) -> Self {
        let proto::metrics::HistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            bucket_counts,
            explicit_bounds,
            exemplars,
            flags,
            min,
            max,
        } = value;
        let attributes = vec_into!(attributes);
        let exemplars = vec_into!(exemplars);
        Self {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            bucket_counts,
            explicit_bounds,
            exemplars,
            flags,
            min,
            max,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ExponentialHistogram {
    pub data_points: Vec<ExponentialHistogramDataPoint>,
    #[cfg_attr(feature = "with-serde", serde(with = "crate::serde::agg_temp"))]
    pub aggregation_temporality: Result<AggregationTemporality, i32>,
}

impl From<proto::metrics::ExponentialHistogram> for ExponentialHistogram {
    fn from(value: proto::metrics::ExponentialHistogram) -> Self {
        let proto::metrics::ExponentialHistogram {
            data_points,
            aggregation_temporality,
        } = value;
        let data_points = vec_into!(data_points);
        let aggregation_temporality = agg_temp(aggregation_temporality);
        Self {
            data_points,
            aggregation_temporality,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ExponentialHistogramDataPoint {
    pub attributes: Vec<common::KeyValue>,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
    pub count: u64,
    pub sum: Option<f64>,
    pub scale: i32,
    pub zero_count: u64,
    pub positive: Option<Buckets>,
    pub negative: Option<Buckets>,
    pub flags: u32,
    pub exemplars: Vec<Exemplar>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub zero_threshold: f64,
}

impl From<proto::metrics::ExponentialHistogramDataPoint> for ExponentialHistogramDataPoint {
    fn from(value: proto::metrics::ExponentialHistogramDataPoint) -> Self {
        let proto::metrics::ExponentialHistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            scale,
            zero_count,
            positive,
            negative,
            flags,
            exemplars,
            min,
            max,
            zero_threshold,
        } = value;
        let attributes = vec_into!(attributes);
        let positive = opt_into!(positive);
        let negative = opt_into!(negative);
        let exemplars = vec_into!(exemplars);
        Self {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count,
            sum,
            scale,
            zero_count,
            positive,
            negative,
            flags,
            exemplars,
            min,
            max,
            zero_threshold,
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Buckets {
    pub offset: i32,
    pub bucket_counts: Vec<u64>,
}

impl From<proto::metrics::exponential_histogram_data_point::Buckets> for Buckets {
    fn from(value: proto::metrics::exponential_histogram_data_point::Buckets) -> Self {
        let proto::metrics::exponential_histogram_data_point::Buckets {
            offset,
            bucket_counts,
        } = value;
        Self {
            offset,
            bucket_counts,
        }
    }
}
