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

#![allow(deprecated)]
//! Server metric layer with OpenTelemetry-based metric names.
//!
//! Metrics loosely follow the [OpenTelemetry semantic conventions for RPC metrics].
//!
//! See [`TonicMetricLayer::builder()`] to get started.
//!
//! [OpenTelemetry semantic conventions for RPC metrics]: https://github.com/open-telemetry/semantic-conventions/blob/v1.39.0/docs/rpc/rpc-metrics.md
use crate::common::as_u64;
use crate::common::classify::server;
use crate::common::extract::{self, GRPC_MESSAGE, GRPC_STATUS, GRPC_STATUS_DETAILS};
use crate::rpc_map::RpcMap;
use crate::server_verbose as verbose;
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::{KeyValue, StringValue, Value};
use opentelemetry_semantic_conventions::{attribute as attr, metric as sem};
use pin_project_lite::pin_project;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::body::Body;
use tonic::codegen::http;
use tower_layer::Layer;
use tower_service::Service;

/// To cut down on the number of attributes, instead of `rpc.system` and
/// `rpc.method`, we record `rpc`.
const ATTR_RPC: &str = "rpc";
/// Non-semantic convention attribute to indicate whether a gRPC status code
/// is [a server error](https://opentelemetry.io/docs/specs/semconv/rpc/grpc/#grpc-status).
const ATTR_RPC_GRPC_SERVER_ERROR: &str = "rpc.grpc.server_error";

fn attr_rpc(rpc: &Value) -> KeyValue {
    KeyValue::new(ATTR_RPC, rpc.clone())
}

fn attr_status_code(code: Option<u16>) -> KeyValue {
    let value = code.unwrap_or(tonic::Code::Unknown as _);
    KeyValue::new(attr::RPC_GRPC_STATUS_CODE, value as i64)
}

fn attr_server_err(code: Option<u16>) -> KeyValue {
    let value = code.map(server::code_is_error).unwrap_or(false);
    KeyValue::new(ATTR_RPC_GRPC_SERVER_ERROR, value)
}

fn header_value_len(v: &http::HeaderValue) -> usize {
    v.as_bytes().len()
}

/// The builder for [`TonicMetricLayer`].
#[derive(Debug)]
#[deprecated = "Omniverse Metrics SRD mandated A66-style metrics. Additionally, the A66 proposal is more stable."]
pub struct TonicMetricLayerBuilder {
    meter: Meter,
    rpc_map: RpcMap,
}

impl TonicMetricLayerBuilder {
    pub(super) fn new(meter: Meter) -> Self {
        Self {
            meter,
            rpc_map: RpcMap::default(),
        }
    }

    /// Assign an [`RpcMap`] to the layer.
    ///
    /// It's recommended to provide an RPC map as knowing which RPCs to expect
    /// allows the layer to optimize some operations.
    #[inline]
    pub fn with_rpc_map(mut self, rpc_map: RpcMap) -> Self {
        self.rpc_map = rpc_map;
        self
    }

    /// Build the tonic server metric layer.
    #[inline]
    #[deprecated = "Omniverse Metrics SRD mandated A66-style metrics. Additionally, the A66 proposal is more stable."]
    pub fn build(self) -> TonicMetricLayer {
        let Self { meter, rpc_map } = self;
        TonicMetricLayer { meter, rpc_map }
    }
}

/// Tonic server metric layer.
#[derive(Debug, Clone)]
#[deprecated = "Omniverse Metrics SRD mandated A66-style metrics. Additionally, the A66 proposal is more stable."]
pub struct TonicMetricLayer {
    meter: Meter,
    rpc_map: RpcMap,
}

