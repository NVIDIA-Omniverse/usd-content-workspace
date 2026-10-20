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

use super::{TracesConfig, TracesGuard};
use crate::testing::unsafe_env;

macro_rules! assert_disabled {
    ($guard:ident) => {
        assert!($guard.0.is_none());
    };
}

#[test]
fn disabled_via_config() {
    let config = TracesConfig::disabled_for_test();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_otel_sdk_disabled() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "true",
    }

    let config = TracesConfig::from_env();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_otel_traces_exporter() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "none",
    }

    let config = TracesConfig::from_env();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_guard() {
    let guard = TracesGuard::disabled_for_test();
    assert_disabled!(guard);
}

#[test]
fn tracer_traits() {
    fn send_sync_static<T>(t: &Option<T>)
    where
        T: Send + Sync + 'static,
    {
        let _ = t;
    }

    fn otel_tracer<T>(t: &Option<T>)
    where
        T: opentelemetry::trace::Tracer + Send + Sync + 'static,
        T::Span: opentelemetry::trace::Span + Send + Sync + 'static,
    {
        let _ = t;
    }

    fn object_safe_tracer<T>(t: &Option<T>)
    where
        T: opentelemetry::global::ObjectSafeTracer,
    {
        let _ = t;
    }

    fn object_safe_span<T>(t: &Option<T>)
    where
        T: opentelemetry::trace::Tracer,
        T::Span: opentelemetry::global::ObjectSafeSpan,
    {
        let _ = t;
    }

    let guard = TracesGuard::disabled_for_test();
    let tracer = guard.tracer("foo");

    send_sync_static(&tracer);
    otel_tracer(&tracer);
    object_safe_tracer(&tracer);
    object_safe_span(&tracer);
}

#[test]
fn tracer_can_box() {
    use opentelemetry::global::{BoxedSpan, BoxedTracer};

    let guard = TracesGuard::disabled_for_test();

    let tracer: Option<BoxedTracer> = guard
        .tracer("foo")
        .map(|tracer: _| BoxedTracer::new(Box::new(tracer)));

    let _span: Option<BoxedSpan> = tracer.as_ref().map(|tracer| {
        use opentelemetry::trace::Tracer as _;
        tracer.span_builder("bar").start(tracer)
    });
}

#[test]
fn shutdown_handle_debug() {
    let guard = TracesGuard::disabled_for_test();
    let handle = guard.shutdown_handle();
    assert_eq!(format!("{handle:?}"), "TracesShutdownHandle { .. }");
}
