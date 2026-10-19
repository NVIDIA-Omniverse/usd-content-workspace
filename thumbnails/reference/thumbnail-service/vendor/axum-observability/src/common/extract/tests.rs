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

#[test]
fn method_known() {
    for method in [
        http::Method::CONNECT,
        http::Method::DELETE,
        http::Method::GET,
        http::Method::HEAD,
        http::Method::OPTIONS,
        http::Method::PATCH,
        http::Method::POST,
        http::Method::PUT,
        http::Method::TRACE,
        http::Method::from_bytes(b"QUERY").unwrap(),
    ] {
        assert_eq!(super::method(&method), Some(method.as_str()));
    }
}

#[test]
fn method_other() {
    let method = http::Method::from_bytes(b"FOO").unwrap();
    assert_eq!(super::method(&method), None);
}

#[test]
fn version_known() {
    for (version, repr) in [
        (http::Version::HTTP_09, "0.9"),
        (http::Version::HTTP_10, "1.0"),
        (http::Version::HTTP_11, "1.1"),
        (http::Version::HTTP_2, "2"),
        (http::Version::HTTP_3, "3"),
    ] {
        assert_eq!(super::version(version), repr);
    }
}

macro_rules! assert_forwarded_host {
    ($input:expr, $expected:expr) => {{
        let value = http::HeaderValue::from_str($input).unwrap();
        assert_eq!(super::forwarded_host(&value), $expected);
    }};
}

#[test]
fn forwarded_host_single_element() {
    assert_forwarded_host!("host=example.com", Some("example.com"));
}

#[test]
fn forwarded_host_with_other_params() {
    assert_forwarded_host!(
        "for=192.0.2.60;proto=https;host=example.com;by=203.0.113.43",
        Some("example.com")
    );
}

#[test]
fn forwarded_host_quoted() {
    assert_forwarded_host!("host=\"example.com\"", Some("example.com"));
}

#[test]
fn forwarded_host_case_insensitive() {
    assert_forwarded_host!("HOST=example.com", Some("example.com"));
    assert_forwarded_host!("Host=example.com", Some("example.com"));
}

#[test]
fn forwarded_host_first_element_used() {
    assert_forwarded_host!(
        "host=first.example.com, host=second.example.com",
        Some("first.example.com")
    );
}

#[test]
fn forwarded_host_missing() {
    assert_forwarded_host!("for=192.0.2.60;proto=https", None);
}

#[test]
fn forwarded_host_first_element_missing() {
    assert_forwarded_host!("for=192.0.2.60, host=second.example.com", None);
}

#[test]
fn req_http_host_no_host() {
    let (parts, ()) = http::Request::new(()).into_parts();
    let host = super::req_http_host(&parts);
    assert_eq!(host, None);
}
