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

use super::builder::TonicTraceLayerBuilder;
use super::service::TonicTraceService;
use super::span::MakeSpan;
use crate::common::target_fn::TargetFn;
use tower_layer::Layer;

/// Tonic client trace layer.
#[derive(Debug, Clone)]
pub struct TonicTraceLayer {
    pub(super) make_span: MakeSpan,
    pub(super) target_fn: TargetFn,
}

impl TonicTraceLayer {
    /// Create a builder for the client trace layer.
    ///
    /// Use this with `tower`'s `ServiceBuilder` to construct clients
    /// instrumented with gRPC spans.
    ///
    /// ```rust
    /// use tonic_observability::client::trace::TonicTraceLayer;
    /// let layer = TonicTraceLayer::builder().build();
    /// ```
    #[inline]
    pub fn builder() -> TonicTraceLayerBuilder {
        TonicTraceLayerBuilder::new()
    }
}

impl<Inner> Layer<Inner> for TonicTraceLayer {
    type Service = TonicTraceService<Inner>;

    #[inline]
    fn layer(&self, inner: Inner) -> Self::Service {
        TonicTraceService {
            make_span: self.make_span,
            target_fn: self.target_fn.clone(),
            inner,
        }
    }
}
