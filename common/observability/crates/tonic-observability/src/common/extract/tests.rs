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
use tonic::transport::Uri;

#[test]
fn req_http_host() {
    let (mut parts, ()) = http::Request::new(()).into_parts();

    let http_host = super::req_http_host(&parts);
    assert_eq!(http_host, None);

    let val = http::HeaderValue::from_static("127.0.0.1");
    parts.headers.insert(http::header::HOST, val);
    let http_host = super::req_http_host(&parts);
    assert_eq!(http_host, Some("127.0.0.1"));

    let uri = Uri::from_static("http://localhost/example/HelloWorld");
    parts.uri = uri;
    let http_host = super::req_http_host(&parts);
    assert_eq!(http_host, Some("127.0.0.1"));

    parts.headers.clear();
    let http_host = super::req_http_host(&parts);
    assert_eq!(http_host, Some("localhost"));
}

#[test]
fn resp_grpc_status_from_header_value() {
    let val = http::HeaderValue::from_static("3");
    let grpc_status = super::resp_grpc_status_from_header_value(&val);
    assert_eq!(grpc_status, Some(3));
}

#[test]
fn resp_grpc_status_from_header_value_known() {
    for (val, code, _name) in super::status_codes::KNOWN_HEADER_VALUES.iter().cloned() {
        let grpc_status = super::resp_grpc_status_from_header_value(&val);
        assert_eq!(
            grpc_status,
            Some(code as u16),
            "code: {code}, header: {}",
            val.as_bytes().escape_ascii()
        );
    }
}

#[test]
fn resp_grpc_status_from_header_value_int() {
    let val = http::HeaderValue::from_static("42");
    let grpc_status = super::resp_grpc_status_from_header_value(&val);
    assert_eq!(grpc_status, Some(42));
}

#[test]
fn resp_grpc_status_from_header_value_str() {
    let val = http::HeaderValue::from_static("foo");
    let grpc_status = super::resp_grpc_status_from_header_value(&val);
    assert_eq!(grpc_status, None);
}

#[test]
fn resp_grpc_status_from_header_value_bin() {
    let val = http::HeaderValue::from_bytes(&[0xFF]).unwrap();
    let grpc_status = super::resp_grpc_status_from_header_value(&val);
    assert_eq!(grpc_status, None);
}

#[test]
fn resp_grpc_status_code_to_name_known() {
    for (_val, code, name) in super::status_codes::KNOWN_HEADER_VALUES.iter().cloned() {
        let grpc_name = super::resp_grpc_status_code_to_name(code as u16);
        assert_eq!(grpc_name, Some(name), "code: {code}");
    }
}

#[test]
fn resp_grpc_status_code_to_name_unknown() {
    let code = u16::MAX;
    let grpc_name = super::resp_grpc_status_code_to_name(code);
    assert_eq!(grpc_name, None, "code: {code}");
}

#[test]
fn resp_known_status_code_to_name() {
    for (_val, code, name) in super::status_codes::KNOWN_HEADER_VALUES.iter().cloned() {
        let grpc_name = super::resp_known_status_code_to_name(code);
        assert_eq!(grpc_name, name, "code: {code}");
    }
}