impl TonicMetricLayer {
    /// Create a builder for the server metric layer with an empty RPC map.
    ///
    /// Use this with `tower`'s `ServiceBuilder` to construct services
    /// instrumented with gRPC OpenTelemetry-based metrics.
    ///
    /// See [`RpcMap::builder()`] for what an RPC map is and why to use them.
    ///
    /// ```rust
    /// use tonic_observability::server::metric_otel::TonicMetricLayer;
    /// use tonic_observability::rpc_map::RpcMap;
    /// // see `RpcMap::builder()` for a more complete example
    /// let rpc_map = RpcMap::builder().build();
    /// // this is an example of how to get a meter
    /// let meter = opentelemetry::global::meter("my-meter");
    ///
    /// let layer = TonicMetricLayer::builder(meter)
    ///     .with_rpc_map(rpc_map)
    ///     .build();
    /// ```
    #[inline]
    #[deprecated = "Omniverse Metrics SRD mandated A66-style metrics. Additionally, the A66 proposal is more stable."]
    pub fn builder(meter: Meter) -> TonicMetricLayerBuilder {
        TonicMetricLayerBuilder::new(meter)
    }
}

impl<Inner> Layer<Inner> for TonicMetricLayer {
    type Service = TonicMetricService<Inner>;

    fn layer(&self, inner: Inner) -> Self::Service {
        let rpc_map = self.rpc_map.clone();

        // See https://opentelemetry.io/docs/specs/semconv/rpc/rpc-metrics/
        //
        // Note:
        //
        // * `rpc.server.duration` is not implemented, as it does not apply to
        //   streaming RPCs.
        // * `rpc.server.request.size` and `rpc.server.response.size` may be the
        //   compressed size (assuming compression is used). Otel specifies
        //   uncompressed size, but this is both technically
        //   difficult/inefficient and the gRPC metrics spec (A66) also
        //   specifies the compressed size.
        let request_size = self
            .meter
            .u64_histogram(sem::RPC_SERVER_REQUEST_SIZE)
            .with_unit("By")
            .build();

        let requests_per_rpc = self
            .meter
            .u64_histogram(sem::RPC_SERVER_REQUESTS_PER_RPC)
            .with_unit("{count}")
            .build();

        let active_requests = self
            .meter
            .i64_up_down_counter("rpc.server.active_requests")
            .with_unit("{request}")
            .build();

        let req_metrics = TonicRequestMetrics {
            request_size,
            requests_per_rpc,
            active_requests,
        };

        let response_size = self
            .meter
            .u64_histogram(sem::RPC_SERVER_RESPONSE_SIZE)
            .with_unit("By")
            .build();

        let responses_per_rpc = self
            .meter
            .u64_histogram(sem::RPC_SERVER_RESPONSES_PER_RPC)
            .with_unit("{count}")
            .build();

        let calls = self
            .meter
            .u64_counter("rpc.server.calls")
            .with_unit("{count}")
            .build();

        let active_methods = self
            .meter
            .i64_up_down_counter("rpc.server.active_methods")
            .with_unit("{method}")
            .build();

        let active_responses = self
            .meter
            .i64_up_down_counter("rpc.server.active_responses")
            .with_unit("{response}")
            .build();

        let resp_metrics = TonicResponseMetrics {
            response_size,
            responses_per_rpc,
            calls,
            active_methods,
            active_responses,
        };

        TonicMetricService {
            inner,
            rpc_map,
            req_metrics,
            resp_metrics,
        }
    }
}

/// For ease of cloning multiple metrics.
#[derive(Debug, Clone)]
struct TonicRequestMetrics {
    request_size: Histogram<u64>,
    requests_per_rpc: Histogram<u64>,
    active_requests: UpDownCounter<i64>,
}

/// For ease of cloning multiple metrics.
#[derive(Debug, Clone)]
struct TonicResponseMetrics {
    response_size: Histogram<u64>,
    responses_per_rpc: Histogram<u64>,
    calls: Counter<u64>,
    active_methods: UpDownCounter<i64>,
    active_responses: UpDownCounter<i64>,
}

