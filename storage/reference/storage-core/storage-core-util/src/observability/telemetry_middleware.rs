// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::warn_limited;
use hyper::{body::Body as HttpBody, Request};
use opentelemetry::{metrics as otel_metrics, metrics::Meter, KeyValue as OtelKeyValue};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};
use tower::{Layer, Service};

/// A layer which collects metrics for gRPC calls.
///
/// The end output of this layer are the server metrics specified by [A66: OpenTelemetry Metrics][grpc-a66] written out
/// to the [`Meter`] provided with [`GrpcServerTelemetryLayer::new`].
///
/// [grpc-a66]: https://github.com/grpc/proposal/blob/master/A66-otel-stats.md
#[derive(Clone, Debug)]
pub struct GrpcServerTelemetryLayer {
    metrics: Arc<GrpcServerMetrics>,
}

impl GrpcServerTelemetryLayer {
    /// Create an instance outputting metrics to the `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self {
            metrics: Arc::new(GrpcServerMetrics::new(meter)),
        }
    }
}

impl<S> Layer<S> for GrpcServerTelemetryLayer {
    type Service = GrpcServerTelemetryMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcServerTelemetryMiddleware {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GrpcServerTelemetryMiddleware<S> {
    inner: S,
    metrics: Arc<GrpcServerMetrics>,
}

/// The output type of a `Box::pin(async move { ... })` block.
type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

impl<S, ReqBody, RespBody> Service<http::Request<ReqBody>> for GrpcServerTelemetryMiddleware<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<RespBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: HttpBody + Send + 'static,
    RespBody: HttpBody + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        // start the timer immediately, but don't report until we're off the initial network task
        let metric_track = self.metrics.enter(&req);
        Box::pin(async move {
            let response = inner.call(req).await?;
            Ok(metric_track.exit_with(response))
        })
    }
}

/// Extract relevant attributes from the request.
///
/// ## TODO
/// This function should not return a `Vec`, but something optimized for the fact that there are very few elements.
fn extract_attrs<B>(request: &Request<B>) -> Vec<OtelKeyValue> {
    let mut out = Vec::with_capacity(8); // <- is it the right number? Who knows?
    let path = request.uri().path();
    if let Err(reason) = extract_attrs_from_path(&mut out, path) {
        warn_limited!("provided URI {path:?} does not look like a gRPC call because {reason}");
    }
    out
}

/// Used by [`extract_attrs`] to collect from the `path` URI into `out`.
fn extract_attrs_from_path(out: &mut Vec<OtelKeyValue>, path: &str) -> Result<(), &'static str> {
    if !path.starts_with('/') {
        return Err("path did not start with '/'");
    }
    let path = &path[1..];

    // Get exactly 2 elements by splitting on '/'
    let mut path_iter = path.split('/');
    let Some(service) = path_iter.next() else {
        return Err("empty after initial '/'");
    };
    if service.is_empty() {
        return Err("service portion is an empty string");
    }

    let Some(name) = path_iter.next() else {
        return Err("did not find method name after '/'");
    };
    if name.is_empty() {
        return Err("method name portion is an empty string");
    }

    if path_iter.next().is_some() {
        return Err("too many '/'s");
    }

    out.push(OtelKeyValue::new("grpc.method", path.to_owned()));
    out.push(OtelKeyValue::new("grpc.service", service.to_owned()));
    out.push(OtelKeyValue::new("grpc.name", name.to_owned()));

    Ok(())
}

/// Wrapper for server metrics specified by [A66: OpenTelemetry Metrics][grpc-a66].
///
/// [grpc-a66]: https://github.com/grpc/proposal/blob/master/A66-otel-stats.md
#[derive(Clone, Debug)]
struct GrpcServerMetrics {
    started: otel_metrics::Counter<u64>,
    rcvd_msg_size: otel_metrics::Histogram<u64>,
    sent_msg_size: otel_metrics::Histogram<u64>,
    duration: otel_metrics::Histogram<f64>,
}

