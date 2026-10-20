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

//! Client trace layer.
//!
//! Spans follow the [OpenTelemetry semantic conventions for RPC spans].
//!
//! See [`TonicTraceLayer::builder()`] to get started.
//!
//! [OpenTelemetry semantic conventions for RPC spans]: https://github.com/open-telemetry/semantic-conventions/blob/v1.40.0/docs/rpc/rpc-spans.md
mod builder;
mod call;
mod layer;
mod response_body;
mod service;
pub(crate) mod span;

pub use builder::TonicTraceLayerBuilder;
pub use call::TonicTraceResponse;
pub use layer::TonicTraceLayer;
pub use response_body::TonicTraceResponseBody;
pub use service::TonicTraceService;
#[doc(hidden)]
pub use span::MakeSpan;

/// Define a custom trace span.
///
/// See [`TonicTraceLayerBuilder::with_span`].
#[doc(inline)]
pub use crate::__client_trace_otel_make_span as make_span;

const MAKE_SPAN: MakeSpan = crate::__client_trace_otel_make_span!("client");
