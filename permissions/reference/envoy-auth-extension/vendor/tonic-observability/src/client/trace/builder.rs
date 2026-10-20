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

use super::MAKE_SPAN;
use super::layer::TonicTraceLayer;
use super::span::MakeSpan;
use crate::common::attributes::target_fallback;
use crate::common::target_fn::TargetFn;
use opentelemetry::StringValue;
use tonic::codegen::http;

/// The builder for [`TonicTraceLayer`].
#[derive(Debug)]
pub struct TonicTraceLayerBuilder {
    make_span: MakeSpan,
    target_fn: TargetFn,
}

impl TonicTraceLayerBuilder {
    pub(super) fn new() -> Self {
        Self {
            make_span: MAKE_SPAN,
            target_fn: TargetFn::new(target_fallback),
        }
    }

    /// Set a custom span name and code location.
    ///
    /// Use [`make_span!`](crate::client::trace::make_span) to create this
    /// argument:
    ///
    /// ```rust
    /// use tonic_observability::client::trace::{TonicTraceLayer, make_span};
    /// TonicTraceLayer::builder().with_span(make_span!());
    /// ```
    ///
    /// As with [`tracing::span!`], the span will have call site information of
    /// the macro invocation site. For example, if you call this macro in a file
    /// `crates/my_client/src/observability.rs`, then:
    ///
    /// * The span's code file path is `crates/my_client/src/observability.rs`.
    /// * The span's code module name is `my_client::observability`.
    /// * The span's target is also `my_client::observability`. This is used for
    ///   filtering via e.g. `RUST_LOG`.
    ///
    /// The target name can be overwritten via `make_span!("my target")`.
    #[inline]
    pub fn with_span(mut self, make_span: MakeSpan) -> Self {
        self.make_span = make_span;
        self
    }

    /// Specify a function that returns the `server.address` attribute value for
    /// a request.
    ///
    /// Because the OTel spec is a bit of a mess, we're treating this attribute
    /// as if it were `grpc.target` from the A66 gRPC metrics spec. This means
    /// that `server.port` cannot be specified; and the port should be included
    /// in the `server.address` value contrary to OTel guidance.
    ///
    /// The default implementation derives the address from the request URI's
    /// scheme and authority. However this only works for tonic clients built
    /// with an origin.
    #[inline]
    pub fn with_target_fn<F>(mut self, target_fn: F) -> Self
    where
        F: for<'a> Fn(&'a http::request::Parts) -> Option<StringValue>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        self.target_fn = TargetFn::new(target_fn);
        self
    }

    /// Build the tonic client trace layer.
    #[inline]
    pub fn build(self) -> TonicTraceLayer {
        let Self {
            make_span,
            target_fn,
        } = self;
        TonicTraceLayer {
            make_span,
            target_fn,
        }
    }
}
