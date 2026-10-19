// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::http;
use opentelemetry::metrics::UpDownCounter;
use opentelemetry::KeyValue;
use tower::{Layer, Service};

use crate::metrics::Metrics;

struct InFlightGuard(UpDownCounter<i64>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.add(-1, &[]);
    }
}

#[derive(Clone)]
pub struct GrpcMetricsLayer {
    metrics: Arc<Metrics>,
}

impl GrpcMetricsLayer {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }
}

impl<S> Layer<S> for GrpcMetricsLayer {
    type Service = GrpcMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcMetricsService {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

#[derive(Clone)]
pub struct GrpcMetricsService<S> {
    inner: S,
    metrics: Arc<Metrics>,
}

fn extract_grpc_service_method(path: &str) -> (&str, &str) {
    let trimmed = path.trim_start_matches('/');
    trimmed.split_once('/').unwrap_or((trimmed, ""))
}

fn extract_grpc_status<B>(response: &http::Response<B>) -> i32 {
    response
        .headers()
        .get("grpc-status")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for GrpcMetricsService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        let metrics = self.metrics.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        let path = request.uri().path().to_string();
        let (service, method) = extract_grpc_service_method(&path);
        let service = service.to_string();
        let method = method.to_string();
        let start = Instant::now();

        metrics.rpc_requests_in_flight.add(1, &[]);
        let in_flight_guard = InFlightGuard(metrics.rpc_requests_in_flight.clone());

        Box::pin(async move {
            let _in_flight_guard = in_flight_guard;
            let result = inner.call(request).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;

            let status_code = match &result {
                Ok(response) => extract_grpc_status(response),
                Err(_) => 2,
            };

            let attrs = [
                KeyValue::new("rpc.service", service),
                KeyValue::new("rpc.method", method),
                KeyValue::new("rpc.grpc.status_code", i64::from(status_code)),
            ];

            metrics.rpc_request_count.add(1, &attrs);
            metrics.rpc_request_duration.record(elapsed_ms, &attrs);

            if status_code != 0 {
                metrics.rpc_request_errors.add(1, &attrs);
            }

            result
        })
    }
}
