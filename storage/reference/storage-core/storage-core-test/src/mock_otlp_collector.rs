// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Mock OTLP collector for observability integration tests.
//!
//! Implements the three OTLP gRPC collector services (traces, metrics, logs) as an
//! in-process tonic server. Received telemetry is stored in memory and queryable
//! for test assertions.
//!
//! # Example
//!
//! ```ignore
//! let (port, telemetry, _handle) = start_mock_collector().await;
//! // ... configure OTEL to export to http://127.0.0.1:{port} ...
//! // ... run some instrumented code ...
//! let spans = telemetry.wait_for_spans(1, Duration::from_secs(5)).await.unwrap();
//! assert_eq!(spans[0].name, "my_operation");
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry_proto::tonic::collector::{
    logs::v1::{
        logs_service_server::{LogsService, LogsServiceServer},
        ExportLogsServiceRequest, ExportLogsServiceResponse,
    },
    metrics::v1::{
        metrics_service_server::{MetricsService, MetricsServiceServer},
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
    },
    trace::v1::{
        trace_service_server::{TraceService, TraceServiceServer},
        ExportTraceServiceRequest, ExportTraceServiceResponse,
    },
};
use opentelemetry_proto::tonic::{common::v1::KeyValue, logs::v1::LogRecord, metrics::v1::Metric, trace::v1::Span};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;

/// Thread-safe storage for telemetry received by the mock collector.
#[derive(Debug, Clone)]
pub struct CollectedTelemetry {
    inner: Arc<CollectedTelemetryInner>,
}

#[derive(Debug, Default)]
struct CollectedTelemetryInner {
    traces: Mutex<Vec<ExportTraceServiceRequest>>,
    metrics: Mutex<Vec<ExportMetricsServiceRequest>>,
    logs: Mutex<Vec<ExportLogsServiceRequest>>,
    trace_notify: Notify,
    metric_notify: Notify,
    log_notify: Notify,
}

impl Default for CollectedTelemetry {
    fn default() -> Self {
        Self {
            inner: Arc::new(CollectedTelemetryInner::default()),
        }
    }
}

impl CollectedTelemetry {
    /// Return all spans received so far.
    pub fn spans(&self) -> Vec<Span> {
        let requests = self.inner.traces.lock().unwrap();
        requests
            .iter()
            .flat_map(|r| &r.resource_spans)
            .flat_map(|rs| &rs.scope_spans)
            .flat_map(|ss| &ss.spans)
            .cloned()
            .collect()
    }

    /// Return all metrics received so far.
    pub fn metrics(&self) -> Vec<Metric> {
        let requests = self.inner.metrics.lock().unwrap();
        requests
            .iter()
            .flat_map(|r| &r.resource_metrics)
            .flat_map(|rm| &rm.scope_metrics)
            .flat_map(|sm| &sm.metrics)
            .cloned()
            .collect()
    }

    /// Return all log records received so far.
    pub fn log_records(&self) -> Vec<LogRecord> {
        let requests = self.inner.logs.lock().unwrap();
        requests
            .iter()
            .flat_map(|r| &r.resource_logs)
            .flat_map(|rl| &rl.scope_logs)
            .flat_map(|sl| &sl.log_records)
            .cloned()
            .collect()
    }

    /// Return resource attributes from the first trace export (if any).
    pub fn trace_resource_attributes(&self) -> Vec<KeyValue> {
        let requests = self.inner.traces.lock().unwrap();
        requests
            .first()
            .and_then(|r| r.resource_spans.first())
            .and_then(|rs| rs.resource.as_ref())
            .map(|r| r.attributes.clone())
            .unwrap_or_default()
    }

    /// Return resource attributes from the first log export (if any).
    pub fn log_resource_attributes(&self) -> Vec<KeyValue> {
        let requests = self.inner.logs.lock().unwrap();
        requests
            .first()
            .and_then(|r| r.resource_logs.first())
            .and_then(|rl| rl.resource.as_ref())
            .map(|r| r.attributes.clone())
            .unwrap_or_default()
    }

    /// Return resource attributes from the first metrics export (if any).
    pub fn metric_resource_attributes(&self) -> Vec<KeyValue> {
        let requests = self.inner.metrics.lock().unwrap();
        requests
            .first()
            .and_then(|r| r.resource_metrics.first())
            .and_then(|rm| rm.resource.as_ref())
            .map(|r| r.attributes.clone())
            .unwrap_or_default()
    }

    /// Find spans matching a predicate.
    pub fn find_spans(&self, predicate: impl Fn(&Span) -> bool) -> Vec<Span> {
        self.spans().into_iter().filter(predicate).collect()
    }

    /// Find metrics by name.
    pub fn find_metrics(&self, name: &str) -> Vec<Metric> {
        self.metrics().into_iter().filter(|m| m.name == name).collect()
    }

    /// Find log records matching a predicate.
    pub fn find_logs(&self, predicate: impl Fn(&LogRecord) -> bool) -> Vec<LogRecord> {
        self.log_records().into_iter().filter(predicate).collect()
    }

