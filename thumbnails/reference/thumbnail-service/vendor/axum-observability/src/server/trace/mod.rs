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

//! Server trace layer.
//!
//! Spans follow the [OpenTelemetry semantic conventions for HTTP spans], events
//! follow the [OpenTelemetry semantic conventions for HTTP exceptions].
//!
//! See [`TowerTraceLayer::builder()`] to get started.
//!
//! [OpenTelemetry semantic conventions for HTTP spans]: https://github.com/open-telemetry/semantic-conventions/blob/v1.40.0/docs/http/http-spans.md
//! [OpenTelemetry semantic conventions for HTTP exceptions]: https://github.com/open-telemetry/semantic-conventions/blob/v1.40.0/docs/http/http-exceptions.md
mod builder;
mod handler;
mod layer;
mod response_body;
mod service;
pub(crate) mod span;

pub use builder::TowerTraceLayerBuilder;
pub use handler::TowerTraceResponse;
pub use layer::TowerTraceLayer;
pub use response_body::TowerTraceResponseBody;
pub use service::TowerTraceService;
#[doc(hidden)]
pub use span::MakeSpan;

/// Define a custom trace span.
///
/// See [`TowerTraceLayerBuilder::with_span`].
#[doc(inline)]
pub use crate::__server_trace_otel_make_span as make_span;

const MAKE_SPAN: MakeSpan = crate::__server_trace_otel_make_span!("server");
