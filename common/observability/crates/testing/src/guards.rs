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

use otel_tools::exporters::logs::{LogsConfig, LogsGuard};
use otel_tools::exporters::metrics::{MetricsConfig, MetricsGuard};
use otel_tools::exporters::traces::{TracesConfig, TracesGuard};
use otel_tools::resource::Resource;
use otel_tools::tracing::TracingConfig;

pub struct Guards {
    pub traces_guard: TracesGuard,
    pub logs_guard: LogsGuard,
    pub metrics_guard: MetricsGuard,
}

impl Guards {
    pub async fn new(resource: &Resource) -> Self {
        Self::make(resource, "info", false)
    }

    pub async fn custom(resource: &Resource, dirs: &str, target: bool) -> Self {
        Self::make(resource, dirs, target)
    }

    fn make(resource: &Resource, dirs: &str, target: bool) -> Self {
        let config = TracesConfig::from_env();
        let traces_guard = config.guard().with_resource(resource).build();

        let config = LogsConfig::from_env();
        let logs_guard = config.guard().with_resource(resource).build();

        let config = TracingConfig::from_env();
        config
            .subscriber()
            .with_default_env_filter(dirs)
            .with_logs(&logs_guard)
            .with_resource(resource)
            .with_traces(&traces_guard)
            .with_span_target(target)
            .build("test");

        let config = MetricsConfig::from_env();
        let metrics_guard = config.guard().with_resource(resource).build();

        Self {
            traces_guard,
            logs_guard,
            metrics_guard,
        }
    }
}
