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

use std::error::Error;
use tonic::Code;
use tonic::codegen::http;

/// All non-OK codes are client errors.
///
/// See <https://opentelemetry.io/docs/specs/semconv/rpc/grpc/#grpc-status>.
pub(crate) fn code_is_error(code: u16) -> bool {
    const OK: u16 = tonic::Code::Ok as u16;
    code != OK
}

pub(crate) fn http_to_tonic_status(status: http::StatusCode) -> Code {
    match status {
        // https://github.com/grpc/grpc/blob/master/doc/http-grpc-status-mapping.md
        http::StatusCode::BAD_REQUEST => Code::Internal,
        http::StatusCode::UNAUTHORIZED => Code::Unauthenticated,
        http::StatusCode::FORBIDDEN => Code::PermissionDenied,
        http::StatusCode::NOT_FOUND => Code::Unimplemented,
        http::StatusCode::TOO_MANY_REQUESTS
        | http::StatusCode::BAD_GATEWAY
        | http::StatusCode::SERVICE_UNAVAILABLE
        | http::StatusCode::GATEWAY_TIMEOUT => Code::Unavailable,
        _ => Code::Unknown,
    }
}

pub(crate) fn transport_err_to_code(e: &tonic::transport::Error) -> Code {
    // `tonic::transport::Error` is an opaque type that doesn't implement clone.
    // so we can't call `Status::from_error` or `Status::try_from_error` to
    // reuse the tonic code-mapping logic. we have to re-implement that
    // ourselves by inspecting the source and down-casting.
    //
    // for clients, any error is ultimately converted to a `Status` by the gRPC
    // client dispatcher:
    // https://github.com/hyperium/tonic/blob/v0.14.5/tonic/src/client/grpc.rs#L314
    //
    // * `Status::from_error_generic` -> `Status::from_error` -> `Status::try_from_error`
    // * because the type is `tonic::transport::Error`, none of the downcasts in
    //   `Status::try_from_error()` succeed. this is lucky for us. while it
    //   isn't possible to construct `tonic::transport::Error`, this means we
    //   only have to inspect the source.
    // * finally, `find_status_in_source_chain` is called.
    //
    // in short, we "only" have to replicate the `find_status_in_source_chain`
    // logic.
    find_code_in_source_chain(e.source())
}

fn find_code_in_source_chain(mut source: Option<&(dyn std::error::Error + 'static)>) -> Code {
    while let Some(err) = source {
        if let Some(status) = err.downcast_ref::<tonic::Status>() {
            return status.code();
        }
        if let Some(_timeout) = err.downcast_ref::<tonic::TimeoutExpired>() {
            return Code::Cancelled;
        }
        if let Some(_connect) = err.downcast_ref::<tonic::ConnectError>() {
            return Code::Unavailable;
        }
        // In `find_status_in_source_chain`, this block is guarded by:
        // ```
        // #[cfg(any(feature = "server", feature = "channel"))]
        // ```
        // Since we enable `transport`, which enables both `server` and
        // `channel`, the block always applies (due to feature unification).
        if let Some(hyper_err) = err.downcast_ref::<hyper::Error>() {
            if hyper_err.is_timeout() {
                return Code::Unavailable;
            }
            if hyper_err.is_canceled() {
                return Code::Cancelled;
            }
            if let Some(h2_err) = hyper_err
                .source()
                .and_then(|src| src.downcast_ref::<h2::Error>())
            {
                // message becomes "h2 protocol error: " when this branch is hit
                return h2_err_to_code(h2_err);
            }
        }
        source = err.source();
    }
    Code::Unknown
}

fn h2_err_to_code(err: &h2::Error) -> Code {
    use h2::Reason;
    // see https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md#errors
    match err.reason() {
        Some(Reason::NO_ERROR) => Code::Internal,
        Some(Reason::PROTOCOL_ERROR) => Code::Internal,
        Some(Reason::INTERNAL_ERROR) => Code::Internal,
        Some(Reason::FLOW_CONTROL_ERROR) => Code::Internal,
        Some(Reason::SETTINGS_TIMEOUT) => Code::Internal,
        Some(Reason::STREAM_CLOSED) => Code::Unknown,
        // note: there might be a bug in tonic, per the doc FRAME_SIZE_ERROR
        // should be mapped to INTERNAL.
        // https://github.com/hyperium/tonic/issues/2577
        // Some(Reason::FRAME_SIZE_ERROR) => Code::Internal,
        Some(Reason::FRAME_SIZE_ERROR) => Code::Unknown,
        Some(Reason::REFUSED_STREAM) => Code::Unavailable,
        Some(Reason::CANCEL) => Code::Cancelled,
        Some(Reason::COMPRESSION_ERROR) => Code::Internal,
        Some(Reason::CONNECT_ERROR) => Code::Internal,
        Some(Reason::ENHANCE_YOUR_CALM) => Code::ResourceExhausted,
        Some(Reason::INADEQUATE_SECURITY) => Code::PermissionDenied,
        _ => Code::Unknown,
    }
}

#[cfg(test)]
mod tests;
