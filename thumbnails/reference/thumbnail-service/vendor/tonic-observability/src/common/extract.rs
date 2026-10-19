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

use tonic::codegen::http;

pub const GRPC_STATUS: http::HeaderName = tonic::Status::GRPC_STATUS;
pub const GRPC_MESSAGE: http::HeaderName = tonic::Status::GRPC_MESSAGE;
pub const GRPC_STATUS_DETAILS: http::HeaderName = tonic::Status::GRPC_STATUS_DETAILS;

fn header_to_str(header: &http::HeaderValue) -> Option<&str> {
    header.to_str().ok()
}

fn req_http_host_from_headers(headers: &http::HeaderMap) -> Option<&str> {
    headers.get(http::header::HOST).and_then(header_to_str)
}

/// Extract the HTTP host from a request.
///
/// This favours the value from the `host` header over the URI's host component.
#[inline]
pub fn req_http_host(parts: &http::request::Parts) -> Option<&str> {
    req_http_host_from_headers(&parts.headers).or(parts.uri.host())
}

/// A macro to construct a function to extract the gRPC status from a header
/// value.
///
/// We really want to avoid parsing the header value each time. For known
/// codes, we can compare the raw header value to known gRPC status codes.
macro_rules! grpc_status {
    ($($code:path => ($name:ident, $num:literal, $desc:literal)),+ $(,)?) => {
        mod status_codes {
            $(grpc_status!(@const $name, $num, $code);)+

            #[allow(unused, reason = "used for test")]
            pub(super) const KNOWN_HEADER_VALUES: [(
                ::tonic::codegen::http::HeaderValue, ::tonic::Code, &'static str
            ); 17] = [
                $((
                    ::tonic::codegen::http::HeaderValue::from_static(stringify!($num)),
                    $code,
                    $desc,
                ),)+
            ];
        }

        /// Extract the gRPC status from a header value.
        ///
        /// To be forwards-compatible, the value is parsed as a `u16` and not
        /// [`tonic::Status`]. For known header codes, binary comparison is used
        /// instead of parsing the value.
        pub(crate) fn resp_grpc_status_from_header_value(val: &http::HeaderValue) -> Option<u16> {
            let bytes = val.as_bytes();
            match bytes {
                // fast path
                $(status_codes::$name => Some($code as u16),)+
                // slow path
                _ => {
                    let res = val.to_str().ok().and_then(|s| s.parse().ok());
                    if res.is_none() {
                        // the header was not a string or an integer...
                        let header = tracing::field::display(bytes.escape_ascii());
                        tracing::error!(%header, "invalid grpc-status header");
                    }
                    res
                }
            }
        }

        /// Map the gRPC status code to a string.
        pub(crate) fn resp_grpc_status_code_to_name(code: u16) -> Option<&'static str> {
            match code {
                $($num => Some($desc),)+
                _ => None,
            }
        }

        /// Map the gRPC status code to a string.
        ///
        /// This is a special case where the code has to be known, in general
        /// to be future-compatible use [`resp_grpc_status_code_to_name`] to
        /// handle unknown codes.
        pub(crate) fn resp_known_status_code_to_name(code: tonic::Code) -> &'static str {
            match code {
                $($code => $desc,)+
            }
        }
    };
    (@const $name:ident, $num:literal, $code:path) => {
        pub(super) const $name: &[u8] = stringify!($num).as_bytes();
        const _: () = if $code as u16 != $num {
            panic!("invalid code");
        };
    };
}

grpc_status! {
    tonic::Code::Ok => (GRPC_STATUS_OK, 0, "OK"),
    tonic::Code::Cancelled => (GRPC_STATUS_CANCELLED, 1, "CANCELLED"),
    tonic::Code::Unknown => (GRPC_STATUS_UNKNOWN, 2, "UNKNOWN"),
    tonic::Code::InvalidArgument => (GRPC_STATUS_INVALID_ARGUMENT, 3, "INVALID_ARGUMENT"),
    tonic::Code::DeadlineExceeded => (GRPC_STATUS_DEADLINE_EXCEEDED, 4, "DEADLINE_EXCEEDED"),
    tonic::Code::NotFound => (GRPC_STATUS_NOT_FOUND, 5, "NOT_FOUND"),
    tonic::Code::AlreadyExists => (GRPC_STATUS_ALREADY_EXISTS, 6, "ALREADY_EXISTS"),
    tonic::Code::PermissionDenied => (GRPC_STATUS_PERMISSION_DENIED, 7, "PERMISSION_DENIED"),
    tonic::Code::ResourceExhausted => (GRPC_STATUS_RESOURCE_EXHAUSTED, 8, "RESOURCE_EXHAUSTED"),
    tonic::Code::FailedPrecondition => (GRPC_STATUS_FAILED_PRECONDITION, 9, "FAILED_PRECONDITION"),
    tonic::Code::Aborted => (GRPC_STATUS_ABORTED, 10, "ABORTED"),
    tonic::Code::OutOfRange => (GRPC_STATUS_OUT_OF_RANGE, 11, "OUT_OF_RANGE"),
    tonic::Code::Unimplemented => (GRPC_STATUS_UNIMPLEMENTED, 12, "UNIMPLEMENTED"),
    tonic::Code::Internal => (GRPC_STATUS_INTERNAL, 13, "INTERNAL"),
    tonic::Code::Unavailable => (GRPC_STATUS_UNAVAILABLE, 14, "UNAVAILABLE"),
    tonic::Code::DataLoss => (GRPC_STATUS_DATA_LOSS, 15, "DATA_LOSS"),
    tonic::Code::Unauthenticated => (GRPC_STATUS_UNAUTHENTICATED, 16, "UNAUTHENTICATED"),
}

#[cfg(test)]
mod tests;
