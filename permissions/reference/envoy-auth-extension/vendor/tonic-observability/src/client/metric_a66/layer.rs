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

use super::builder::TonicMetricLayerBuilder;
use super::metrics::Metrics;
use super::service::TonicMetricService;
use crate::common::target_fn::TargetFn;
use crate::rpc_map::RpcMap;
use opentelemetry::metrics::Meter;
use tower_layer::Layer;

/// Tonic client metric layer.
#[derive(Debug, Clone)]
pub struct TonicMetricLayer {
    pub(super) meter: Meter,
    pub(super) rpc_map: RpcMap,
    pub(super) target_fn: TargetFn,
}

impl TonicMetricLayer {
    /// Create a builder for the client metric layer with an empty RPC map.
    ///
    /// Use this with `tower`'s `ServiceBuilder` to construct services
    /// instrumented with gRPC A66-based metrics.
    ///
    /// See [`RpcMap::builder()`] for what an RPC map is and why to use them.
    ///
    /// ```rust
    /// use tonic_observability::client::metric_a66::TonicMetricLayer;
    /// use tonic_observability::rpc_map::RpcMap;
    /// // see `RpcMap::builder()` for a more complete example
    /// let rpc_map = RpcMap::builder().build();
    /// // this is an example of how to get a meter
    /// let meter = opentelemetry::global::meter("my-meter");
    ///
    /// let layer = TonicMetricLayer::builder(meter)
    ///     .with_rpc_map(rpc_map)
    ///     .build();
    /// ```
    #[inline]
    pub fn builder(meter: Meter) -> TonicMetricLayerBuilder {
        TonicMetricLayerBuilder::new(meter)
    }
}

impl<Svc> Layer<Svc> for TonicMetricLayer {
    type Service = TonicMetricService<Svc>;

    fn layer(&self, inner: Svc) -> Self::Service {
        let rpc_map = self.rpc_map.clone();

        let Metrics {
            call_metrics,
            req_metrics,
            resp_metrics,
        } = Metrics::new(&self.meter);

        TonicMetricService {
            inner,
            rpc_map,
            target_fn: self.target_fn.clone(),
            call_metrics,
            req_metrics,
            resp_metrics,
        }
    }
}
