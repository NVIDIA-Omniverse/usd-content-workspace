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

use super::{attr_status_code, target_fallback};
use tonic::codegen::http;

#[test]
fn attr_status_code_unknown() {
    let kv = attr_status_code(Some(u16::MAX));
    assert_eq!(kv.value, "65535".into());
}

#[test]
fn target_fallback_authority_only() {
    let mut uri_parts = http::uri::Parts::default();
    uri_parts.authority = Some(http::uri::Authority::from_static("foo"));
    let uri = http::uri::Uri::from_parts(uri_parts).unwrap();
    let (parts, ()) = http::Request::builder()
        .uri(uri)
        .body(())
        .unwrap()
        .into_parts();
    let value = target_fallback(&parts);
    assert_eq!(value, Some("foo".into()));
}

#[test]
fn target_fallback_relative_uri() {
    let uri: http::uri::Uri = "/foo".parse().unwrap();
    let (parts, ()) = http::Request::builder()
        .uri(uri)
        .body(())
        .unwrap()
        .into_parts();
    let value = target_fallback(&parts);
    assert_eq!(value, None);
}
