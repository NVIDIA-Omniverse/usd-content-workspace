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

use std::time::Duration;

use serde::Deserialize;

/// Strips the `Bearer ` prefix from a raw `Authorization` header value.
pub fn parse_bearer_token(authorization: Option<&str>) -> Option<&str> {
    authorization.and_then(|v| v.strip_prefix("Bearer "))
}

/// Extracts a Bearer token from an HTTP `Authorization` header.
pub fn extract_bearer(headers: &http::HeaderMap) -> Option<&str> {
    parse_bearer_token(
        headers
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
}

/// Extract the `oid` (object ID) claim from a JWT without signature validation.
/// Returns `None` if the token is malformed or the claim is absent.
pub fn decode_token_oid(token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Claims {
        oid: Option<String>,
    }
    jsonwebtoken::dangerous::insecure_decode::<Claims>(token)
        .ok()
        .and_then(|data| data.claims.oid)
        .filter(|s| !s.is_empty())
}

/// Non-success HTTP response classified into retryable (429) and
/// non-retryable categories. Client-specific error types implement
/// `From<HttpResponseError>` to map these into their own variants.
#[derive(Debug)]
pub enum HttpResponseError {
    RateLimited { retry_after: u64 },
    Api { status: u16, body: String },
}

/// Classify an HTTP response: return it as-is for 2xx, or convert
/// the status and body into an `HttpResponseError`.
pub async fn classify_response(
    resp: reqwest::Response,
) -> Result<reqwest::Response, HttpResponseError> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    if status == 429 {
        let retry_after = parse_retry_after(resp.headers());
        return Err(HttpResponseError::RateLimited { retry_after });
    }
    let body = match resp.text().await {
        Ok(text) if !text.is_empty() => text,
        Ok(_) => format!("(empty response body, status {status})"),
        Err(e) => format!("(failed to read response body: {e})"),
    };
    Err(HttpResponseError::Api { status, body })
}

pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> u64 {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Errors that may be transient and worth retrying.
pub trait Retryable {
    fn is_retryable(&self) -> bool;
    fn retry_after(&self) -> Option<u64>;
}

/// Compute a retry delay with jitter. Uses the `retry_after` hint for
/// rate-limited responses, exponential backoff (1s, 2s, 4s …) otherwise.
pub fn retry_delay(attempt: u32, retry_after: Option<u64>) -> Duration {
    let base = retry_after.unwrap_or(1u64 << attempt);
    let jitter = rand::random::<u64>() % (base / 2).max(1);
    Duration::from_secs(base + jitter)
}
