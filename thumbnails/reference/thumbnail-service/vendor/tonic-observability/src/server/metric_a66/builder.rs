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
use crate::rpc_map::RpcMap;
use opentelemetry::metrics::Meter;

/// The builder for [`TonicMetricLayer`].
#[derive(Debug)]
pub struct TonicMetricLayerBuilder {
    meter: Meter,
    rpc_map: RpcMap,
}

impl TonicMetricLayerBuilder {
    pub(super) fn new(meter: Meter) -> Self {
        Self {
            meter,
            rpc_map: RpcMap::default(),
        }
    }

    /// Assign an [`RpcMap`] to the layer.
    ///
    /// It's recommended to provide an RPC map as knowing which RPCs to expect
    /// allows the layer to optimize some operations. An RPC map is also
    /// required to protect against high cardinality attacks against the
    /// `grpc.method` attribute.
    #[inline]
    pub fn with_rpc_map(mut self, rpc_map: RpcMap) -> Self {
        self.rpc_map = rpc_map;
        self
    }

    /// Build the server metric layer.
    #[inline]
    pub fn build(self) -> TonicMetricLayer {
        let Self { meter, rpc_map } = self;
        TonicMetricLayer { meter, rpc_map }
    }
}
