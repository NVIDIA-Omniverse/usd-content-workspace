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
use crate::rpc_map::RpcMap;

/// The builder for [`TonicTraceLayer`].
#[derive(Debug)]
pub struct TonicTraceLayerBuilder {
    make_span: MakeSpan,
    rpc_map: RpcMap,
}

impl TonicTraceLayerBuilder {
    pub(super) fn new() -> Self {
        Self {
            make_span: MAKE_SPAN,
            rpc_map: RpcMap::default(),
        }
    }

    /// Set a custom span name and code location.
    ///
    /// Use [`make_span!`](crate::server::trace::make_span) to create this
    /// argument:
    ///
    /// ```rust
    /// use tonic_observability::server::trace::{TonicTraceLayer, make_span};
    /// TonicTraceLayer::builder().with_span(make_span!());
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

    /// Assign an [`RpcMap`] to the layer.
    ///
    /// It's recommended to provide an RPC map as knowing which RPCs to expect
    /// allows the layer to optimize some operations. An RPC map is also
    /// required to protect against high cardinality attacks against the
    /// `rpc.method` attribute.
    #[inline]
    pub fn with_rpc_map(mut self, rpc_map: RpcMap) -> Self {
        self.rpc_map = rpc_map;
        self
    }

    /// Build the tonic server trace layer.
    #[inline]
    pub fn build(self) -> TonicTraceLayer {
        let Self { make_span, rpc_map } = self;
        TonicTraceLayer { make_span, rpc_map }
    }
}
