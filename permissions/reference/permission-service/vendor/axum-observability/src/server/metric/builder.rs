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

use super::layer::TowerMetricLayer;
use opentelemetry::metrics::Meter;

/// The builder returned by [`TowerMetricLayer::builder()`].
#[derive(Debug)]
pub struct TowerMetricLayerBuilder {
    meter: Meter,
}

impl TowerMetricLayerBuilder {
    pub(super) fn new(meter: Meter) -> Self {
        Self { meter }
    }

    /// Build the server metric layer.
    #[inline]
    pub fn build(self) -> TowerMetricLayer {
        let Self { meter } = self;
        TowerMetricLayer { meter }
    }
}
