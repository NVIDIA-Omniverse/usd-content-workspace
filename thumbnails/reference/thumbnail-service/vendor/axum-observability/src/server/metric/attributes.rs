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

use crate::common::extract;
use opentelemetry::{KeyValue, StringValue, Value};
use opentelemetry_semantic_conventions::attribute as attr;

fn request_route(extensions: &http::Extensions) -> Option<Value> {
    extensions
        .get::<axum::extract::MatchedPath>()
        .map(|path| Value::String(StringValue::from(path.as_str().to_string())))
}

#[derive(Debug, Clone)]
pub(super) struct Attributes {
    pub(super) method: KeyValue,
    pub(super) version: KeyValue,
    pub(super) route: Option<KeyValue>,
}

impl Attributes {
    pub(super) fn new(parts: &http::request::Parts) -> Self {
        let method = extract::method(&parts.method).unwrap_or("_OTHER");
        let method = KeyValue::new(attr::HTTP_REQUEST_METHOD, method);
        let version = extract::version(parts.version);
        let version = KeyValue::new(attr::NETWORK_PROTOCOL_VERSION, version);
        let route = request_route(&parts.extensions);
        let route = route.map(|route| KeyValue::new(attr::HTTP_ROUTE, route));
        Self {
            method,
            version,
            route,
        }
    }
}

pub(super) fn attr_status(status: http::StatusCode) -> KeyValue {
    let value = Value::I64(status.as_u16().into());
    KeyValue::new(attr::HTTP_RESPONSE_STATUS_CODE, value)
}

pub(super) fn attr_error_type(error_type: &str) -> KeyValue {
    KeyValue::new(attr::ERROR_TYPE, error_type.to_string())
}

macro_rules! attrs {
    ($attrs:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        let version = attrs.version.clone();
        match attrs.route.clone() {
            Some(route) => &[method, version, route] as &[_],
            None => &[method, version] as &[_],
        }
    }};
    ($attrs:expr, $status:expr, $error_type:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        let version = attrs.version.clone();
        let status = $crate::server::metric::attributes::attr_status($status);
        let error_type: Option<&str> = $error_type;
        let error_type = error_type.map($crate::server::metric::attributes::attr_error_type);
        match (attrs.route.clone(), error_type) {
            (Some(route), Some(et)) => &[method, version, route, status, et] as &[_],
            (Some(route), None) => &[method, version, route, status] as &[_],
            (None, Some(et)) => &[method, version, status, et] as &[_],
            (None, None) => &[method, version, status] as &[_],
        }
    }};
}
pub(super) use attrs;
