// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

pub mod accumulator;
pub mod config;
pub mod flush;
pub mod metrics;
pub mod observer;

use std::sync::Arc;
use std::time::Duration;

pub use config::SyntheticDirEventsConfig;

use crate::pubsub::types::AnyConsumer;

use accumulator::Accumulator;
use flush::Flusher;
use metrics::Metrics;
use observer::SyntheticDirEventsObserver;
use storage_core::layer::Layer;

/// Synthetic directory events middleware.
///
/// This is the single entry point for the feature: decorating a consumer with it wires up
/// everything. [`Layer::layer`] builds the accumulator, spawns the flush loop that periodically
/// drains it and republishes synthetic `dir_created` / `dir_deleted` events through the same
/// downstream consumer, and returns the observer that tees file events into that accumulator.
/// There is nothing else to start — the component is operational once layered.
pub struct SyntheticDirEvents {
    metrics: Arc<Metrics>,
    flush_interval: Duration,
}

impl SyntheticDirEvents {
    /// Build the feature from configuration, returning `None` when disabled.
    pub fn from_config(config: SyntheticDirEventsConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(Self {
            metrics: Arc::new(Metrics::new()),
            flush_interval: config.flush_interval.into(),
        })
    }
}

impl Layer<AnyConsumer> for SyntheticDirEvents {
    type Service = SyntheticDirEventsObserver<AnyConsumer>;

    fn layer(&self, inner: AnyConsumer) -> Self::Service {
        let accumulator = Accumulator::new();
        let flusher = Flusher::new(inner.clone(), self.metrics.clone());
        tokio::spawn(flusher.run(accumulator.clone(), self.flush_interval));
        SyntheticDirEventsObserver::new(accumulator, self.metrics.clone(), inner)
    }
}
