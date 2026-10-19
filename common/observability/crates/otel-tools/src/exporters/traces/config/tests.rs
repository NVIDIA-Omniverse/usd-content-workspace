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

use super::TracesConfig;
use crate::testing::unsafe_env;

#[test]
fn traces_config_debug_env_empty() {
    const EXPECTED: &str = r#"TracesConfig {
    sdk_disabled: None,
    propagators: None,
    exporter: None,
    protocol: None,
    endpoint: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = TracesConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn traces_config_debug_env_traces() {
    const EXPECTED: &str = r#"TracesConfig {
    sdk_disabled: Some(
        false,
    ),
    propagators: Some(
        OtelPropagators {
            trace_context: true,
        },
    ),
    exporter: Some(
        Otlp,
    ),
    protocol: Some(
        Grpc,
    ),
    endpoint: Some(
        "http://localhost:5555",
    ),
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "false",
        "OTEL_PROPAGATORS": "tracecontext",
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": "http://localhost:5555",
    }

    let config = TracesConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn traces_config_debug_env_otlp_grpc() {
    const EXPECTED: &str = r#"TracesConfig {
    sdk_disabled: Some(
        false,
    ),
    propagators: Some(
        OtelPropagators {
            trace_context: true,
        },
    ),
    exporter: Some(
        Otlp,
    ),
    protocol: Some(
        Grpc,
    ),
    endpoint: Some(
        "http://localhost:5555",
    ),
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "false",
        "OTEL_PROPAGATORS": "tracecontext",
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:5555",
    }

    let config = TracesConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn traces_config_debug_env_otlp_http() {
    const EXPECTED: &str = r#"TracesConfig {
    sdk_disabled: Some(
        false,
    ),
    propagators: Some(
        OtelPropagators {
            trace_context: true,
        },
    ),
    exporter: Some(
        Otlp,
    ),
    protocol: Some(
        HttpJson,
    ),
    endpoint: Some(
        "http://localhost:5555/v1/traces",
    ),
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "false",
        "OTEL_PROPAGATORS": "tracecontext",
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_PROTOCOL": "http/json",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:5555",
    }

    let config = TracesConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn traces_config_debug_for_test() {
    const EXPECTED: &str = r#"TracesConfig {
    sdk_disabled: Some(
        true,
    ),
    propagators: None,
    exporter: None,
    protocol: None,
    endpoint: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = TracesConfig::disabled_for_test();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}