/// Tonic server metric service.
///
/// This is exported for server type signatures constructed with a
/// [`TonicMetricLayer`]. A service cannot be constructed directly.
#[derive(Debug, Clone)]
pub struct TonicMetricService<Svc> {
    inner: Svc,
    rpc_map: RpcMap,
    req_metrics: TonicRequestMetrics,
    resp_metrics: TonicResponseMetrics,
}

impl<Svc> TonicMetricService<Svc> {
    /// Extract the combined gRPC service/method names from a path.
    ///
    /// This uses the provided [`RpcMap`] to use static strings where possible,
    /// which makes cloning values very cheap. Unknown names, including when no
    /// map is configured, share one bounded label to prevent high cardinality.
    fn rpc_from_request_path(&self, path: &str) -> Value {
        let rpc = path.strip_prefix('/').unwrap_or(path);
        let s = self
            .rpc_map
            .find(rpc)
            .unwrap_or_else(|| StringValue::from("_OTHER"));
        Value::String(s)
    }
}

impl<Svc, RespBody> Service<http::Request<Body>> for TonicMetricService<Svc>
where
    Svc: Service<http::Request<Body>, Response = http::Response<RespBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = TonicMetricResponse<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        verbose!("metrics service call");
        let rpc = self.rpc_from_request_path(request.uri().path());
        let attributes = &[attr_rpc(&rpc)];

        // wrap the request
        let request = request.map(|inner| {
            let body = TonicMetricRequestBody {
                inner,
                count: 0,
                rpc: rpc.clone(),
                metrics: self.req_metrics.clone(),
            };
            self.req_metrics.active_requests.add(1, attributes);
            Body::new(body)
        });

        // "create" the future (futures aren't executed until they are polled)
        let inner = self.inner.call(request);

        // wrap the future
        let response = TonicMetricResponse {
            inner,
            rpc,
            metrics: self.resp_metrics.clone(),
        };
        self.resp_metrics.active_methods.add(1, attributes);
        response
    }
}

pin_project! {
    /// Tonic server metric request body.
    ///
    /// This is a wrapper around the original request body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    struct TonicMetricRequestBody {
        #[pin]
        inner: Body,
        count: u64,
        rpc: Value,
        metrics: TonicRequestMetrics,
    }

    impl PinnedDrop for TonicMetricRequestBody {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricRequestBody::pinned_drop(me)
        }
    }
}

impl TonicMetricRequestBody {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = &[attr_rpc(this.rpc)];

        this.metrics.active_requests.add(-1, attributes);

        // record the number of requests, unless it's zero, since that
        // indicates the request wasn't polled
        let count = *this.count;
        if count > 0 {
            this.metrics.requests_per_rpc.record(count, attributes);
        }
    }
}

impl http_body::Body for TonicMetricRequestBody {
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        // poll the future
        let outcome = match this.inner.poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        let attributes = &[attr_rpc(this.rpc)];

        match &outcome {
            Some(Ok(frame)) => {
                // note: currently, a frame can be either data or trailers,
                // but not both. and there's no way to match exhaustively.

                if let Some(data) = frame.data_ref() {
                    let size = data.len();
                    verbose!("metrics req body data ({})", size);

                    // zero-sized frames occur regularly at the end of a
                    // request:
                    // > In scenarios where the Request stream needs to be
                    // > closed but no data remains to be sent implementations
                    // > MUST send an empty DATA frame with this flag set.
                    // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                    if size != 0 {
                        *this.count += 1;
                        this.metrics.request_size.record(as_u64!(size), attributes);
                    }
                } // cov-exclude [branch]: gRPC requests may not have trailers

                // gRPC requests may not have trailers, instead EOS is used
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                if frame.is_trailers() {
                    verbose!("metrics req body trailers (!!!)"); // cov-exclude [line]: see above
                }
            }
            Some(Err(status)) => {
                // record the "size" as best we can
                let size = status.message().len().wrapping_add(status.details().len());

                // unsure how to trigger this branch... it should not happen in
                // normal gRPC flow, and may just be required by the HTTP/2
                // typing for services?
                verbose!("metrics req body err ({}): {}", size, status);

                // count this as a request
                *this.count += 1;
                this.metrics.request_size.record(as_u64!(size), attributes);
            }
            None => {
                // end-of-stream (EOS)
                verbose!("metrics req body eos");
            }
        }

        Poll::Ready(outcome)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

