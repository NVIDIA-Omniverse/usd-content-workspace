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

mod client;
mod flavor;
mod http;
mod request;
mod response;
mod run;
mod server;

pub(crate) use run::harness_custom;
pub(crate) use server::MakeService;

pub(crate) mod prelude {
    pub(crate) use super::client::{USER_AGENT, poll_body};
    pub(crate) use super::flavor::{Http2, Http11, HttpVersion, http_test};
    pub(crate) use super::http::*;
    pub(crate) use super::request::Request;
    pub(crate) use super::response::{Consumed, Response, assert_resp, body};
    pub(crate) use super::run::harness;
    pub(crate) use axum::Router;
    pub(crate) use axum::response::IntoResponse;
    pub(crate) use axum::routing::{get, post};
    pub(crate) use http::header;
    pub(crate) use otel_asserts::semconv;
    pub(crate) use testing::{
        expected_attrs, expected_events, expected_hierarchy,
        expected_http_metrics as expected_metrics,
    };
    pub(crate) use tracing::instrument;

    pub(crate) fn resource() -> otel_tools::resource::Resource {
        otel_tools::resource::ResourceBuilder::without_default("nv.test").build()
    }

    pub(crate) const SERVER_TRACE_PATH: &str = "crates/axum-observability/src/server/trace/mod.rs";
    pub(crate) const SERVER_TRACE_MODULE: &str = "axum_observability::server::trace";
}
