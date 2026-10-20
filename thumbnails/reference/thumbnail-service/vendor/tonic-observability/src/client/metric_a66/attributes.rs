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

use crate::common::attributes::attr_method_raw;
use crate::rpc_map::RpcMap;
use opentelemetry::{KeyValue, StringValue};
use tonic::codegen::http;

const GRPC_TARGET_ATTR: &str = "grpc.target";

#[derive(Debug, Clone)]
pub(super) struct Attributes {
    pub(super) method: KeyValue,
    pub(super) target: KeyValue,
}

impl Attributes {
    pub(super) fn new(rpc_map: &RpcMap, parts: &http::request::Parts, target: StringValue) -> Self {
        let method = attr_method_raw(rpc_map, &parts.uri);
        let target = KeyValue::new(GRPC_TARGET_ATTR, target);
        Self { method, target }
    }
}

macro_rules! attrs {
    ($attrs:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        let target = attrs.target.clone();
        &[method, target] as &[_]
    }};
    ($attrs:expr, $code:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        let target = attrs.target.clone();
        let status = $crate::common::attributes::attr_status_code($code);
        &[method, target, status] as &[_]
    }};
}
pub(super) use attrs;
