// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "projectcontour.io",
    version = "v1",
    kind = "HTTPProxy",
    namespaced
)]
pub struct HTTPProxySpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtualhost: Option<VirtualHost>,
    #[serde(default)]
    pub routes: Vec<Route>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct VirtualHost {
    pub fqdn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<VirtualHostTls>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct VirtualHostTls {
    #[serde(
        default,
        rename = "secretName",
        skip_serializing_if = "Option::is_none"
    )]
    pub secret_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Route {
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub services: Vec<RouteService>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Condition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct RouteService {
    pub name: String,
    pub port: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}
