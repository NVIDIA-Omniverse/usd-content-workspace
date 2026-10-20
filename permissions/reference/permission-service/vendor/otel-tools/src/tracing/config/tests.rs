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
use super::{EnvFilterSource, TracingConfig, logs, subscriber};
use crate::testing::unsafe_env;

#[test]
fn tracing_config_debug_env_empty() {
    const EXPECTED: &str = r#"TracingConfig {
    sdk_disabled: None,
    exporter: None,
    rust_log: None,
    otlp_logs_env_filter: None,
    otlp_traces_env_filter: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = TracingConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn tracing_config_debug_for_test() {
    const EXPECTED: &str = r#"TracingConfig {
    sdk_disabled: Some(
        true,
    ),
    exporter: None,
    rust_log: None,
    otlp_logs_env_filter: None,
    otlp_traces_env_filter: None,
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let config = TracingConfig::disabled_for_test();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

#[test]
fn tracing_config_debug_env_filled() {
    const EXPECTED: &str = r#"TracingConfig {
    sdk_disabled: Some(
        false,
    ),
    exporter: Some(
        Console,
    ),
    rust_log: Some(
        FilterDirectives {
            dirs: "error",
            valid: true,
        },
    ),
    otlp_logs_env_filter: Some(
        FilterDirectives {
            dirs: "warn",
            valid: true,
        },
    ),
    otlp_traces_env_filter: Some(
        FilterDirectives {
            dirs: "info",
            valid: true,
        },
    ),
}"#;
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "false",
        "OTEL_LOGS_EXPORTER": "console",
        "RUST_LOG": "error",
        "OTEL_RUST_OTLP_LOGS_FILTER": "warn",
        "OTEL_RUST_OTLP_TRACES_FILTER": "info",
    }

    let config = TracingConfig::from_env();
    let actual = format!("{config:#?}");
    assert_eq!(actual, EXPECTED);
}

fn tracing_config_empty() -> TracingConfig {
    TracingConfig {
        sdk_disabled: None,
        exporter: None,
        rust_log: None,
        otlp_logs_env_filter: None,
        otlp_traces_env_filter: None,
    }
}

#[test]
fn tracing_config_console_enabled_sdk_enabled() {
    fn mk(exporter: logs::OtelLogsExporter) -> TracingConfig {
        TracingConfig {
            sdk_disabled: None,
            exporter: Some(exporter),
            ..tracing_config_empty()
        }
    }

    let config = mk(logs::OtelLogsExporter::None);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Otlp);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Console);
    assert_eq!(config.console_enabled(), true, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::OtlpAndConsole);
    assert_eq!(config.console_enabled(), true, "{config:#?}");
    let config = TracingConfig {
        sdk_disabled: None,
        exporter: None,
        ..tracing_config_empty()
    };
    assert_eq!(config.console_enabled(), true, "{config:#?}");
}

#[test]
fn tracing_config_console_enabled_sdk_disabled() {
    fn mk(exporter: logs::OtelLogsExporter) -> TracingConfig {
        TracingConfig {
            sdk_disabled: Some(true),
            exporter: Some(exporter),
            ..tracing_config_empty()
        }
    }

    let config = mk(logs::OtelLogsExporter::None);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Otlp);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::Console);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = mk(logs::OtelLogsExporter::OtlpAndConsole);
    assert_eq!(config.console_enabled(), false, "{config:#?}");
    let config = TracingConfig {
        sdk_disabled: Some(true),
        exporter: None,
        ..tracing_config_empty()
    };
    assert_eq!(config.console_enabled(), false, "{config:#?}");
}

#[test]
fn console_filter() {
    let config = TracingConfig {
        rust_log: Some(subscriber::FilterDirectives::new("info")),
        ..tracing_config_empty()
    };

    let filter = config.console_filter();
    assert_eq!(
        filter,
        EnvFilterSource {
            dirs: Some("info"),
            src: "RUST_LOG",
        }
    );
}

#[test]
fn otlp_traces_filter() {
    let config = TracingConfig {
        otlp_traces_env_filter: Some(subscriber::FilterDirectives::new("info")),
        ..tracing_config_empty()
    };

    let filter = config.otlp_traces_filter();
    assert_eq!(
        filter,
        EnvFilterSource {
            dirs: Some("info"),
            src: "OTEL_RUST_OTLP_TRACES_FILTER",
        }
    );
}

#[test]
fn otlp_logs_filter() {
    let config = TracingConfig {
        otlp_logs_env_filter: Some(subscriber::FilterDirectives::new("info")),
        ..tracing_config_empty()
    };

    let filter = config.otlp_logs_filter();
    assert_eq!(
        filter,
        EnvFilterSource {
            dirs: Some("info"),
            src: "OTEL_RUST_OTLP_LOGS_FILTER",
        }
    );
}
