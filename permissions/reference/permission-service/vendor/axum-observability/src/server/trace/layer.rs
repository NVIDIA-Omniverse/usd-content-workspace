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

use super::builder::TowerTraceLayerBuilder;
use super::service::TowerTraceService;
use super::span::MakeSpan;
use tower_layer::Layer;

/// Tower server trace layer.
#[derive(Debug, Clone)]
pub struct TowerTraceLayer {
    pub(super) make_span: MakeSpan,
}

impl TowerTraceLayer {
    /// Create a builder for the server trace layer.
    ///
    /// Use this with `tower`'s `ServiceBuilder` to construct services instrumented
    /// with HTTP spans.
    ///
    /// ```rust
    /// use axum_observability::server::trace::TowerTraceLayer;
    /// let layer = TowerTraceLayer::builder().build();
    /// ```
    #[inline]
    pub fn builder() -> TowerTraceLayerBuilder {
        TowerTraceLayerBuilder {
            make_span: super::MAKE_SPAN,
        }
    }
}

impl<Inner> Layer<Inner> for TowerTraceLayer {
    type Service = TowerTraceService<Inner>;

    #[inline]
    fn layer(&self, inner: Inner) -> Self::Service {
        TowerTraceService {
            make_span: self.make_span,
            inner,
        }
    }
}
