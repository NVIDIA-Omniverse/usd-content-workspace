// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use super::traits::{Greeter, HelloReply, HelloRequest};
use std::sync::Arc;
use storage_core::layer::Layer;
use storage_core::metrics::record_operation;
use storage_core::metrics::utils::{latency_base, results_counter_base, MethodMetrics};
use storage_core::traits::{async_trait, Request, Result};
use tracing::instrument;

// --- Layer ---
pub struct GreeterMetricsLayer {
    storage_backend: String,
}

impl GreeterMetricsLayer {
    pub fn new(storage_backend: impl Into<String>) -> Self {
        Self {
            storage_backend: storage_backend.into(),
        }
    }
}

impl<S> Layer<S> for GreeterMetricsLayer {
    type Service = GreeterMetrics<S>;

    fn layer(&self, service: S) -> Self::Service {
        GreeterMetrics::new(service, &self.storage_backend)
    }
}

// --- Metrics Wrapper ---
pub struct GreeterMetrics<S> {
    inner: S,
    say_hello: Arc<MethodMetrics>,
}

impl<S> GreeterMetrics<S> {
    pub fn new(inner: S, storage_backend: &str) -> Self {
        Self {
            inner,
            say_hello: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "say_hello",
                "v1",
                storage_backend,
            )),
        }
    }
}

// --- Trait Impl ---
#[async_trait]
impl<S: Greeter> Greeter for GreeterMetrics<S> {
    fn inner(&self) -> Option<&dyn Greeter> {
        Some(&self.inner)
    }

    #[instrument(name = "greeter_metrics::say_hello", skip_all)]
    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<HelloReply> {
        let _timer = self.say_hello.latency.start();
        record_operation!(self.say_hello, self.inner.say_hello(request).await)
    }
}
