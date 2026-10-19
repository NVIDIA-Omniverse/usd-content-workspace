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

use super::{LogsConfig, LogsGuard};
use crate::testing::unsafe_env;

macro_rules! assert_disabled {
    ($guard:ident) => {
        assert!($guard.0.is_none());
    };
}

#[test]
fn disabled_via_config() {
    let config = LogsConfig::disabled_for_test();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_otel_sdk_disabled() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SDK_DISABLED": "true",
    }

    let config = LogsConfig::from_env();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_otel_traces_exporter() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "none",
    }

    let config = LogsConfig::from_env();
    let resource = opentelemetry_sdk::Resource::builder_empty().build();

    let guard = config.guard().with_resource(&resource).build();
    assert_disabled!(guard);
}

#[test]
fn disabled_via_guard() {
    let guard = LogsGuard::disabled_for_test();
    assert_disabled!(guard);
}

#[test]
fn shutdown_handle_debug() {
    let guard = LogsGuard::disabled_for_test();
    let handle = guard.shutdown_handle();
    assert_eq!(format!("{handle:?}"), "LogsShutdownHandle { .. }");
}
