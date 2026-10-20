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

use super::{
    OtelEnvVal as _, OtelExporterOtlpLogsEndpoint, OtelExporterOtlpProtocol, OtelLogsExporter,
};
use crate::testing::unsafe_env;

#[test]
fn otel_logs_exporter_from_env() {
    assert_eq!(
        OtelLogsExporter::from_env("otlp"),
        Ok(OtelLogsExporter::Otlp)
    );
    assert_eq!(
        OtelLogsExporter::from_env("console"),
        Ok(OtelLogsExporter::Console)
    );
    assert_eq!(
        OtelLogsExporter::from_env("otlp,console"),
        Ok(OtelLogsExporter::OtlpAndConsole)
    );
    assert_eq!(
        OtelLogsExporter::from_env("none"),
        Ok(OtelLogsExporter::None)
    );
    assert_eq!(
        OtelLogsExporter::from_env("foo"),
        Err("expected `otlp`, `console`, `otlp,console` or `none`")
    );
}

#[test]
fn otel_exporter_otlp_logs_endpoint_get_valid() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": "foo://invalid",
    }

    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::Grpc),
        Some(OtelExporterOtlpLogsEndpoint("foo://invalid".into())),
        "grpc"
    );
    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::HttpJson),
        Some(OtelExporterOtlpLogsEndpoint("foo://invalid/".into())),
        "http/json"
    );
    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::HttpProtobuf),
        Some(OtelExporterOtlpLogsEndpoint("foo://invalid/".into())),
        "http/protobuf"
    );
}

#[test]
fn otel_exporter_otlp_logs_endpoint_get_invalid() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": "foo!://invalid",
    }

    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::Grpc),
        Some(OtelExporterOtlpLogsEndpoint("foo!://invalid".into())),
        "grpc"
    );
    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::HttpJson),
        None,
        "http/json"
    );
    assert_eq!(
        OtelExporterOtlpLogsEndpoint::get(OtelExporterOtlpProtocol::HttpProtobuf),
        None,
        "http/protobuf"
    );
}
