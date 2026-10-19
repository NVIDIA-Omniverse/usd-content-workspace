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

use super::{Attributes, RequestMetrics, TonicMetricRequestBody};
use http_body::Body as _;
use opentelemetry::KeyValue;
use testing::body::BogusBody;
use testing::metrics::BogusInstrumentProvider;

#[test]
fn request_body_forwards_to_inner() {
    let attrs = Attributes {
        method: KeyValue::new("bogus", "bogus"),
    };

    let meter = BogusInstrumentProvider::meter();
    let request_size = meter.u64_histogram("foo").build();
    let active_requests = meter.i64_up_down_counter("bar").build();
    let metrics = RequestMetrics {
        request_size,
        active_requests,
    };

    let body = TonicMetricRequestBody {
        inner: tonic::body::Body::new(BogusBody::<tonic::Status>::new()),
        attrs,
        metrics,
        deframer: Default::default(),
    };

    assert_eq!(body.size_hint().exact(), Some(42));
}