    /// Wait until at least `min_count` spans have been received, or timeout.
    pub async fn wait_for_spans(&self, min_count: usize, timeout: Duration) -> Result<Vec<Span>, String> {
        self.wait_for(min_count, timeout, || self.spans(), &self.inner.trace_notify)
            .await
    }

    /// Wait until at least `min_count` metrics with the given name have been received.
    pub async fn wait_for_metrics_named(
        &self,
        name: &str,
        min_count: usize,
        timeout: Duration,
    ) -> Result<Vec<Metric>, String> {
        let name = name.to_string();
        self.wait_for(
            min_count,
            timeout,
            || self.find_metrics(&name),
            &self.inner.metric_notify,
        )
        .await
    }

    /// Wait until at least `min_count` log records have been received.
    pub async fn wait_for_logs(&self, min_count: usize, timeout: Duration) -> Result<Vec<LogRecord>, String> {
        self.wait_for(min_count, timeout, || self.log_records(), &self.inner.log_notify)
            .await
    }

    /// Wait until at least `min_count` metrics (any name) have been received.
    pub async fn wait_for_any_metrics(&self, min_count: usize, timeout: Duration) -> Result<Vec<Metric>, String> {
        self.wait_for(min_count, timeout, || self.metrics(), &self.inner.metric_notify)
            .await
    }

    async fn wait_for<T: Clone>(
        &self,
        min_count: usize,
        timeout: Duration,
        snapshot: impl Fn() -> Vec<T>,
        notify: &Notify,
    ) -> Result<Vec<T>, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let items = snapshot();
            if items.len() >= min_count {
                return Ok(items);
            }
            let remaining = deadline.duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("timed out waiting for {min_count} items, got {}", items.len()));
            }
            tokio::select! {
                _ = notify.notified() => {},
                _ = tokio::time::sleep(remaining) => {},
            }
        }
    }
}

// ---------------------------------------------------------------------------
// gRPC service implementations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MockCollector {
    telemetry: CollectedTelemetry,
}

#[tonic::async_trait]
impl TraceService for MockCollector {
    async fn export(
        &self,
        request: tonic::Request<ExportTraceServiceRequest>,
    ) -> Result<tonic::Response<ExportTraceServiceResponse>, tonic::Status> {
        self.telemetry.inner.traces.lock().unwrap().push(request.into_inner());
        self.telemetry.inner.trace_notify.notify_waiters();
        Ok(tonic::Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl MetricsService for MockCollector {
    async fn export(
        &self,
        request: tonic::Request<ExportMetricsServiceRequest>,
    ) -> Result<tonic::Response<ExportMetricsServiceResponse>, tonic::Status> {
        self.telemetry.inner.metrics.lock().unwrap().push(request.into_inner());
        self.telemetry.inner.metric_notify.notify_waiters();
        Ok(tonic::Response::new(ExportMetricsServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl LogsService for MockCollector {
    async fn export(
        &self,
        request: tonic::Request<ExportLogsServiceRequest>,
    ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
        self.telemetry.inner.logs.lock().unwrap().push(request.into_inner());
        self.telemetry.inner.log_notify.notify_waiters();
        Ok(tonic::Response::new(ExportLogsServiceResponse {
            partial_success: None,
        }))
    }
}

// ---------------------------------------------------------------------------
// Server startup
// ---------------------------------------------------------------------------

/// Start a mock OTLP collector on a random port.
///
/// Returns `(port, collected_telemetry, server_handle)`. The server runs until
/// the handle is aborted or the process exits.
pub async fn start_mock_collector() -> (u16, CollectedTelemetry, JoinHandle<()>) {
    let telemetry = CollectedTelemetry::default();
    let collector = MockCollector {
        telemetry: telemetry.clone(),
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock collector");
    let port = listener.local_addr().unwrap().port();

    let trace_svc =
        TraceServiceServer::new(collector.clone()).accept_compressed(tonic::codec::CompressionEncoding::Gzip);
    let metrics_svc =
        MetricsServiceServer::new(collector.clone()).accept_compressed(tonic::codec::CompressionEncoding::Gzip);
    let logs_svc = LogsServiceServer::new(collector).accept_compressed(tonic::codec::CompressionEncoding::Gzip);

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(trace_svc)
            .add_service(metrics_svc)
            .add_service(logs_svc)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("mock collector server error");
    });

    (port, telemetry, handle)
}

/// Helper to look up a resource attribute by key.
pub fn find_attribute<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a KeyValue> {
    attrs.iter().find(|kv| kv.key == key)
}

/// Helper to get a string attribute value.
pub fn get_string_attribute(attrs: &[KeyValue], key: &str) -> Option<String> {
    find_attribute(attrs, key)
        .and_then(|kv| kv.value.as_ref())
        .and_then(|v| v.value.as_ref())
        .and_then(|v| match v {
            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => Some(s.clone()),
            _ => None,
        })
}

/// Extract the body text from a log record (empty string if not a string value).
pub fn log_body_text(record: &LogRecord) -> String {
    record
        .body
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| match v {
            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
