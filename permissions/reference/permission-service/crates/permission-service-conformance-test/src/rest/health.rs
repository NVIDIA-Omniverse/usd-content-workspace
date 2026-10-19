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

use crate::{TestError, TestRunner};
use std::time::Instant;

/// Health check used both by the readiness wait loop and as a test case.
pub async fn check(
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
) -> Result<(), TestError> {
    let url = format!("{}/health", rest_url);
    let request_str = format!("GET {}", url);
    let mut request = client.get(&url);
    if let Some(auth) = auth_header {
        request = request.header("Authorization", auth);
    }

    let response = request
        .send()
        .await
        .map_err(|e| TestError::from_reqwest_error(&e, rest_url).with_request(&request_str))?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(TestError::from_status(status, &body, rest_url)
            .with_request(&request_str)
            .with_response(format!("HTTP {}\n{}", status, body)))
    }
}

pub async fn run_tests(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
) {
    let start = Instant::now();
    match check(client, rest_url, auth_header).await {
        Ok(()) => runner.record_pass("GET /health - health check", start.elapsed().as_millis()),
        Err(e) => runner.record_fail(
            "GET /health - health check",
            start.elapsed().as_millis(),
            &e,
        ),
    }
}
