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

use crate::common::attributes::attr_method_safe;
use crate::rpc_map::RpcMap;
use opentelemetry::KeyValue;
use tonic::codegen::http;

#[derive(Debug, Clone)]
pub(super) struct Attributes {
    pub(super) method: KeyValue,
}

impl Attributes {
    pub(super) fn new(rpc_map: &RpcMap, parts: &http::request::Parts) -> Self {
        let method = attr_method_safe(rpc_map, &parts.uri);
        Self { method }
    }
}

macro_rules! attrs {
    ($attrs:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        &[method] as &[_]
    }};
    ($attrs:expr, $code:expr) => {{
        let attrs: &Attributes = &$attrs;
        let method = attrs.method.clone();
        let status = $crate::common::attributes::attr_status_code($code);
        &[method, status] as &[_]
    }};
}
pub(super) use attrs;