pin_project! {
    /// Tonic server metric response.
    ///
    /// This is exported for type signatures when using a
    /// [`TonicMetricLayer`]/[`TonicMetricService`]. A response cannot be
    /// constructed directly.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicMetricResponse<Fut> {
        #[pin]
        inner: Fut,
        rpc: Value,
        metrics: TonicResponseMetrics,
    }

    impl<Fut> PinnedDrop for TonicMetricResponse<Fut> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricResponse::pinned_drop(me)
        }
    }
}

impl<Fut> TonicMetricResponse<Fut> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = &[attr_rpc(this.rpc)];
        this.metrics.active_methods.add(-1, attributes);
    }
}

impl<Fut, RespBody> Future for TonicMetricResponse<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, Infallible>>,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Output = Result<http::Response<Body>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // poll the future
        let outcome = match this.inner.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        let attributes = &[attr_rpc(this.rpc)];

        match outcome {
            Ok(response) => {
                let headers = response.headers();
                // if the `grpc-status` header is present, record it. otherwise,
                // it should be in the trailers.
                let code = headers
                    .get(GRPC_STATUS)
                    .and_then(extract::resp_grpc_status_from_header_value);

                // every tonic service is wrapped with the `RecoverError` middleware:
                // https://github.com/hyperium/tonic/blob/v0.13.0/tonic/src/transport/server/mod.rs#L1086
                // this converts `Status` "errors" to responses with empty bodies:
                // https://github.com/hyperium/tonic/blob/v0.13.0/tonic/src/service/recover_error.rs#L100-L101
                // and `tonic::body::Body::empty()` does not call `poll_frame`:
                // https://github.com/hyperium/tonic/blob/v0.13.0/tonic/src/body.rs#L75-L78
                //
                // so, if the service returns a `Status`, it is converted into
                // an empty body, and `TonicMetricResponseBody` can't do much.
                //
                // the only semi-reliable way to detect this is via
                // end-of-stream and/or the size hint.
                let response = if response.body().is_end_stream() {
                    let message_size = headers
                        .get(GRPC_MESSAGE)
                        .map(header_value_len)
                        .unwrap_or_default();
                    let details_size = headers
                        .get(GRPC_STATUS_DETAILS)
                        .map(header_value_len)
                        .unwrap_or_default();
                    let size = message_size.wrapping_add(details_size);

                    verbose!("metrics response end stream ({})", size);

                    this.metrics.responses_per_rpc.record(1, attributes);
                    this.metrics.response_size.record(as_u64!(size), attributes);

                    let call_attributes = &[
                        attr_rpc(this.rpc),
                        attr_status_code(code),
                        attr_server_err(code),
                    ];
                    this.metrics.calls.add(1, call_attributes);

                    response.map(|inner| {
                        let body = Body::new(TonicMetricNoopBody {
                            inner,
                            rpc: this.rpc.clone(),
                            active_responses: this.metrics.active_responses.clone(),
                        });
                        this.metrics.active_responses.add(1, attributes);
                        body
                    })
                } else {
                    verbose!("metrics response start stream");
                    // wrap the response body
                    response.map(|inner| {
                        let body = Body::new(TonicMetricResponseBody {
                            inner,
                            count: 0,
                            rpc: this.rpc.clone(),
                            metrics: this.metrics.clone(),
                            code,
                        });
                        this.metrics.active_responses.add(1, attributes);
                        body
                    })
                };

                Poll::Ready(Ok(response))
            }
        }
    }
}

