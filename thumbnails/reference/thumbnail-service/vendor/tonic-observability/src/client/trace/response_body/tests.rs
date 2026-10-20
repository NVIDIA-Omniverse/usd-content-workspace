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

use super::{TonicTraceResponseBody, span};
use crate::test_support::BogusBody;
use http_body::Body as _;

#[test]
fn response_body_forwards_size_hint_to_inner() {
    let span = tracing::info_span!("noop");
    let body = TonicTraceResponseBody {
        inner: BogusBody::<tonic::Status>::new(),
        span,
        span_status: span::SpanStatus::new(),
    };
    assert_eq!(body.size_hint().exact(), Some(42));
}
