// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use futures::Stream;
use std::pin::Pin;

use crate::traits::{BoxStream, Result};
use storage_core_util::observability::metrics::{
    self, BoundTimerMetric, BoundUIntCounter, BoundUIntHistogram, HistogramCommonBucket, InitializedHistogramU64,
    MetricBinder, TimerMetric, TimerMetricGuard, UIntCounter,
};

metrics::metric_base!(
    static RESULTS_COUNTER_BASE: UIntCounter,
    name = "requests",
    description = "Total number of storage API requests",
    { method, api_version, storage_backend, result, pod_name, pkg_version }
);
metrics::metric_base!(
    static LATENCY_BASE: TimerMetric,
    name = "request.duration",
    { method, api_version, storage_backend, pod_name, pkg_version }
);

metrics::metric_base!(
    static SDK_RESULTS_COUNTER_BASE: UIntCounter,
    name = "sdk.requests",
    description = "Total number of requests to the storage backend SDK",
    { method, api_version, storage_backend, result, pod_name, pkg_version }
);
metrics::metric_base!(
    static SDK_LATENCY_BASE: TimerMetric,
    name = "sdk.request.duration",
    { method, api_version, storage_backend, pod_name, pkg_version }
);

// Public getter functions to access the metric bases
pub fn results_counter_base() -> &'static UIntCounter {
    &RESULTS_COUNTER_BASE
}

pub fn latency_base() -> &'static TimerMetric {
    &LATENCY_BASE
}

pub fn sdk_results_counter_base() -> &'static UIntCounter {
    &SDK_RESULTS_COUNTER_BASE
}

pub fn sdk_latency_base() -> &'static TimerMetric {
    &SDK_LATENCY_BASE
}

metrics::metric_base!(
    static ITEM_COUNT_BASE: InitializedHistogramU64,
    name = "enumeration.items",
    description = "Number of items returned per enumeration operation",
    buckets = HistogramCommonBucket::CallCount.boundaries(),
    { method, api_version, storage_backend, pod_name, pkg_version }
);

pub fn item_count_base() -> &'static InitializedHistogramU64 {
    &ITEM_COUNT_BASE
}

metrics::metric_base!(
    static REDIRECTS_BASE: UIntCounter,
    name = "read.redirects",
    description = "Number of redirect URLs returned for read operations",
    { method, api_version, storage_backend, pod_name, pkg_version }
);

pub fn redirects_base() -> &'static UIntCounter {
    &REDIRECTS_BASE
}

metrics::metric_base!(
    static CHUNK_SIZE_HIST_BASE: InitializedHistogramU64,
    name = "read.chunk.size",
    description = "Size of data chunks received from backend in bytes",
    buckets = HistogramCommonBucket::Data.boundaries(),
    { method, api_version, storage_backend, pod_name, pkg_version }
);

pub fn chunk_size_hist_base() -> &'static InitializedHistogramU64 {
    &CHUNK_SIZE_HIST_BASE
}

metrics::metric_base!(
    static DATA_SIZE_HIST_BASE: InitializedHistogramU64,
    name = "read.object.size",
    description = "Total size of read objects in bytes",
    buckets = HistogramCommonBucket::Data.boundaries(),
    { method, api_version, storage_backend, pod_name, pkg_version }
);

pub fn data_size_hist_base() -> &'static InitializedHistogramU64 {
    &DATA_SIZE_HIST_BASE
}

metrics::metric_base!(
    static UPLOAD_METHOD_BASE: UIntCounter,
    name = "write.operations",
    description = "Number of write operations by upload method",
        { method, api_version, storage_backend, upload_preference, pod_name, pkg_version }
);

pub fn upload_method_base() -> &'static UIntCounter {
    &UPLOAD_METHOD_BASE
}

pub struct MethodMetrics {
    pub results_ok: BoundUIntCounter,
    pub results_cancelled: BoundUIntCounter,
    pub results_unknown: BoundUIntCounter,
    pub results_invalid_argument: BoundUIntCounter,
    pub results_deadline_exceeded: BoundUIntCounter,
    pub results_not_found: BoundUIntCounter,
    pub results_already_exists: BoundUIntCounter,
    pub results_permission_denied: BoundUIntCounter,
    pub results_resource_exhausted: BoundUIntCounter,
    pub results_failed_precondition: BoundUIntCounter,
    pub results_aborted: BoundUIntCounter,
    pub results_out_of_range: BoundUIntCounter,
    pub results_unimplemented: BoundUIntCounter,
    pub results_internal: BoundUIntCounter,
    pub results_unavailable: BoundUIntCounter,
    pub results_data_loss: BoundUIntCounter,
    pub results_unauthenticated: BoundUIntCounter,
    pub latency: BoundTimerMetric,
}

