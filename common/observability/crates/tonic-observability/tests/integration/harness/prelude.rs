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

pub(crate) use super::run::harness;
pub(crate) use super::utils::{
    ConsumeResponse, consume_request, echo_req, echo_resp, get_histogram_sum, make_req, make_resp,
    resource, rpc_map, sn,
};
pub(crate) use futures::StreamExt;
pub(crate) use otel_asserts::semconv;
pub(crate) use protos::{EchoRequest, EchoResponse};
pub(crate) use testing::{
    expected_attrs, expected_events, expected_grpc_metrics as expected_metrics, expected_hierarchy,
    stream,
};
pub(crate) use tonic::{Code, Request, Response, Result, Status, Streaming, async_trait};
pub(crate) use tracing::{Instrument, info_span, instrument};

pub(crate) const CLIENT_TRACE_PATH: &str = "crates/tonic-observability/src/client/trace/mod.rs";
pub(crate) const CLIENT_TRACE_MODULE: &str = "tonic_observability::client::trace";

pub(crate) const SERVER_TRACE_PATH: &str = "crates/tonic-observability/src/server/trace/mod.rs";
pub(crate) const SERVER_TRACE_MODULE: &str = "tonic_observability::server::trace";
