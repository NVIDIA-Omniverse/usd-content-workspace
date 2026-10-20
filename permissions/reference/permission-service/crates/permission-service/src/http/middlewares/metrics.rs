// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::state::AppState;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::metrics::UpDownCounter;
use opentelemetry::KeyValue;

struct InFlightGuard(UpDownCounter<i64>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.add(-1, &[]);
    }
}

pub async fn metrics_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    state.metrics.http_requests_in_flight.add(1, &[]);
    let _in_flight_guard = InFlightGuard(state.metrics.http_requests_in_flight.clone());

    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| MatchedPath::as_str(path).to_string())
        .unwrap_or_default();
    let method = request.method();

    let mut attrs = vec![
        KeyValue::new("path", path),
        KeyValue::new("method", method.to_string()),
    ];
    state.metrics.request_count.add(1, &attrs);

    let response = next.run(request).await;

    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        attrs.push(KeyValue::new("status_code", status.as_u16().to_string()));
        state.metrics.request_errors.add(1, &attrs);
    }

    response
}
