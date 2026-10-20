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

//! A test HTTP/Protobuf Otel [`Collector`].
use super::TcpListener;
pub use super::select::{All, Async, Logs, Metrics, Spans, Synch};
use super::select::{CollectorRt, CollectorType};
use super::types;
use super::types::proto;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Response;
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use prost::bytes::BytesMut;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug)]
struct AppState<T> {
    data: Mutex<Vec<T>>,
}

impl<T> AppState<T> {
    fn new() -> Arc<Self> {
        let data = Self {
            data: Mutex::new(Vec::new()),
        };
        Arc::new(data)
    }

    fn take(&self) -> Vec<T> {
        let mut guard = self.data.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

fn decode<T: Message + Default>(body: Bytes) -> T {
    T::decode(body).unwrap()
}

fn encode<T: Message>(v: T) -> Response {
    let mut buf = BytesMut::with_capacity(128);
    v.encode(&mut buf).unwrap();
    let body = Body::from(buf.freeze());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(body)
        .unwrap()
}

async fn logs_handler(
    State(state): State<Arc<AppState<proto::logs::ResourceLogs>>>,
    body: Bytes,
) -> Response {
    let mut req: ExportLogsServiceRequest = decode(body);

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut req.resource_logs);
    }

    encode(ExportLogsServiceResponse {
        partial_success: None,
    })
}

async fn metrics_handler(
    State(state): State<Arc<AppState<proto::metrics::ResourceMetrics>>>,
    body: Bytes,
) -> Response {
    let mut req: ExportMetricsServiceRequest = decode(body);

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut req.resource_metrics);
    }

    encode(ExportMetricsServiceResponse {
        partial_success: None,
    })
}

async fn spans_handler(
    State(state): State<Arc<AppState<proto::trace::ResourceSpans>>>,
    body: Bytes,
) -> Response {
    let mut req: ExportTraceServiceRequest = decode(body);

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut req.resource_spans);
    }

    encode(ExportTraceServiceResponse {
        partial_success: None,
    })
}

async fn any_logs_handler(State(state): State<Arc<AppState<types::Any>>>, body: Bytes) -> Response {
    let req: ExportLogsServiceRequest = decode(body);
    let mut items = req
        .resource_logs
        .into_iter()
        .map(types::Any::Logs)
        .collect();

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut items);
    }

    encode(ExportLogsServiceResponse {
        partial_success: None,
    })
}

async fn any_metrics_handler(
    State(state): State<Arc<AppState<types::Any>>>,
    body: Bytes,
) -> Response {
    let req: ExportMetricsServiceRequest = decode(body);
    let mut items = req
        .resource_metrics
        .into_iter()
        .map(types::Any::Metrics)
        .collect();

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut items);
    }

    encode(ExportMetricsServiceResponse {
        partial_success: None,
    })
}

async fn any_spans_handler(
    State(state): State<Arc<AppState<types::Any>>>,
    body: Bytes,
) -> Response {
    let req: ExportTraceServiceRequest = decode(body);
    let mut items = req
        .resource_spans
        .into_iter()
        .map(types::Any::Spans)
        .collect();

    {
        let mut guard = state.data.lock().unwrap();
        guard.append(&mut items);
    }

    encode(ExportTraceServiceResponse {
        partial_success: None,
    })
}

impl Collector<All, Synch> {
    /// Create a synchronous collector for logs, metrics, and spans.
    ///
    /// See [`crate::grpc::Collector::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn all(listener: TcpListener) -> Self {
        Self::all_rt(listener)
    }
}

impl Collector<All, Async> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn all_async(listener: TcpListener) -> Self {
        Self::all_rt(listener)
    }
}

impl<Rt: CollectorRt> Collector<All, Rt> {
    #[doc(hidden)]
    #[must_use]
    pub fn all_rt(listener: TcpListener) -> Self {
        let router = axum::Router::new()
            .route("/v1/logs", axum::routing::post(any_logs_handler))
            .route("/v1/metrics", axum::routing::post(any_metrics_handler))
            .route("/v1/traces", axum::routing::post(any_spans_handler));

        Self::new(listener, router)
    }
}

