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

use super::layer::TowerTraceLayer;
use super::span::MakeSpan;

/// The builder for [`TowerTraceLayer`].
#[derive(Debug)]
pub struct TowerTraceLayerBuilder {
    pub(super) make_span: MakeSpan,
}

impl TowerTraceLayerBuilder {
    /// Set a custom span name and code location.
    ///
    /// Use [`make_span!`](crate::server::trace::make_span) to create this argument.
    ///
    /// ```rust
    /// use axum_observability::server::trace::{TowerTraceLayer, make_span};
    /// TowerTraceLayer::builder().with_span(make_span!());
    /// ```
    ///
    /// As with [`tracing::span!`], the span will have call site information of
    /// the macro invocation site. For example, if you call this macro in a file
    /// `crates/my_server/src/observability.rs`, then:
    ///
    /// * The span's code file path is `crates/my_server/src/observability.rs`.
    /// * The span's code module name is `my_server::observability`.
    /// * The span's target is also `my_server::observability`. This is used for
    ///   filtering via e.g. `RUST_LOG`.
    ///
    /// The target name can be overwritten via `make_span!("my target")`.
    #[inline]
    pub fn with_span(mut self, make_span: MakeSpan) -> Self {
        self.make_span = make_span;
        self
    }

    /// Build the tower server trace layer.
    #[inline]
    pub fn build(self) -> TowerTraceLayer {
        let Self { make_span } = self;
        TowerTraceLayer { make_span }
    }
}
