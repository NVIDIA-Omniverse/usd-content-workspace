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

use snafu::{IntoError, Location, Snafu};

use crate::error::BoxError;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum Error {
    #[snafu(display("environment parse failed"))]
    EnvironmentParse {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to configure the global subscriber"))]
    SetGlobalSubscriber {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to configure a traces exporter"))]
    TracesExporterInit {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to configure a metrics exporter"))]
    MetricsExporterInit {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to configure a logger exporter"))]
    LogsExporterInit {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to start a prometheus server"))]
    PrometheusExporterInit {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl Error {
    #[track_caller]
    pub fn environment_parse(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::EnvironmentParseSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn set_global_subscriber(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::SetGlobalSubscriberSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn traces_exporter_init(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::TracesExporterInitSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn metrics_exporter_init(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::MetricsExporterInitSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn logs_exporter_init(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::LogsExporterInitSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn prometheus_exporter_init(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::PrometheusExporterInitSnafu.into_error(Box::new(source))
    }
}
