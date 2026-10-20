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
use opentelemetry::metrics::{InstrumentProvider, Meter};
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use std::fmt;

/// When dropped, ensures the meter provider and metrics exporter is shut down
/// correctly.
///
/// See [`TracesGuard`](crate::exporters::traces::TracesGuard) for more
/// information.
#[derive(Debug)]
#[repr(transparent)]
#[must_use = "metric exporter is shut down when dropped"]
pub struct MetricsGuard(pub(super) Option<SdkMeterProvider>);

impl MetricsGuard {
    /// Returns a meter if metrics are enabled.
    #[inline]
    pub fn meter(&self, name: &'static str) -> Option<Meter> {
        use opentelemetry::metrics::MeterProvider as _;
        self.0.as_ref().map(|provider| provider.meter(name))
    }

    /// Returns a meter, or a no-op meter if metrics are disabled.
    #[inline]
    pub fn meter_or_noop(&self, name: &'static str) -> Meter {
        self.meter(name).unwrap_or_else(noop_meter)
    }

    /// Returns the meter provider if metrics are enabled.
    ///
    /// This is useful when the meter provider needs to be registered
    /// globally via [`opentelemetry::global::set_meter_provider`].
    #[inline]
    pub fn provider(&self) -> Option<&SdkMeterProvider> {
        self.0.as_ref()
    }

    /// Creates a completely disabled metrics guard.
    ///
    /// Usually, a guard would be created via
    /// [`MetricsConfig::guard()`](super::config::MetricsConfig::guard).
    /// However, for testing scenarios it may be more convenient to use this
    /// method.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self(None)
    }

    /// Create a handle to manually shut down the provider.
    ///
    /// See [`TracesGuard`](crate::exporters::traces::TracesGuard) for more
    /// information.
    pub fn shutdown_handle(&self) -> MetricsShutdownHandle {
        MetricsShutdownHandle(self.0.clone())
    }
}

impl Drop for MetricsGuard {
    fn drop(&mut self) {
        // see note on `TracesGuard`, we implement this to output any errors to
        // stderr.
        if let Some(provider) = self.0.take()
            && let Err(e) = provider.shutdown()
            && !matches!(e, OTelSdkError::AlreadyShutdown)
        {
            eprintln!("failed to shut down meter provider: {e}");
        }
    }
}

/// Handle to manually shut down the provider.
#[derive(Clone)]
pub struct MetricsShutdownHandle(Option<SdkMeterProvider>);

impl MetricsShutdownHandle {
    /// Shut down the provider.
    pub fn shutdown(&self) -> Result<(), impl ShutdownError + Send + Sync + 'static + use<>> {
        self.0.as_ref().map_or(Ok(()), SdkMeterProvider::shutdown)
    }
}

impl fmt::Debug for MetricsShutdownHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricsShutdownHandle")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct NoopMeter;

impl InstrumentProvider for NoopMeter {}

fn noop_meter() -> Meter {
    Meter::new(std::sync::Arc::new(NoopMeter))
}
