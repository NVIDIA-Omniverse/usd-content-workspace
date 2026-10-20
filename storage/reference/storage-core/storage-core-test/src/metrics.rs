// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use tracing::info;

/// Key for compacted metrics: (metric_name, sorted_labels as Vec<(key, value)>)
type CompactedMetricKey = (String, Vec<(String, String)>);

/// Entry for displaying metrics: (name, labels, value)
type MetricDisplayEntry = (String, Vec<(String, String)>, u64);

/// Compacted metric values by type
#[derive(Default)]
struct CompactedMetrics {
    /// Sum metrics: key -> max value
    sum: HashMap<CompactedMetricKey, u64>,
    /// Gauge metrics: key -> max value
    gauge: HashMap<CompactedMetricKey, u64>,
    /// Histogram metrics: key -> max count
    histogram: HashMap<CompactedMetricKey, u64>,
}

pub struct InMemoryMetrics {
    pub meter_provider: SdkMeterProvider,
    pub exporter: InMemoryMetricExporter,
    /// Compacted metrics cache - updated on flush, read by get_metric_value
    compacted: RwLock<CompactedMetrics>,
    /// Lock to ensure atomic read+reset of the exporter (prevents race conditions)
    exporter_lock: Mutex<()>,
}

impl InMemoryMetrics {
    pub fn new() -> Self {
        // Override the default exporter to use the InMemoryMetricExporter
        let exporter = InMemoryMetricExporter::default();
        let meter_provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        info!("Meter provider is set: {meter_provider:?}");
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        Self {
            meter_provider,
            exporter,
            compacted: RwLock::new(CompactedMetrics::default()),
            exporter_lock: Mutex::new(()),
        }
    }

    pub fn get_finished_metrics(&self) -> Vec<ResourceMetrics> {
        self.exporter.get_finished_metrics().unwrap()
    }

    pub fn flush(&self) {
        self.meter_provider.force_flush().unwrap();
        self.compact_metrics();
    }

    /// Compact all finished metrics into the cache and reset the exporter.
    /// This prevents unbounded memory growth during long test runs.
    fn compact_metrics(&self) {
        // Hold the exporter lock while reading AND resetting to ensure atomicity.
        // This prevents race conditions where metrics exported between get and reset would be lost.
        let metrics = {
            let _lock = self.exporter_lock.lock().unwrap();
            let metrics = self.exporter.get_finished_metrics().unwrap();
            self.exporter.reset();
            metrics
        };

        // Now compact the cloned metrics into our cache (lock released, so other threads can proceed)
        let mut compacted = self.compacted.write().unwrap();

        for metric in metrics {
            for scope_metric in metric.scope_metrics() {
                for m in scope_metric.metrics() {
                    let name = m.name();
                    match m.data() {
                        AggregatedMetrics::U64(metric_data) => {
                            Self::compact_u64_metric(&mut compacted, name, metric_data);
                        },
                        AggregatedMetrics::F64(metric_data) => {
                            Self::compact_f64_metric(&mut compacted, name, metric_data);
                        },
                        AggregatedMetrics::I64(metric_data) => {
                            Self::compact_i64_metric(&mut compacted, name, metric_data);
                        },
                    }
                }
            }
        }
    }

    fn make_compact_key(name: &str, attrs: &[opentelemetry::KeyValue]) -> CompactedMetricKey {
        let mut labels: Vec<(String, String)> = attrs
            .iter()
            .map(|kv| (kv.key.as_str().to_string(), format!("{}", kv.value)))
            .collect();
        labels.sort();
        (name.to_string(), labels)
    }

