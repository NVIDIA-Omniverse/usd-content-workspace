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

use crate::harness::prelude::*;
use testing::output::{OutputMetrics, OutputSpans};

/// Asserts the server span and the metrics after the client resets the
/// stream with a request body error (see [`super::req_err_layer`]).
#[track_caller]
pub(super) fn assert_server_after_reset(
    actual_spans: &OutputSpans<'_>,
    actual_metrics: &OutputMetrics<'_>,
) {
    // h2 0.4.16 and later deliver the request headers to the server before the
    // injected body error resets the stream, so what the server records is a
    // race between the reset and the server's handling. Older h2 never exposed
    // the stream at all. The server either:
    // * never sees the call (no span),
    // * fails the call reading the request body: with `Internal` if it reads
    //   the reset, or `Unknown` if the client is already done and it reads
    //   the connection going away instead, or
    // * sees the call, but the reset lands before it reads the request body,
    //   and the call is dropped without a response (no response status).
    let server_status = actual_spans.server.map(|server| {
        server
            .attributes
            .iter()
            .find(|kv| kv.key() == semconv::RPC_RESPONSE_STATUS_CODE)
            .and_then(|kv| kv.value.clone())
    });
    match server_status {
        None => {}
        Some(Some(status)) => {
            let code = if status == sn(Code::Unknown) {
                Code::Unknown
            } else {
                Code::Internal
            };
            actual_spans.assert_server_attrs(expected_attrs! {
                // metadata
                semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
                semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
                semconv::ERROR_TYPE => sn(code),
                semconv::OTEL_STATUS_CODE => "Error",
                semconv::OTEL_STATUS_DESCRIPTION => "h2%20protocol%20error:%20error%20reading%20a%20body%20from%20connection",
                // General
                semconv::SERVER_ADDRESS => "127.0.0.1",
                // gRPC
                semconv::RPC_SYSTEM_NAME => "grpc",
                semconv::RPC_METHOD => "example.Unary/Echo",
                semconv::RPC_RESPONSE_STATUS_CODE => sn(code),
            });
        }
        Some(None) => actual_spans.assert_server_attrs(expected_attrs! {
            // metadata
            semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
            semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
            semconv::OTEL_STATUS_CODE => "Unset",
            // General
            semconv::SERVER_ADDRESS => "127.0.0.1",
            // gRPC
            semconv::RPC_SYSTEM_NAME => "grpc",
            semconv::RPC_METHOD => "example.Unary/Echo",
        }),
    }

    // the server records the same outcomes as above
    let mut candidates = vec![expected_metrics!(
        build actual_metrics => "example.Unary/Echo",
        {
            semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
            semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
            semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Internal),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x 0.0 },
        },
    )];
    for code in [Code::Internal, Code::Unknown] {
        candidates.push(expected_metrics!(
            build actual_metrics => "example.Unary/Echo",
            {
                semconv::GRPC_SERVER_CALL_DURATION => "s": histogram { 1 x > 0.0 } => {
                    semconv::GRPC_STATUS => sn(code),
                },
                semconv::GRPC_SERVER_CALL_RX_SIZE => "By": histogram { 1 x 0.0 },
                semconv::GRPC_SERVER_CALL_TX_SIZE => "By": histogram { 1 x 0.0 } => {
                    semconv::GRPC_STATUS => sn(code),
                },
                semconv::GRPC_SERVER_CALL_STARTED => "{call}": sum { 1 },
                semconv::GRPC_SERVER_HANDLER_ACTIVE => "{handler}": sum { 0 },
                semconv::GRPC_SERVER_REQUEST_ACTIVE => "{request}": sum { 0 },
                semconv::GRPC_SERVER_RESPONSE_ACTIVE => "{response}": sum { 0 },
            },
            {
                semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
                semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
                semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
                semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                    semconv::GRPC_STATUS => sn(Code::Internal),
                },
                semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x 0.0 },
            },
        ));
    }
    candidates.push(expected_metrics!(
        build actual_metrics => "example.Unary/Echo",
        {
            semconv::GRPC_SERVER_CALL_RX_SIZE => "By": histogram { 1 x 0.0 },
            semconv::GRPC_SERVER_CALL_STARTED => "{call}": sum { 1 },
            semconv::GRPC_SERVER_HANDLER_ACTIVE => "{handler}": sum { 0 },
            semconv::GRPC_SERVER_REQUEST_ACTIVE => "{request}": sum { 0 },
        },
        {
            semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
            semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
            semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Internal),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x 0.0 },
        },
    ));
    actual_metrics.assert_one_of(candidates);
}