impl GrpcServerMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            // WARNING: These descriptions MUST match the C++ code that also collects metrics with the same name or the
            // metric collection will fail! If we need to update the descriptions, it must be synchronized across all
            // possible metrics generators. This should be fixed at a higher level, but has not been at the time of
            // writing.
            started: meter.u64_counter("grpc.server.call.started")
                .with_description("Number of server calls started")
                .with_unit("call")
                .build(),
            rcvd_msg_size: meter.u64_histogram("grpc.server.call.rcvd_total_compressed_message_size")
                .with_description("Total bytes (compressed but not encrypted) received across all request messages (metadata excluded) per RPC")
                .with_unit("By")
                .with_boundaries(HistogramCommonBucket::Data.boundaries().into())
                .build(),
            sent_msg_size: meter.u64_histogram("grpc.server.call.sent_total_compressed_message_size")
                .with_description("Compressed message bytes sent per server call")
                .with_unit("By")
                .with_boundaries(HistogramCommonBucket::Data.boundaries().into())
                .build(),
            duration: meter.f64_histogram("grpc.server.call.duration")
                .with_description("End-to-end time taken to complete a call from server transport's perspective")
                .with_unit("s")
                .with_boundaries(HistogramCommonBucket::Time.boundaries().into())
                .build(),
        }
    }

    pub fn enter<B: HttpBody>(self: &Arc<Self>, request: &'_ hyper::Request<B>) -> GrpcServerMetricsGuard {
        let start = Instant::now();
        let attrs = extract_attrs(request);
        self.started.add(1, &attrs);
        if let Some(recv_sz) = request.size_hint().exact() {
            self.rcvd_msg_size.record(recv_sz, &attrs);
        }
        GrpcServerMetricsGuard {
            on: Some(self.clone()),
            attrs,
            start,
        }
    }
}

struct GrpcServerMetricsGuard {
    on: Option<Arc<GrpcServerMetrics>>,
    attrs: Vec<OtelKeyValue>,
    start: Instant,
}

impl GrpcServerMetricsGuard {
    pub fn exit_with<B: HttpBody>(self, response: http::Response<B>) -> http::Response<B> {
        if let Some(on) = self.on {
            let elapsed = self.start.elapsed();
            let mut attrs = self.attrs;
            attrs.push(OtelKeyValue::new("grpc.status", response.status().as_u16() as i64));

            on.duration.record(elapsed.as_secs_f64(), &attrs);
            if let Some(send_sz) = response.size_hint().exact() {
                on.sent_msg_size.record(send_sz, &attrs);
            }
        }
        response
    }
}

/// Access commonly used histogram buckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistogramCommonBucket {
    /// Used for data size histograms (e.g.: size of a file).
    Data,
    /// Used for timing histograms.
    Time,
}

impl HistogramCommonBucket {
    pub fn boundaries(&self) -> &'static [f64] {
        match self {
            Self::Data => {
                const KIB: usize = 1024;
                const MIB: usize = KIB * KIB;
                const GIB: usize = MIB * KIB;
                &[
                    KIB as f64,
                    (2 * KIB) as f64,
                    (4 * KIB) as f64,
                    (8 * KIB) as f64,
                    (16 * KIB) as f64,
                    (32 * KIB) as f64,
                    (64 * KIB) as f64,
                    (128 * KIB) as f64,
                    (256 * KIB) as f64,
                    (512 * KIB) as f64,
                    MIB as f64,
                    (4 * MIB) as f64,
                    (16 * MIB) as f64,
                    (64 * MIB) as f64,
                    (256 * MIB) as f64,
                    GIB as f64,
                    (4 * GIB) as f64,
                    (16 * GIB) as f64,
                    (64 * GIB) as f64,
                    (256 * GIB) as f64,
                ]
            },
            // Buckets for latency measured in seconds. This is a modification of the suggested values for gRPC to
            // support longer latencies, as seen in Omniverse usage.
            // Suggested gRPC values and example usage:
            // https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#units
            // https://github.com/grpc/grpc/blob/724fa60db4e6ea23de14d8763efcf090323a5ec0/examples/cpp/otel/util.cc#L56
            Self::Time => &[
                0.0, 0.0001, 0.0005, 0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.008, 0.01, 0.013, 0.016, 0.02, 0.025,
                0.03, 0.04, 0.05, 0.065, 0.08, 0.1, 0.13, 0.16, 0.2, 0.25, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0, 2.0, 5.0,
                10.0, 20.0, 50.0, 100.0, 300.0, 600.0, 1000.0,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_attrs_from_path;

    #[test]
    fn extract_attrs_from_path_malformed() {
        for input in [
            "",
            "/",
            "ab/",
            "/nvidia.thing",
            "/nvidia.thing/",
            "//MethodOnly",
            "/nvidia.thing/Method/",
            "/nvidia.thing/Method/Extra",
        ] {
            let mut out = Vec::new();
            extract_attrs_from_path(&mut out, input).unwrap_err();
            assert!(out.is_empty());
        }
    }

    #[test]
    fn extract_attrs_from_path_good() {
        let mut out = Vec::new();
        extract_attrs_from_path(&mut out, "/nvidia.thing/MethodName").unwrap();
        let get_attr = |name| {
            out.iter()
                .find(|elem| elem.key.as_str() == name)
                .map(|elem| &elem.value)
        };

        assert_eq!(get_attr("grpc.method").unwrap().as_str(), "nvidia.thing/MethodName");
        assert_eq!(get_attr("grpc.service").unwrap().as_str(), "nvidia.thing");
        assert_eq!(get_attr("grpc.name").unwrap().as_str(), "MethodName");
    }
}
