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
    OtelEnvVal as _, OtelExporterOtlpProtocol, OtelExporterOtlpTracesEndpoint, OtelPropagators,
    OtelTracesExporter,
};
use crate::testing::unsafe_env;

#[test]
fn otel_traces_exporter_from_env() {
    assert_eq!(
        OtelTracesExporter::from_env("otlp"),
        Ok(OtelTracesExporter::Otlp)
    );
    assert_eq!(
        OtelTracesExporter::from_env("none"),
        Ok(OtelTracesExporter::None)
    );
    assert_eq!(
        OtelTracesExporter::from_env("foo"),
        Err("expected `otlp` or `none`")
    );
}

#[test]
fn otel_propagators_from_env() {
    assert_eq!(
        OtelPropagators::from_env("tracecontext"),
        Ok(OtelPropagators {
            trace_context: true
        })
    );
    assert_eq!(
        OtelPropagators::from_env("tracecontext,baggage"),
        Err("expected `tracecontext` only")
    );
}

#[test]
fn otel_exporter_otlp_traces_endpoint_get_valid() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": "foo://invalid",
    }

    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::Grpc),
        Some(OtelExporterOtlpTracesEndpoint("foo://invalid".into())),
        "grpc"
    );
    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::HttpJson),
        Some(OtelExporterOtlpTracesEndpoint("foo://invalid/".into())),
        "http/json"
    );
    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::HttpProtobuf),
        Some(OtelExporterOtlpTracesEndpoint("foo://invalid/".into())),
        "http/protobuf"
    );
}

#[test]
fn otel_exporter_otlp_traces_endpoint_get_invalid() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": "foo!://invalid",
    }

    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::Grpc),
        Some(OtelExporterOtlpTracesEndpoint("foo!://invalid".into())),
        "grpc"
    );
    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::HttpJson),
        None,
        "http/json"
    );
    assert_eq!(
        OtelExporterOtlpTracesEndpoint::get(OtelExporterOtlpProtocol::HttpProtobuf),
        None,
        "http/protobuf"
    );
}
