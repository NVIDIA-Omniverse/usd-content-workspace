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

pub(crate) const CHUNKED: http::HeaderValue = http::HeaderValue::from_static("chunked");
pub(crate) const MIME_TEXT_PLAIN: http::HeaderValue =
    http::HeaderValue::from_static("text/plain; charset=utf-8");

/// Extract the authority (host:port) from a `http://host:port` URI.
///
/// This is hacky, but re-parsing the Uri creates an owned version.
pub(crate) fn server_address(server_uri: &str) -> &str {
    server_uri.strip_prefix("http://").unwrap()
}

pub(crate) fn sc(code: http::StatusCode) -> otel_asserts::types::common::Value {
    otel_asserts::types::common::Value::String(format!("{}", code.as_u16()))
}

pub(crate) fn ver(version: http::Version) -> otel_asserts::types::common::Value {
    let v = match version {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2",
        http::Version::HTTP_3 => "3",
        _ => unreachable!(),
    };
    otel_asserts::types::common::Value::String(v.into())
}

/// Convert a [`http::StatusCode`] number to an integer compatible with metric
/// asserts.
pub(crate) fn si(code: http::StatusCode) -> i64 {
    code.as_u16().into()
}

macro_rules! status_codes {
        ($($name:ident,)+) => {
            $(pub(crate) const $name: http::StatusCode = http::StatusCode::$name;)+
        };
    }

status_codes! {
    OK,
    BAD_REQUEST,
    NOT_FOUND,
    INTERNAL_SERVER_ERROR,
}

pub(crate) fn make_trailers() -> http::HeaderMap {
    const X_CLACKS_OVERHEAD: http::HeaderName = http::HeaderName::from_static("x-clacks-overhead");
    const GNU_TERRY_PRATCHETT: http::HeaderValue =
        http::HeaderValue::from_static("GNU Terry Pratchett");

    let mut trailers = http::HeaderMap::new();
    trailers.append(X_CLACKS_OVERHEAD, GNU_TERRY_PRATCHETT);
    trailers
}
