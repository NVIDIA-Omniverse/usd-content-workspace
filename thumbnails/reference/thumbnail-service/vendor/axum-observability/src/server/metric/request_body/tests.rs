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

use super::{Attributes, RequestMetrics, TowerMetricRequestBody};
use crate::test_support::BogusBody;
use crate::test_support::BogusInstrumentProvider;
use http_body::Body as _;
use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions::attribute as attr;

#[test]
fn request_body_forwards_to_inner() {
    let meter = BogusInstrumentProvider::meter();
    let request_started = meter.u64_counter("bar").build();
    let request_size = meter.u64_histogram("foo").build();
    let metrics = RequestMetrics {
        request_started,
        request_size,
    };

    let body = TowerMetricRequestBody {
        inner: BogusBody::<std::convert::Infallible>::new(),
        attrs: Attributes {
            method: KeyValue::new(attr::HTTP_REQUEST_METHOD, "GET"),
            version: KeyValue::new(attr::NETWORK_PROTOCOL_VERSION, "HTTP/2"),
            route: None,
        },
        metrics,
        size: 0,
    };

    assert!(!body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(42));
}
