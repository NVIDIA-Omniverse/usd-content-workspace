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

use super::layer::TonicMetricLayer;
use crate::common::attributes::target_fallback;
use crate::common::target_fn::TargetFn;
use crate::rpc_map::RpcMap;
use opentelemetry::StringValue;
use opentelemetry::metrics::Meter;
use tonic::codegen::http;

/// The builder for [`TonicMetricLayer`].
#[derive(Debug)]
pub struct TonicMetricLayerBuilder {
    meter: Meter,
    rpc_map: RpcMap,
    target_fn: TargetFn,
}

impl TonicMetricLayerBuilder {
    pub(super) fn new(meter: Meter) -> Self {
        Self {
            meter,
            rpc_map: RpcMap::default(),
            target_fn: TargetFn::new(target_fallback),
        }
    }

    /// Assign an [`RpcMap`] to the layer.
    ///
    /// It's recommended to provide an RPC map as knowing which RPCs to expect
    /// allows the layer to optimize some operations.
    #[inline]
    pub fn with_rpc_map(mut self, rpc_map: RpcMap) -> Self {
        self.rpc_map = rpc_map;
        self
    }

    /// Specify a function that returns the `grpc.target` attribute value for a
    /// request.
    ///
    /// Technically, the returned value should be a valid gRPC target, which is
    /// a URI.
    ///
    /// The default implementation derives the target from the request URI's
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

    /// Build the client metric layer.
    #[inline]
    pub fn build(self) -> TonicMetricLayer {
        let Self {
            meter,
            rpc_map,
            target_fn,
        } = self;
        TonicMetricLayer {
            meter,
            rpc_map,
            target_fn,
        }
    }
}
