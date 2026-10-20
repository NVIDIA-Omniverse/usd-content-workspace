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

//! [`tracing`] layers for tracing subscribers. These are different than `tower`
//! layers aka. middleware.
//!
//! * [`LoggingLayer`] is a structured JSON logging layer.
//! * [`OpenTelemetryLayer`] is a re-export from the [`tracing_opentelemetry`]
//!   crate.

// allow types to be named if needed, but don't advertise this
#[doc(hidden)]
pub mod logging;

/// A layer that propagates [`tracing`] spans to Otel.
///
///
pub use ::tracing_opentelemetry::OpenTelemetryLayer;
pub use logging::LoggingLayer;