    fn compact_u64_metric(compacted: &mut CompactedMetrics, name: &str, metric_data: &MetricData<u64>) {
        match metric_data {
            MetricData::Gauge(gauge) => {
                for dp in gauge.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    compacted
                        .gauge
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(dp.value()))
                        .or_insert(dp.value());
                }
            },
            MetricData::Sum(sum) => {
                for dp in sum.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    compacted
                        .sum
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(dp.value()))
                        .or_insert(dp.value());
                }
            },
            MetricData::Histogram(histogram) => {
                for dp in histogram.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    compacted
                        .histogram
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(dp.count()))
                        .or_insert(dp.count());
                }
            },
            _ => {},
        }
    }

    fn compact_f64_metric(compacted: &mut CompactedMetrics, name: &str, metric_data: &MetricData<f64>) {
        match metric_data {
            MetricData::Gauge(gauge) => {
                for dp in gauge.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    let value = dp.value() as u64;
                    compacted
                        .gauge
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(value))
                        .or_insert(value);
                }
            },
            MetricData::Sum(sum) => {
                for dp in sum.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    let value = dp.value() as u64;
                    compacted
                        .sum
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(value))
                        .or_insert(value);
                }
            },
            MetricData::Histogram(histogram) => {
                for dp in histogram.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    compacted
                        .histogram
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(dp.count()))
                        .or_insert(dp.count());
                }
            },
            _ => {},
        }
    }

    fn compact_i64_metric(compacted: &mut CompactedMetrics, name: &str, metric_data: &MetricData<i64>) {
        match metric_data {
            MetricData::Gauge(gauge) => {
                for dp in gauge.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    let value = dp.value() as u64;
                    compacted
                        .gauge
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(value))
                        .or_insert(value);
                }
            },
            MetricData::Sum(sum) => {
                for dp in sum.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    let value = dp.value() as u64;
                    compacted
                        .sum
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(value))
                        .or_insert(value);
                }
            },
            MetricData::Histogram(histogram) => {
                for dp in histogram.data_points() {
                    let attrs: Vec<_> = dp.attributes().cloned().collect();
                    let key = Self::make_compact_key(name, &attrs);
                    compacted
                        .histogram
                        .entry(key)
                        .and_modify(|v| *v = (*v).max(dp.count()))
                        .or_insert(dp.count());
                }
            },
            _ => {},
        }
    }

    pub fn get_metric_value(&self, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        // Flush to ensure the compacted cache is up-to-date
        self.flush();

        let compacted = self.compacted.read().unwrap();

        // Helper to check if metric labels contain all requested labels (partial match)
        let labels_match = |metric_labels: &[(String, String)]| -> bool {
            for (key, expected) in labels {
                match metric_labels.iter().find(|(k, _)| k == *key) {
                    Some((_, v)) if v == *expected => continue,
                    _ => return false,
                }
            }
            true
        };

        // Priority: Sum > Gauge > Histogram (matching original behavior)
        // Find matching entries and return the maximum value
        let sum_max = compacted
            .sum
            .iter()
            .filter(|((metric_name, metric_labels), _)| metric_name == name && labels_match(metric_labels))
            .map(|(_, &v)| v)
            .max();
        if let Some(value) = sum_max {
            return Some(value);
        }

        let gauge_max = compacted
            .gauge
            .iter()
            .filter(|((metric_name, metric_labels), _)| metric_name == name && labels_match(metric_labels))
            .map(|(_, &v)| v)
            .max();
        if let Some(value) = gauge_max {
            return Some(value);
        }

        let histogram_max = compacted
            .histogram
            .iter()
            .filter(|((metric_name, metric_labels), _)| metric_name == name && labels_match(metric_labels))
            .map(|(_, &v)| v)
            .max();
        if let Some(value) = histogram_max {
            return Some(value);
        }

        None
    }

    pub fn print_metrics(&self) {
        self.flush();

        // Read from the compacted cache
        let compacted = self.compacted.read().unwrap();

        // Collect entries for sorting
        let mut entries: Vec<MetricDisplayEntry> = Vec::new();

        for ((name, labels), &value) in &compacted.sum {
            entries.push((name.clone(), labels.clone(), value));
        }

        // Sort by api_version, then method, then metric name
        entries.sort_by(|a, b| {
            let a_api_version =
                a.1.iter()
                    .find(|(k, _)| k == "api_version")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let b_api_version =
                b.1.iter()
                    .find(|(k, _)| k == "api_version")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let api_cmp = a_api_version.cmp(b_api_version);
            if api_cmp != std::cmp::Ordering::Equal {
                return api_cmp;
            }

            let a_method =
                a.1.iter()
                    .find(|(k, _)| k == "method")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let b_method =
                b.1.iter()
                    .find(|(k, _)| k == "method")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let method_cmp = a_method.cmp(b_method);
            if method_cmp != std::cmp::Ordering::Equal {
                return method_cmp;
            }

            a.0.cmp(&b.0)
        });

        // Print sorted metrics
        for (name, labels, value) in entries {
            let attrs_str = labels
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::error!("Metric: {} [{}] = {}", name, attrs_str, value);
        }
    }

    /// Print metrics filtered by the provided labels
    /// Returns a formatted string of matching metrics
    pub fn print_filtered_metrics(&self, name: &str, filter_labels: &[(&str, &str)]) -> String {
        self.flush();

        // Read from the compacted cache
        let compacted = self.compacted.read().unwrap();

        // Helper to check if labels match filter
        let labels_match = |labels: &[(String, String)]| -> bool {
            for (key, expected) in filter_labels {
                match labels.iter().find(|(k, _)| k == *key) {
                    Some((_, v)) if v == *expected => continue,
                    _ => return false,
                }
            }
            true
        };

        // Collect matching entries
        let mut entries: Vec<MetricDisplayEntry> = Vec::new();

        // Check sum metrics
        for ((metric_name, labels), &value) in &compacted.sum {
            if metric_name == name && labels_match(labels) {
                entries.push((metric_name.clone(), labels.clone(), value));
            }
        }

        // Check gauge metrics
        for ((metric_name, labels), &value) in &compacted.gauge {
            if metric_name == name && labels_match(labels) {
                entries.push((metric_name.clone(), labels.clone(), value));
            }
        }

        // Check histogram metrics
        for ((metric_name, labels), &value) in &compacted.histogram {
            if metric_name == name && labels_match(labels) {
                entries.push((metric_name.clone(), labels.clone(), value));
            }
        }

        // Sort by api_version, then method, then metric name, then result label
        entries.sort_by(|a, b| {
            let a_api_version =
                a.1.iter()
                    .find(|(k, _)| k == "api_version")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let b_api_version =
                b.1.iter()
                    .find(|(k, _)| k == "api_version")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let api_cmp = a_api_version.cmp(b_api_version);
            if api_cmp != std::cmp::Ordering::Equal {
                return api_cmp;
            }

            let a_method =
                a.1.iter()
                    .find(|(k, _)| k == "method")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let b_method =
                b.1.iter()
                    .find(|(k, _)| k == "method")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let method_cmp = a_method.cmp(b_method);
            if method_cmp != std::cmp::Ordering::Equal {
                return method_cmp;
            }

            let name_cmp = a.0.cmp(&b.0);
            if name_cmp != std::cmp::Ordering::Equal {
                return name_cmp;
            }

            let a_result =
                a.1.iter()
                    .find(|(k, _)| k == "result")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            let b_result =
                b.1.iter()
                    .find(|(k, _)| k == "result")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
            a_result.cmp(b_result)
        });

        // Format output
        if entries.is_empty() {
            format!("No metrics found matching labels: {:?}", filter_labels)
        } else {
            entries
                .iter()
                .map(|(metric_name, labels, value)| {
                    let attrs_str = labels
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Metric: {} [{}] = {}", metric_name, attrs_str, value)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

impl Default for InMemoryMetrics {
    fn default() -> Self {
        Self::new()
    }
}
