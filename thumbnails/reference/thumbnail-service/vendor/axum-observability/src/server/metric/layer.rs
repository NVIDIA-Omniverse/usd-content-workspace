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

use super::builder::TowerMetricLayerBuilder;
use super::metrics::Metrics;
use super::service::TowerMetricService;
use opentelemetry::metrics::Meter;
use tower_layer::Layer;

/// Tower server metric layer.
#[derive(Debug, Clone)]
pub struct TowerMetricLayer {
    pub(super) meter: Meter,
}

impl TowerMetricLayer {
    /// Create a builder for the server metric layer.
    ///
    /// Use this with `tower`'s `ServiceBuilder` to construct services
    /// instrumented with HTTP metrics.
    ///
    /// ```rust
    /// use axum_observability::server::metric::TowerMetricLayer;
    /// // this is an example of how to get a meter
    /// let meter = opentelemetry::global::meter("my-meter");
    ///
    /// let layer = TowerMetricLayer::builder(meter).build();
    /// ```
    #[inline]
    pub fn builder(meter: Meter) -> TowerMetricLayerBuilder {
        TowerMetricLayerBuilder::new(meter)
    }
}

impl<Svc> Layer<Svc> for TowerMetricLayer {
    type Service = TowerMetricService<Svc>;

    fn layer(&self, inner: Svc) -> Self::Service {
        let Metrics {
            req_metrics,
            resp_metrics,
        } = Metrics::new(&self.meter);

        TowerMetricService {
            inner,
            req_metrics,
            resp_metrics,
        }
    }
}
