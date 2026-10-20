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

//! A test gRPC Otel [`Collector`].
use super::TcpListener;
pub use super::select::{All, Async, Logs, Metrics, Spans, Synch};
use super::select::{CollectorRt, CollectorType};
use super::types;
use super::types::proto;
use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tonic::transport::server::{Router, Server};
use tonic::{Request, Response, Status, async_trait};

#[derive(Debug)]
struct Service<T> {
    data: Mutex<Vec<T>>,
}

impl<T> Service<T> {
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

#[async_trait]
impl LogsService for Service<proto::logs::ResourceLogs> {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let mut req = request.into_inner();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut req.resource_logs);
        }

        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: None,
        }))
    }
}

#[async_trait]
impl MetricsService for Service<proto::metrics::ResourceMetrics> {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let mut req = request.into_inner();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut req.resource_metrics);
        }

        Ok(Response::new(ExportMetricsServiceResponse {
            partial_success: None,
        }))
    }
}

#[async_trait]
impl TraceService for Service<proto::trace::ResourceSpans> {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let mut req = request.into_inner();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut req.resource_spans);
        }

        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

#[async_trait]
impl LogsService for Service<types::Any> {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let req = request.into_inner();
        let mut items = req
            .resource_logs
            .into_iter()
            .map(types::Any::Logs)
            .collect();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut items);
        }

        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: None,
        }))
    }
}

#[async_trait]
impl MetricsService for Service<types::Any> {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let req = request.into_inner();
        let mut items = req
            .resource_metrics
            .into_iter()
            .map(types::Any::Metrics)
            .collect();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut items);
        }

        Ok(Response::new(ExportMetricsServiceResponse {
            partial_success: None,
        }))
    }
}

#[async_trait]
impl TraceService for Service<types::Any> {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let req = request.into_inner();
        let mut items = req
            .resource_spans
            .into_iter()
            .map(types::Any::Spans)
            .collect();

        {
            let mut guard = self.data.lock().unwrap();
            guard.append(&mut items);
        }

        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

impl Collector<All, Synch> {
    /// Create a synchronous collector for logs, metrics, and spans.
    ///
    /// The collector constructs a new [`tokio::runtime::Runtime`] with default
    /// settings (e.g. 3 threads), and begins serving immediately. Call
    /// [`Collector::finish()`] to shut down the collector and return any
    /// captured information.
    ///
    /// If you want to run on a custom or an existing runtime, use the `*_async`
    /// methods, e.g. [`Collector::all_async`].
    ///
    /// If the test case needs to be run in an asynchronous context,
    /// [`Collector::block_on`] can be used to run a future on the runtime.
    /// Synchronous code can simply be run in the test case as normal.
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
        let service = Service::new();

        let logs_svc = LogsServiceServer::from_arc(Arc::clone(&service));
        let metrics_svc = MetricsServiceServer::from_arc(Arc::clone(&service));
        let spans_svc = TraceServiceServer::from_arc(Arc::clone(&service));
        let router = Server::builder()
            .add_service(logs_svc)
            .add_service(metrics_svc)
            .add_service(spans_svc);

        Self::new(listener, service, router)
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
        let service = Service::new();

        let logs_svc = LogsServiceServer::from_arc(Arc::clone(&service));
        let router = Server::builder().add_service(logs_svc);

        Self::new(listener, service, router)
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
        let service = Service::new();

        let metrics_svc = MetricsServiceServer::from_arc(Arc::clone(&service));
        let router = Server::builder().add_service(metrics_svc);

        Self::new(listener, service, router)
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
        let service = Service::new();

        let spans_svc = TraceServiceServer::from_arc(Arc::clone(&service));
        let router = Server::builder().add_service(spans_svc);

        Self::new(listener, service, router)
    }
}

/// A test gRPC Otel [`Collector`].
#[derive(Debug)]
pub struct Collector<Ty: CollectorType, Rt: CollectorRt> {
    rt: Rt,
    service: Arc<Service<Ty::Proto>>,
    shutdown_tx: oneshot::Sender<()>,
    handle: JoinHandle<Result<(), tonic::transport::Error>>,
}

impl<Ty: CollectorType, Rt: CollectorRt> Collector<Ty, Rt> {
    fn new(
        listener: TcpListener,
        service: Arc<Service<Ty::Proto>>,
        router: Router<tower_layer::Identity>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let rt = Rt::build();

        let signal = async move {
            let _ = shutdown_rx.await;
        };

        let handle = rt.spawn(async move {
            let listener = listener.into_tokio();
            let incoming = tonic::transport::server::TcpIncoming::from(listener);

            router.serve_with_incoming_shutdown(incoming, signal).await
        });

        Self {
            rt,
            service,
            shutdown_tx,
            handle,
        }
    }
}

impl<T: CollectorType> Collector<T, Synch> {
    /// Runs a future to completion on the collector's runtime.
    ///
    /// Do not run synchronous code in the future.
    #[inline]
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }

    /// Shuts down the collector and returns any captured information.
    pub fn finish(self) -> T::Output {
        let Self {
            rt,
            service,
            shutdown_tx,
            handle,
        } = self;

        let _res: Result<(), ()> = shutdown_tx.send(());
        let _res: () = rt
            .block_on(handle)
            .expect("failed to join server")
            .expect("failed to run server");

        let proto = service.take();
        T::convert(proto)
    }
}

impl<T: CollectorType> Collector<T, Async> {
    /// Shuts down the collector and returns any captured information.
    pub async fn finish(self) -> T::Output {
        let Self {
            rt: _,
            service,
            shutdown_tx,
            handle,
        } = self;

        let _res: Result<(), ()> = shutdown_tx.send(());
        let _res: () = handle
            .await
            .expect("failed to join server")
            .expect("failed to run server");

        let proto = service.take();
        T::convert(proto)
    }
}
