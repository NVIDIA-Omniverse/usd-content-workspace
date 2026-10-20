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

use crate::exporters::ShutdownError;
use opentelemetry::global::{ObjectSafeSpan, ObjectSafeTracer};
use opentelemetry::trace::{Span, Tracer};
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::fmt;

/// When dropped, ensures tracer provider and span exporter is shut down
/// correctly.
///
/// Guards cannot be cloned, as the clone would shut down the provider when
/// dropped.
///
/// Note that if the program aborts without unwinding, the guard is never
/// dropped, and the provider isn't shut down. This can happen with an `abort`
/// panic strategy, or by calling e.g. [`std::process::abort`] or
/// [`std::process::exit`]. In this case, the provider must be shut down
/// manually using a shutdown handle.
///
/// # Why is a guard even needed?
///
/// (This section is implementation detail and may change.)
///
/// The eagle-eyed reader may have spotted that the guards contain the
/// respective SDK providers, which also shut down when dropped. So why is a
/// guard needed? The problem is that the providers are `Clone`-able, and
/// internally have an `Arc`. So it's very easy to accidentally create a clone
/// that prevents shutdown when using layers.
///
/// For traces, the `SdkTracer` retains the provider, the `OpenTelemetryLayer`
/// retains the tracer, and the global tracing subscriber retains the layer. In
/// theory, it's possible to unset/reset the global tracing subscriber to cause
/// the provider to drop. Unfortunately, that's tricky to get right, because the
/// tracing subscriber contains other layers like logging or stdout.
///
/// For logs, the `SdkLogger` is analogue to the tracer, and has the same issue.
///
/// For metrics, this problem doesn't occur because the `Meter` doesn't clone
/// the provider. Instead, the "pipes" are cloned, so drop works as intended.
/// But we still implement drop ourselves to output errors to stderr.
#[derive(Debug)]
#[repr(transparent)]
#[must_use = "trace exporter is shut down when dropped"]
pub struct TracesGuard(pub(super) Option<SdkTracerProvider>);

impl TracesGuard {
    /// Returns a tracer if traces are enabled.
    ///
    /// Otherwise, [`opentelemetry::trace::noop::NoopTracerProvider`] can be
    /// used. Note that each tracer provider returns a separate tracer type, so
    /// it's quite difficult to switch on the tracer even without the guard:
    ///
    /// ```rust,compile_fail
    /// use opentelemetry_sdk::trace::{SdkTracerProvider, SdkTracer};
    /// use opentelemetry::trace::noop::{NoopTracerProvider, NoopTracer};
    /// use opentelemetry::trace::TracerProvider as _;
    ///
    /// fn sdk_tracer() -> SdkTracer {
    ///     let provider = SdkTracerProvider::builder().build();
    ///     provider.tracer("foo")
    /// }
    ///
    /// fn noop_tracer() -> NoopTracer {
    ///     let provider = NoopTracerProvider::new();
    ///     provider.tracer("foo")
    /// }
    ///
    /// let use_sdk = true;
    /// // `if` and `else` have incompatible types (E0308)
    /// let tracer = if use_sdk {
    ///     sdk_tracer()
    /// } else {
    ///     noop_tracer()
    /// };
    /// ```
    #[inline]
    pub fn tracer(
        &self,
        name: &'static str,
    ) -> Option<
        impl Tracer<Span = impl Span + ObjectSafeSpan + Send + Sync + 'static + use<>>
        + ObjectSafeTracer
        + Send
        + Sync
        + 'static
        + use<>,
    > {
        use opentelemetry::trace::TracerProvider as _;
        self.0.as_ref().map(|provider| provider.tracer(name))
    }

    /// Creates a completely disabled traces guard.
    ///
    /// Usually, a guard would be created via
    /// [`TracesConfig::guard()`](super::config::TracesConfig::guard). However,
    /// for testing scenarios it may be more convenient to use this method.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self(None)
    }

    /// Create a handle to manually shut down the provider.
    ///
    /// See [`TracesGuard`](crate::exporters::traces::TracesGuard) for more
    /// information.
    pub fn shutdown_handle(&self) -> TracesShutdownHandle {
        TracesShutdownHandle(self.0.clone())
    }
}

impl Drop for TracesGuard {
    fn drop(&mut self) {
        // see note on `TracesGuard`. we implement this to ensure the provider
        // is shut down even if a global tracing subscriber layer is using it,
        // and also to output any errors to stderr.
        if let Some(provider) = self.0.take()
            && let Err(e) = provider.shutdown()
            && !matches!(e, OTelSdkError::AlreadyShutdown)
        {
            eprintln!("failed to shut down tracer provider: {e}")
        }
    }
}

/// Handle to manually shut down the provider.
#[derive(Clone)]
pub struct TracesShutdownHandle(Option<SdkTracerProvider>);

impl TracesShutdownHandle {
    /// Shut down the provider.
    pub fn shutdown(&self) -> Result<(), impl ShutdownError + Send + Sync + 'static + use<>> {
        self.0.as_ref().map_or(Ok(()), SdkTracerProvider::shutdown)
    }
}

impl fmt::Debug for TracesShutdownHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TracesShutdownHandle")
            .finish_non_exhaustive()
    }
}
