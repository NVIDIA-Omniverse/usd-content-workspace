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
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use std::borrow::Cow;
use std::fmt;

/// When dropped, ensures logger provider and log exporter is shut down
/// correctly.
///
/// See [`TracesGuard`](crate::exporters::traces::TracesGuard) for more
/// information.
#[derive(Debug)]
#[repr(transparent)]
#[must_use = "log exporter is shut down when dropped"]
pub struct LogsGuard(pub(super) Option<SdkLoggerProvider>);

impl LogsGuard {
    /// Returns a logger if logs are enabled.
    ///
    /// Otherwise, [`opentelemetry::logs::NoopLoggerProvider`] can be used. Note
    /// that each logger provider returns a separate logger type, so it's quite
    /// difficult to switch on the logger even without the guard:
    ///
    /// ```rust,compile_fail
    /// use opentelemetry_sdk::logs::{SdkLoggerProvider, SdkLogger};
    /// use opentelemetry::logs::NoopLoggerProvider;
    /// use opentelemetry::logs::LoggerProvider;
    ///
    /// fn sdk_logger() -> SdkLogger {
    ///     let provider = SdkLoggerProvider::builder().build();
    ///     provider.logger("foo")
    /// }
    ///
    /// fn noop_logger() -> <NoopLoggerProvider as LoggerProvider>::Logger {
    ///     let provider = NoopLoggerProvider::new();
    ///     provider.logger("foo")
    /// }
    ///
    /// let use_sdk = true;
    /// // `if` and `else` have incompatible types (E0308)
    /// let logger = if use_sdk {
    ///     sdk_logger()
    /// } else {
    ///     noop_logger()
    /// };
    /// ```
    #[inline]
    pub fn logger<S>(
        &self,
        name: S,
    ) -> Option<
        impl Logger<LogRecord = impl LogRecord + Send + Sync + 'static + use<S>>
        + Send
        + Sync
        + 'static
        + use<S>,
    >
    where
        S: Into<Cow<'static, str>>,
    {
        self.0.as_ref().map(|provider| provider.logger(name))
    }

    /// Returns a logger provider if logs are enabled.
    ///
    /// This is required to construct some logging layers.
    #[inline]
    pub fn provider(
        &self,
    ) -> Option<
        &(
             impl LoggerProvider<
            Logger = impl Logger<LogRecord = impl LogRecord + Send + Sync + 'static + use<>>
                     + Send
                     + Sync
                     + 'static
                     + use<>,
        >
             + Send
             + Sync
             + 'static
             + use<>
         ),
    > {
        self.0.as_ref()
    }

    /// Creates a completely disabled logs guard.
    ///
    /// Usually, a guard would be created via
    /// [`LogsConfig::guard`](super::config::LogsConfig::guard). However, for
    /// testing scenarios it may be more convenient to use this method.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self(None)
    }

    /// Create a handle to manually shut down the provider.
    ///
    /// See [`TracesGuard`](crate::exporters::traces::TracesGuard) for more
    /// information.
    pub fn shutdown_handle(&self) -> LogsShutdownHandle {
        LogsShutdownHandle(self.0.clone())
    }
}

impl Drop for LogsGuard {
    fn drop(&mut self) {
        // see note on `TracesGuard`. we implement this to ensure the provider
        // is shut down even if a global tracing subscriber layer is using it,
        // and also to output any errors to stderr.
        if let Some(provider) = self.0.take() {
            // in theory, we don't need to call `force_flush()` before
            // `shutdown()`. however, `shutdown()` currently ignores errors, so
            // we wouldn't know if an error occurred, and why the data wasn't
            // exported. this is unlike metrics or traces.
            // https://github.com/open-telemetry/opentelemetry-rust/blob/v0.31.0/opentelemetry-sdk/src/logs/logger_provider.rs#L100
            // https://github.com/open-telemetry/opentelemetry-rust/blob/v0.31.0/opentelemetry-sdk/src/logs/log_processor_with_async_runtime.rs#L191
            if let Err(e) = provider.force_flush()
                && !matches!(e, OTelSdkError::AlreadyShutdown)
            {
                eprintln!("failed to flush logger provider: {e}");
            }
            if let Err(e) = provider.shutdown()
                && !matches!(e, OTelSdkError::AlreadyShutdown)
            // start-cov-exclude [branch]: see above, branch is never taken due
            // to `shutdown()` of the log processor ignoring errors
            {
                eprintln!("failed to shut down logger provider: {e}");
            }
            // end-cov-exclude [branch]
        }
    }
}

/// Handle to manually shut down the provider.
#[derive(Clone)]
pub struct LogsShutdownHandle(Option<SdkLoggerProvider>);

impl LogsShutdownHandle {
    /// Shut down the provider.
    pub fn shutdown(&self) -> Result<(), impl ShutdownError + Send + Sync + 'static + use<>> {
        self.0.as_ref().map_or(Ok(()), |provider| {
            // see `LogsGuard::drop()` on why `force_flush()` is needed
            let a = provider.force_flush();
            let b = provider.shutdown();
            a.and(b)
        })
    }
}

impl fmt::Debug for LogsShutdownHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogsShutdownHandle").finish_non_exhaustive()
    }
}
