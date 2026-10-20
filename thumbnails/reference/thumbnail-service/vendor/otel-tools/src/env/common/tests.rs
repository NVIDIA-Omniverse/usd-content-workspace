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

use super::{OtelEnvVal as _, OtelExporterOtlpProtocol, OtelExporterOtlpType, http_endpoint_uri};

#[test]
fn http_endpoint_uri_traces() {
    let ty = OtelExporterOtlpType::Traces;

    assert_eq!(
        http_endpoint_uri("http://localhost", ty),
        "http://localhost/v1/traces"
    );
    assert_eq!(
        http_endpoint_uri("http://localhost/", ty),
        "http://localhost/v1/traces"
    );
}

#[test]
fn http_endpoint_uri_metrics() {
    let ty = OtelExporterOtlpType::Metrics;

    assert_eq!(
        http_endpoint_uri("http://localhost", ty),
        "http://localhost/v1/metrics"
    );
    assert_eq!(
        http_endpoint_uri("http://localhost/", ty),
        "http://localhost/v1/metrics"
    );
}

#[test]
fn http_endpoint_uri_logs() {
    let ty = OtelExporterOtlpType::Logs;

    assert_eq!(
        http_endpoint_uri("http://localhost", ty),
        "http://localhost/v1/logs"
    );
    assert_eq!(
        http_endpoint_uri("http://localhost/", ty),
        "http://localhost/v1/logs"
    );
}

#[test]
fn otel_exporter_otlp_protocol_from_env() {
    assert_eq!(
        OtelExporterOtlpProtocol::from_env("grpc"),
        Ok(OtelExporterOtlpProtocol::Grpc)
    );
    assert_eq!(
        OtelExporterOtlpProtocol::from_env("http/json"),
        Ok(OtelExporterOtlpProtocol::HttpJson)
    );
    assert_eq!(
        OtelExporterOtlpProtocol::from_env("http/protobuf"),
        Ok(OtelExporterOtlpProtocol::HttpProtobuf)
    );
    assert_eq!(
        OtelExporterOtlpProtocol::from_env(""),
        Err("expected `grpc`, `http/json`, or `http/protobuf`")
    );
}

#[test]
fn otel_exporter_otlp_protocol_sdk() {
    assert_eq!(
        OtelExporterOtlpProtocol::Grpc.sdk(),
        opentelemetry_otlp::Protocol::Grpc
    );
    assert_eq!(
        OtelExporterOtlpProtocol::HttpJson.sdk(),
        opentelemetry_otlp::Protocol::HttpJson
    );
    assert_eq!(
        OtelExporterOtlpProtocol::HttpProtobuf.sdk(),
        opentelemetry_otlp::Protocol::HttpBinary
    );
}
