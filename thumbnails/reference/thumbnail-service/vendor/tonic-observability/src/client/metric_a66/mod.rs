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

//! Client metric layer with A66-based metric names.
//!
//! Metrics follow the [Google gRPC A66 OpenTelemetry Metrics].
//!
//! See [`TonicMetricLayer::builder()`] to get started.
//!
//! [Google gRPC A66 OpenTelemetry Metrics]: https://github.com/grpc/proposal/blob/master/A66-otel-stats.md
mod attributes;
mod builder;
mod call;
mod layer;
mod metrics;
mod request_body;
mod response_body;
mod service;

pub use builder::TonicMetricLayerBuilder;
pub use call::TonicMetricAttempt;
pub use layer::TonicMetricLayer;
pub use request_body::TonicMetricRequestBody;
pub use response_body::TonicMetricResponseBody;
pub use service::TonicMetricService;