pin_project! {
    /// Tonic server metric response body.
    ///
    /// This is a wrapper around the original response body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    struct TonicMetricResponseBody<RespBody> {
        #[pin]
        inner: RespBody,
        count: u64,
        rpc: Value,
        metrics: TonicResponseMetrics,
        code: Option<u16>,
    }

    impl<RespBody> PinnedDrop for TonicMetricResponseBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricResponseBody::pinned_drop(me)
        }
    }
}

impl<RespBody> TonicMetricResponseBody<RespBody> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = &[attr_rpc(this.rpc)];

        this.metrics.active_responses.add(-1, attributes);

        // record the number of responses
        let count = *this.count;
        this.metrics.responses_per_rpc.record(count, attributes);

        let code = *this.code;
        let call_attributes = &[
            attr_rpc(this.rpc),
            attr_status_code(code),
            attr_server_err(code),
        ];
        this.metrics.calls.add(1, call_attributes);
    }
}

impl<RespBody> http_body::Body for TonicMetricResponseBody<RespBody>
where
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status>,
{
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        let outcome = match this.inner.poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        let attributes = &[attr_rpc(this.rpc)];

        match &outcome {
            Some(Ok(frame)) => {
                // note: currently, a frame can be either data or trailers,
                // but not both. and there's no way to match exhaustively.

                if let Some(data) = frame.data_ref() {
                    let size = data.len();
                    verbose!("metrics resp body data ({})", size);

                    *this.count += 1;
                    this.metrics.response_size.record(as_u64!(size), attributes);
                }

                // trailers signify the end of the response
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                if let Some(trailers) = frame.trailers_ref() {
                    let status = trailers.get(GRPC_STATUS);
                    let message = trailers.get(GRPC_MESSAGE);

                    *this.code = status
                        .and_then(extract::resp_grpc_status_from_header_value)
                        .or(*this.code);

                    let has_ok_status = status.map(|v| v.as_bytes() == b"0").unwrap_or(false);
                    let has_err_status = !has_ok_status;
                    let should_record = has_err_status && message.is_some();

                    verbose!("metrics resp body trailer ({})", should_record);

                    let _ = should_record;

                    if let Some(message) = message
                        && has_err_status
                    {
                        let details = trailers.get(GRPC_STATUS_DETAILS);
                        let details_size = details.map(header_value_len).unwrap_or_default();
                        let message_size = header_value_len(message);
                        let size = message_size.wrapping_add(details_size);

                        verbose!("metrics resp body status ({})", size);

                        *this.count += 1;
                        this.metrics.response_size.record(as_u64!(size), attributes);
                    }
                }
            }
            Some(Err(status)) => {
                // record the "size" as best we can
                let size = status.message().len().wrapping_add(status.details().len());

                verbose!("metrics resp body err ({}): {}", size, status);

                // an error is also a response
                *this.count += 1;
                *this.code = Some(status.code() as _);
                this.metrics.response_size.record(as_u64!(size), attributes);
            }
            None => {
                // responses shouldn't use end-of-stream (EOS)
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                verbose!("metrics resp body eos");
            }
        }

        Poll::Ready(outcome)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

pin_project! {
    /// Tonic server metric no-op response body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    struct TonicMetricNoopBody<RespBody> {
        #[pin]
        inner: RespBody,
        rpc: Value,
        active_responses: UpDownCounter<i64>,
    }

    impl<RespBody> PinnedDrop for TonicMetricNoopBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricNoopBody::pinned_drop(me)
        }
    }
}

impl<RespBody> TonicMetricNoopBody<RespBody> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = &[attr_rpc(this.rpc)];
        this.active_responses.add(-1, attributes);
    }
}

impl<RespBody> http_body::Body for TonicMetricNoopBody<RespBody>
where
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status>,
{
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    #[inline]
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.project().inner.poll_frame(cx)
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests;
