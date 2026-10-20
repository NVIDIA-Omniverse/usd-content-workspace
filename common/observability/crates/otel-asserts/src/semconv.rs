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

//! Random grab bag of "semantic conventions".
pub use opentelemetry_semantic_conventions::attribute::{
    CODE_FILE_PATH, CODE_LINE_NUMBER, ERROR_TYPE, EXCEPTION_MESSAGE, EXCEPTION_STACKTRACE,
    EXCEPTION_TYPE, HTTP_REQUEST_METHOD, HTTP_REQUEST_METHOD_ORIGINAL, HTTP_RESPONSE_STATUS_CODE,
    HTTP_ROUTE, NETWORK_PROTOCOL_VERSION, OTEL_SCOPE_NAME, OTEL_STATUS_CODE,
    OTEL_STATUS_DESCRIPTION, RPC_GRPC_STATUS_CODE, RPC_METHOD, RPC_SERVICE, RPC_SYSTEM,
    SERVER_ADDRESS, THREAD_ID, THREAD_NAME, URL_PATH, USER_AGENT_ORIGINAL,
};
pub use opentelemetry_semantic_conventions::metric::{
    HTTP_SERVER_ACTIVE_REQUESTS, HTTP_SERVER_REQUEST_BODY_SIZE, HTTP_SERVER_REQUEST_DURATION,
    HTTP_SERVER_RESPONSE_BODY_SIZE, RPC_SERVER_REQUEST_SIZE, RPC_SERVER_REQUESTS_PER_RPC,
    RPC_SERVER_RESPONSE_SIZE, RPC_SERVER_RESPONSES_PER_RPC,
};

/// Module name (same as [`module_path!`]).
pub const CODE_MODULE_NAME: &str = "code.module.name";
/// Event target.
pub const TARGET: &str = "target";
/// Event level.
pub const LEVEL: &str = "level";
/// Span kind (e.g. `client`, `server`, `internal`).
pub const SPAN_KIND: &str = "span.kind";
/// Span busy time in nanoseconds.
pub const BUSY_NS: &str = "busy_ns";
/// Span busy time in nanoseconds.
pub const IDLE_NS: &str = "idle_ns";
/// Internal span format (always `otlp`).
pub const INTERNAL_SPAN_FORMAT: &str = "internal.span.format";
/// GRPC server error.
pub const RPC_GRPC_SERVER_ERROR: &str = "rpc.grpc.server_error";
/// Exception source [`std::fmt::Debug`] representation.
pub const EXCEPTION_SOURCE_DEBUG: &str = "exception.source.debug";
/// Exception source [`std::fmt::Display`] representation.
pub const EXCEPTION_SOURCE_DISPLAY: &str = "exception.source.display";

pub const RPC_SERVER_ACTIVE_METHODS: &str = "rpc.server.active_methods";
pub const RPC_SERVER_ACTIVE_REQUESTS: &str = "rpc.server.active_requests";
pub const RPC_SERVER_ACTIVE_RESPONSES: &str = "rpc.server.active_responses";
pub const RPC_SERVER_CALLS: &str = "rpc.server.calls";

/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments>.
pub const GRPC_SERVER_CALL_STARTED: &str = "grpc.server.call.started";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments>.
pub const GRPC_SERVER_CALL_DURATION: &str = "grpc.server.call.duration";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments>.
pub const GRPC_SERVER_CALL_TX_SIZE: &str = "grpc.server.call.sent_total_compressed_message_size";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#server-instruments>.
pub const GRPC_SERVER_CALL_RX_SIZE: &str = "grpc.server.call.rcvd_total_compressed_message_size";

/// Custom A66 gRPC metric for active handlers.
pub const GRPC_SERVER_HANDLER_ACTIVE: &str = "grpc.server.handler.active";
/// Custom A66 gRPC metric for active request streams.
pub const GRPC_SERVER_REQUEST_ACTIVE: &str = "grpc.server.request.active";
/// Custom A66 gRPC metric for active response streams.
pub const GRPC_SERVER_RESPONSE_ACTIVE: &str = "grpc.server.response.active";

/// Custom HTTP metric mirroring A66's `grpc.server.call.started`.
pub const HTTP_SERVER_REQUEST_STARTED: &str = "http.server.request.started";

// TODO: use upstream constants when opentelemetry-semantic-conventions adopts
// the stable RPC semconv attribute names.
pub const RPC_SYSTEM_NAME: &str = "rpc.system.name";
pub const RPC_METHOD_ORIGINAL: &str = "rpc.method_original";
pub const RPC_RESPONSE_STATUS_CODE: &str = "rpc.response.status_code";

pub const GRPC_STATUS: &str = "grpc.status";

/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#client-instruments>.
pub const GRPC_CLIENT_ATTEMPT_STARTED: &str = "grpc.client.attempt.started";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#client-instruments>.
pub const GRPC_CLIENT_ATTEMPT_DURATION: &str = "grpc.client.attempt.duration";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#client-instruments>.
pub const GRPC_CLIENT_ATTEMPT_TX_SIZE: &str =
    "grpc.client.attempt.sent_total_compressed_message_size";
/// See <https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#client-instruments>.
pub const GRPC_CLIENT_ATTEMPT_RX_SIZE: &str =
    "grpc.client.attempt.rcvd_total_compressed_message_size";

pub const GRPC_CLIENT_ATTEMPT_ACTIVE: &str = "grpc.client.attempt.active";
pub const GRPC_CLIENT_REQUEST_ACTIVE: &str = "grpc.client.request.active";
pub const GRPC_CLIENT_RESPONSE_ACTIVE: &str = "grpc.client.response.active";
