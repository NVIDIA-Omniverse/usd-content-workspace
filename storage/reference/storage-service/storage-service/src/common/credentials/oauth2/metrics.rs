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

use std::fmt::Debug;

use storage_core_util::observability::metrics::{metric_base, BoundTimerMetric};
use storage_service_metrics::{status::http::ResultMetrics, utils::BindExt};

/// Credentials provider metrics.
pub struct Metrics {
    pub get_token_latency: BoundTimerMetric,
    pub get_token_results: ResultMetrics,
}

impl Metrics {
    pub fn new(name: impl Into<String>) -> Self {
        metric_base!(
            static GET_TOKEN_LATENCY_BASE: TimerMetric,
            name = "oauth2_client_credentials_provider_get_token_latency",
            { pod, name }
        );
        metric_base!(
            static GET_TOKEN_RESULTS_BASE: UIntCounter,
            name = "oauth2_client_credentials_provider_get_token_results",
            description = "OAuth2 GetToken operation results",
            { pod, name, result }
        );

        let name = name.into();
        Self {
            get_token_latency: GET_TOKEN_LATENCY_BASE.bind_with_default_attributes([("name", name.clone())]),
            get_token_results: ResultMetrics::new_with_bind(|result| {
                GET_TOKEN_RESULTS_BASE.bind_with_default_attributes([("result", result.into()), ("name", name.clone())])
            }),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}
