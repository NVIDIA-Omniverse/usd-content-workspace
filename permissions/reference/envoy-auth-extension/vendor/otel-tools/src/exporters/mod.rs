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

//! OpenTelemetry Protocol (OTLP) exporters. (Requires feature `otlp`.)
//!
//! This module should not exist. However, `opentelemetry-sdk` is all over the
//! place with regards to configuration:
//!
//! * [`OTEL_SDK_DISABLED`](https://github.com/open-telemetry/opentelemetry-rust/issues/1936)
//! * [`OTEL_EXPORTER_OTLP_PROTOCOL`](https://github.com/open-telemetry/opentelemetry-rust/issues/1983)
//! * <https://github.com/open-telemetry/opentelemetry-rust/issues/1225>
//! * <https://github.com/open-telemetry/opentelemetry-rust/issues/1324>
//! * <https://github.com/open-telemetry/opentelemetry-rust/issues/1892>
//! * etc
//!
//! Additionally, setting up the OTLP exporters is quite verbose, and they do
//! not return guards that shut down the exporters.
//!
//! However, it isn't required to use these functions. It's possible to set up
//! the OTLP exporters using the crates, but note that not all environment
//! variables are respected by `opentelemetry-sdk`/`opentelemetry-otlp`.
pub mod logs;
pub mod metrics;
pub mod traces;

use opentelemetry_sdk::error::OTelSdkError;

/// Implemented for shutdown handle errors.
pub trait ShutdownError: std::error::Error {
    /// True if the error indicates the provider is already shut down.
    fn already_shutdown(&self) -> bool;
}

impl ShutdownError for OTelSdkError {
    #[inline]
    fn already_shutdown(&self) -> bool {
        matches!(self, OTelSdkError::AlreadyShutdown)
    }
}