impl Collector<Logs, Synch> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn logs(listener: TcpListener) -> Self {
        Self::logs_rt(listener)
    }
}

impl Collector<Logs, Async> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn logs_async(listener: TcpListener) -> Self {
        Self::logs_rt(listener)
    }
}

impl<Rt: CollectorRt> Collector<Logs, Rt> {
    #[doc(hidden)]
    #[must_use]
    pub fn logs_rt(listener: TcpListener) -> Self {
        let router = axum::Router::new().route("/v1/logs", axum::routing::post(logs_handler));

        Self::new(listener, router)
    }
}

impl Collector<Metrics, Synch> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn metrics(listener: TcpListener) -> Self {
        Self::metrics_rt(listener)
    }
}

impl Collector<Metrics, Async> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn metrics_async(listener: TcpListener) -> Self {
        Self::metrics_rt(listener)
    }
}

impl<Rt: CollectorRt> Collector<Metrics, Rt> {
    #[doc(hidden)]
    #[must_use]
    pub fn metrics_rt(listener: TcpListener) -> Self {
        let router = axum::Router::new().route("/v1/metrics", axum::routing::post(metrics_handler));

        Self::new(listener, router)
    }
}

impl Collector<Spans, Synch> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn spans(listener: TcpListener) -> Self {
        Self::spans_rt(listener)
    }
}

impl Collector<Spans, Async> {
    /// See [`Self::all()`] for more details.
    #[must_use]
    #[inline]
    pub fn spans_async(listener: TcpListener) -> Self {
        Self::spans_rt(listener)
    }
}

impl<Rt: CollectorRt> Collector<Spans, Rt> {
    #[doc(hidden)]
    #[must_use]
    pub fn spans_rt(listener: TcpListener) -> Self {
        let router = axum::Router::new().route("/v1/traces", axum::routing::post(spans_handler));

        Self::new(listener, router)
    }
}

/// A test HTTP/Protobuf Otel [`Collector`].
#[derive(Debug)]
pub struct Collector<Ty: CollectorType, Rt: CollectorRt> {
    rt: Rt,
    state: Arc<AppState<Ty::Proto>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    handle: JoinHandle<Result<(), std::io::Error>>,
}

impl<Ty: CollectorType, Rt: CollectorRt> Collector<Ty, Rt> {
    fn new(listener: TcpListener, router: axum::Router<Arc<AppState<Ty::Proto>>>) -> Self {
        let state = AppState::new();
        let make_service = router.with_state(Arc::clone(&state));

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let rt = Rt::build();

        let handle = rt.spawn(async {
            let tcp_listener = listener.into_tokio();
            let signal = async move {
                let _ = shutdown_rx.await;
            };
            axum::serve(tcp_listener, make_service)
                .with_graceful_shutdown(signal)
                .await
        });

        Self {
            rt,
            state,
            shutdown_tx,
            handle,
        }
    }
}

impl<Ty: CollectorType> Collector<Ty, Synch> {
    /// Runs a future to completion on the collector's runtime.
    ///
    /// Do not run blocking synchronous code in the future.
    #[inline]
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }

    /// Shuts down the collector and returns any captured information.
    pub fn finish(self) -> Ty::Output {
        let Self {
            rt,
            state,
            shutdown_tx,
            handle,
        } = self;

        let _res: Result<(), ()> = shutdown_tx.send(());
        let _res: () = rt
            .block_on(handle)
            .expect("failed to join server")
            .expect("failed to run server");

        let proto = state.take();
        Ty::convert(proto)
    }
}

impl<Ty: CollectorType> Collector<Ty, Async> {
    /// Shuts down the collector and returns any captured information.
    pub async fn finish(self) -> Ty::Output {
        let Self {
            rt: _,
            state,
            shutdown_tx,
            handle,
        } = self;

        let _res: Result<(), ()> = shutdown_tx.send(());
        let _res: () = handle
            .await
            .expect("failed to join server")
            .expect("failed to run server");

        let proto = state.take();
        Ty::convert(proto)
    }
}
