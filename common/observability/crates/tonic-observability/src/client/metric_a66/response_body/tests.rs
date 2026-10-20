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

use super::{Attributes, ResponseMetrics, TonicMetricResponseBody};
use crate::rpc_map::RpcMap;
use http_body::Body as _;
use opentelemetry::StringValue;
use std::time::Instant;
use testing::body::BogusBody;
use testing::metrics::BogusInstrumentProvider;
use tonic::codegen::http;

#[test]
fn response_body_forwards_to_inner() {
    let (parts, ()) = http::request::Request::builder()
        .body(())
        .unwrap()
        .into_parts();
    let rpc_map = RpcMap::default();
    let attrs = Attributes::new(&rpc_map, &parts, StringValue::from(""));

    let meter = BogusInstrumentProvider::meter();
    let response_size = meter.u64_histogram("a").build();
    let active_responses = meter.i64_up_down_counter("b").build();
    let call_duration = meter.f64_histogram("c").build();
    let active_attempts = meter.i64_up_down_counter("d").build();
    let metrics = ResponseMetrics {
        response_size,
        active_responses,
        call_duration,
        active_attempts,
    };

    let body = TonicMetricResponseBody {
        inner: BogusBody::<tonic::Status>::new(),
        attrs,
        metrics,
        call_start: Instant::now(),
        code: None,
        deframer: Default::default(),
    };

    assert_eq!(body.size_hint().exact(), Some(42));
}