impl MethodMetrics {
    pub fn new(
        base: &UIntCounter,
        latency_base: &TimerMetric,
        method: impl Into<String>,
        api_version: impl Into<String>,
        storage_backend: impl Into<String>,
    ) -> Self {
        let method = method.into();
        let api_version = api_version.into();
        let storage_backend = storage_backend.into();
        let pod_name = std::env::var("STORAGE_POD_NAME").unwrap_or("pod".to_string());
        let pkg_version = env!("CARGO_PKG_VERSION").to_string();
        Self {
            results_ok: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Ok".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_cancelled: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Cancelled".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_unknown: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Unknown".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_invalid_argument: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "InvalidArgument".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_deadline_exceeded: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "DeadlineExceeded".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_not_found: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "NotFound".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_already_exists: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "AlreadyExists".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_permission_denied: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "PermissionDenied".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_resource_exhausted: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "ResourceExhausted".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_failed_precondition: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "FailedPrecondition".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_aborted: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Aborted".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_out_of_range: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "OutOfRange".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_unimplemented: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Unimplemented".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_internal: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Internal".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_unavailable: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Unavailable".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_data_loss: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "DataLoss".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            results_unauthenticated: base.bind([
                ("method", method.clone()),
                ("api_version", api_version.clone()),
                ("storage_backend", storage_backend.clone()),
                ("result", "Unauthenticated".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            latency: latency_base.bind([
                ("method", method),
                ("api_version", api_version),
                ("storage_backend", storage_backend),
                ("pod_name", pod_name),
                ("pkg_version", pkg_version),
            ]),
        }
    }
}

#[macro_export]
macro_rules! increment_error {
    ($requests:expr, $e:expr) => {
        match $e.tonic_status().code() {
            tonic::Code::Ok => $requests.results_ok.add(1),
            tonic::Code::Cancelled => $requests.results_cancelled.add(1),
            tonic::Code::Unknown => $requests.results_unknown.add(1),
            tonic::Code::InvalidArgument => $requests.results_invalid_argument.add(1),
            tonic::Code::DeadlineExceeded => $requests.results_deadline_exceeded.add(1),
            tonic::Code::NotFound => $requests.results_not_found.add(1),
            tonic::Code::AlreadyExists => $requests.results_already_exists.add(1),
            tonic::Code::PermissionDenied => $requests.results_permission_denied.add(1),
            tonic::Code::ResourceExhausted => $requests.results_resource_exhausted.add(1),
            tonic::Code::FailedPrecondition => $requests.results_failed_precondition.add(1),
            tonic::Code::Aborted => $requests.results_aborted.add(1),
            tonic::Code::OutOfRange => $requests.results_out_of_range.add(1),
            tonic::Code::Unimplemented => $requests.results_unimplemented.add(1),
            tonic::Code::Internal => $requests.results_internal.add(1),
            tonic::Code::Unavailable => $requests.results_unavailable.add(1),
            tonic::Code::DataLoss => $requests.results_data_loss.add(1),
            tonic::Code::Unauthenticated => $requests.results_unauthenticated.add(1),
        }
    };
}

#[macro_export]
macro_rules! increment_ok {
    ($requests:expr) => {
        $requests.results_ok.add(1);
    };
}

#[doc(hidden)]
#[macro_export(local_inner_macros)]
macro_rules! record_operation {
    ($requests:expr, $operation:expr) => {
        match $operation {
            Ok(response) => {
                increment_ok!($requests);
                Ok(response)
            },
            Err(e) => {
                increment_error!($requests, e);
                Err(e)
            },
        }
    };
}

// Custom stream wrapper that holds a timer and drops it when the stream ends
pub struct TimedStream<S> {
    stream: BoxStream<S>,
    _timer: TimerMetricGuard,
    requests: std::sync::Arc<MethodMetrics>,
    has_errors: std::cell::RefCell<bool>,
}

impl<S> TimedStream<S> {
    pub fn new(stream: BoxStream<S>, timer: TimerMetricGuard, requests: std::sync::Arc<MethodMetrics>) -> Self {
        Self {
            stream,
            _timer: timer,
            requests,
            has_errors: std::cell::RefCell::new(false),
        }
    }
}

impl<S> Drop for TimedStream<S> {
    fn drop(&mut self) {
        if *self.has_errors.borrow() {
            // If there were errors, we don't increment anything here since errors were already incremented
            // when they occurred during streaming
        } else {
            // If no errors occurred, increment the OK counter
            increment_ok!(self.requests);
        }
    }
}

impl<S> futures::Stream for TimedStream<S> {
    type Item = storage_core_builder::traits::Result<S>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match std::pin::Pin::new(&mut self.stream).poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(item))) => std::task::Poll::Ready(Some(Ok(item))),
            std::task::Poll::Ready(Some(Err(e))) => {
                // Mark that we've seen an error
                *self.has_errors.borrow_mut() = true;
                // Increment the error counter
                increment_error!(self.requests, e);
                std::task::Poll::Ready(Some(Err(e)))
            },
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// Trait to get the number of entries in a response
pub trait EntryCount {
    fn entry_count(&self) -> u64;
}

/// Stream wrapper that counts the number of entries in each response
pub struct EntryCountStream<T: EntryCount> {
    inner: Pin<Box<TimedStream<T>>>,
    item_count: std::sync::Arc<BoundUIntHistogram>,
    count: std::cell::RefCell<u64>,
}

impl<T: EntryCount> EntryCountStream<T> {
    pub fn new(timed_stream: TimedStream<T>, item_count: std::sync::Arc<BoundUIntHistogram>) -> Self {
        Self {
            inner: Box::pin(timed_stream),
            item_count,
            count: std::cell::RefCell::new(0),
        }
    }
}

impl<T: EntryCount> Drop for EntryCountStream<T> {
    fn drop(&mut self) {
        let count = *self.count.borrow();
        self.item_count.record(count);
    }
}

impl<T: EntryCount> Stream for EntryCountStream<T> {
    type Item = Result<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(response))) => {
                *self.count.borrow_mut() += response.entry_count();
                std::task::Poll::Ready(Some(Ok(response)))
            },
            other => other,
        }
    }
}
