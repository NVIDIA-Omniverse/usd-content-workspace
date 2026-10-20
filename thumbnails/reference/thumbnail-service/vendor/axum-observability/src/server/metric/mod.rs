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

//! Server metric layer.
//!
//! Metrics follow the [OpenTelemetry semantic conventions for HTTP metrics].
//!
//! See [`TowerMetricLayer::builder()`] to get started.
//!
//! [OpenTelemetry semantic conventions for HTTP metrics]: https://github.com/open-telemetry/semantic-conventions/blob/v1.40.0/docs/http/http-metrics.md
mod attributes;
mod builder;
mod handler;
mod layer;
mod metrics;
mod request_body;
mod response_body;
mod service;

pub use builder::TowerMetricLayerBuilder;
pub use handler::TowerMetricHandler;
pub use layer::TowerMetricLayer;
pub use request_body::TowerMetricRequestBody;
pub use response_body::TowerMetricResponseBody;
pub use service::TowerMetricService;
