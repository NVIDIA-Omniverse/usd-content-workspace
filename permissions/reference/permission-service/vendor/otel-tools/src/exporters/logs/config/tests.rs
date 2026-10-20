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

#![expect(clippy::bool_assert_comparison, reason = "easier to read")]
use super::{LogsConfig, logs};
use crate::testing::unsafe_env;

#[test]
fn logs_config_debug_env_empty() {
    const EXPECTED: &str = r#"LogsConfig {
    sdk_disabled: None,
    exporter: None,
    protocol: None,
    endpoint: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = LogsConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn logs_config_debug_env_logs() {
    const EXPECTED: &str = r#"LogsConfig {
    sdk_disabled: Some(
        false,
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
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": "http://localhost:5555",
    }

    let config = LogsConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn logs_config_debug_env_otlp_grpc() {
    const EXPECTED: &str = r#"LogsConfig {
    sdk_disabled: Some(
        false,
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
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:5555",
    }

    let config = LogsConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn logs_config_debug_env_otlp_http() {
    const EXPECTED: &str = r#"LogsConfig {
    sdk_disabled: Some(
        false,
    ),
    exporter: Some(
        Otlp,
    ),
    protocol: Some(
        HttpJson,
    ),
    endpoint: Some(
        "http://localhost:5555/v1/logs",
    ),
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "false",
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_PROTOCOL": "http/json",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:5555",
    }

    let config = LogsConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn logs_config_debug_for_test() {
    const EXPECTED: &str = r#"LogsConfig {
    sdk_disabled: Some(
        true,
    ),
    exporter: None,
    protocol: None,
    endpoint: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = LogsConfig::disabled_for_test();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn logs_config_otlp_enabled_sdk_enabled() {
    fn mk(exporter: logs::OtelLogsExporter) -> LogsConfig {
        LogsConfig {
            sdk_disabled: None,
            exporter: Some(exporter),
            protocol: None,
            endpoint: None,
        }
    }

    let config = mk(logs::OtelLogsExporter::None);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Console);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Otlp);
    assert_eq!(config.is_disabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::OtlpAndConsole);
    assert_eq!(config.is_disabled(), false, "{config:#?}");
    let config = LogsConfig {
        sdk_disabled: None,
        exporter: None,
        protocol: None,
        endpoint: None,
    };
    assert_eq!(config.is_disabled(), false, "{config:#?}");
}

#[test]
fn logs_config_otlp_enabled_sdk_disabled() {
    fn mk(exporter: logs::OtelLogsExporter) -> LogsConfig {
        LogsConfig {
            sdk_disabled: Some(true),
            exporter: Some(exporter),
            protocol: None,
            endpoint: None,
        }
    }

    let config = mk(logs::OtelLogsExporter::None);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Console);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Otlp);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::OtlpAndConsole);
    assert_eq!(config.is_disabled(), true, "{config:#?}");
    let config = LogsConfig {
        sdk_disabled: Some(true),
        exporter: None,
        protocol: None,
        endpoint: None,
    };
    assert_eq!(config.is_disabled(), true, "{config:#?}");
}
