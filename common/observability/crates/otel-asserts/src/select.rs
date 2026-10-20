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

use super::types;
use super::types::proto;
use std::fmt;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

pub trait CollectorType: Sized + fmt::Debug + Send + Sync + 'static {
    type Proto: Sized + fmt::Debug + Send + Sync + 'static;
    type Output: Sized + Send + Sync + 'static;

    fn convert(proto: Vec<Self::Proto>) -> Self::Output;
}

/// Marker for logs-only collectors.
#[derive(Debug)]
pub struct Logs;

impl CollectorType for Logs {
    type Proto = proto::logs::ResourceLogs;
    type Output = types::logs::OutputLogs;

    fn convert(proto: Vec<Self::Proto>) -> Self::Output {
        proto.into()
    }
}

/// Marker for metrics-only collectors.
#[derive(Debug)]
pub struct Metrics;

impl CollectorType for Metrics {
    type Proto = proto::metrics::ResourceMetrics;
    type Output = types::metrics::OutputMetrics;

    fn convert(proto: Vec<Self::Proto>) -> Self::Output {
        proto.into()
    }
}

/// Marker for trace/spans-only collectors.
#[derive(Debug)]
pub struct Spans;

impl CollectorType for Spans {
    type Proto = proto::trace::ResourceSpans;
    type Output = types::trace::OutputSpans;

    fn convert(proto: Vec<Self::Proto>) -> Self::Output {
        proto.into()
    }
}

/// Marker for logs, metrics, and trace/spans collectors.
#[derive(Debug)]
pub struct All;

impl CollectorType for All {
    type Proto = types::Any;
    type Output = (
        types::logs::OutputLogs,
        types::metrics::OutputMetrics,
        types::trace::OutputSpans,
    );

    fn convert(items: Vec<Self::Proto>) -> Self::Output {
        let mut resource_logs = Vec::new();
        let mut resource_metrics = Vec::new();
        let mut resource_spans = Vec::new();
        for item in items {
            match item {
                types::Any::Logs(v) => resource_logs.push(v.into()),
                types::Any::Metrics(v) => resource_metrics.push(v.into()),
                types::Any::Spans(v) => resource_spans.push(v.into()),
            }
        }
        (
            types::logs::OutputLogs { resource_logs },
            types::metrics::OutputMetrics { resource_metrics },
            types::trace::OutputSpans { resource_spans },
        )
    }
}

pub trait CollectorRt: Sized + fmt::Debug + Send + Sync + 'static {
    fn build() -> Self;

    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}

/// Marker for a synchronous collector.
#[derive(Debug)]
#[repr(transparent)]
pub struct Synch(Runtime);

impl CollectorRt for Synch {
    fn build() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .enable_all()
            .build()
            .expect("failed to build the runtime");
        Self(rt)
    }

    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.0.spawn(future)
    }
}

impl Synch {
    pub(crate) fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.0.block_on(future)
    }
}

/// Marker for an asynchronous collector.
#[derive(Debug)]
pub struct Async;

impl CollectorRt for Async {
    fn build() -> Self {
        Self
    }

    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future)
    }
}
