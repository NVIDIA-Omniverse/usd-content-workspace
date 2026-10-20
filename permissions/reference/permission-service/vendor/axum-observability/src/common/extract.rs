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

use std::borrow::Cow;

const X_FORWARDED_HOST: http::HeaderName = http::HeaderName::from_static("x-forwarded-host");

/// Returns the canonical method name.
pub fn method(method: &http::Method) -> Option<&'static str> {
    match *method {
        http::Method::CONNECT => Some("CONNECT"),
        http::Method::DELETE => Some("DELETE"),
        http::Method::GET => Some("GET"),
        http::Method::HEAD => Some("HEAD"),
        http::Method::OPTIONS => Some("OPTIONS"),
        http::Method::PATCH => Some("PATCH"),
        http::Method::POST => Some("POST"),
        http::Method::PUT => Some("PUT"),
        http::Method::TRACE => Some("TRACE"),
        _ if method.as_str() == "QUERY" => Some("QUERY"),
        _ => None,
    }
}

/// Returns the HTTP version name.
pub fn version(version: http::Version) -> &'static str {
    // see https://github.com/open-telemetry/semantic-conventions/blob/v1.40.0/docs/http/http-metrics.md
    match version {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2",
        http::Version::HTTP_3 => "3",
        _ => "?", // cov-exclude [line]: impossible to construct other HTTP versions
    }
}

fn header_to_str(header: &http::HeaderValue) -> Option<&str> {
    header.to_str().ok()
}

/// Extract the `host` directive from a `Forwarded` header value (RFC 7239).
///
/// The `Forwarded` header contains semicolon-separated parameters. Multiple
/// forwarded elements may be comma-separated; only the first is considered.
fn forwarded_host(value: &http::HeaderValue) -> Option<&str> {
    let s = header_to_str(value)?;
    // take only the first forwarded-element (before any comma)
    let first = s.split(',').next()?;
    first
        .split(';')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            if key.trim().eq_ignore_ascii_case("host") {
                Some(value.trim().trim_matches('"'))
            } else {
                None
            }
        })
        .next()
}

/// Extract the server address from the request, per the OTel semconv.
///
/// See <https://opentelemetry.io/docs/specs/semconv/http/http-spans/#setting-serveraddress-and-serverport-attributes>.
pub fn req_http_host(parts: &http::request::Parts) -> Option<&str> {
    // 1. Forwarded#host
    if let Some(host) = parts
        .headers
        .get(http::header::FORWARDED)
        .and_then(forwarded_host)
    {
        return Some(host);
    }
    // 2. X-Forwarded-Host
    if let Some(host) = parts.headers.get(X_FORWARDED_HOST).and_then(header_to_str) {
        return Some(host);
    }
    // 3. URI authority (HTTP/2 :authority pseudo-header; hyper stores it here
    // rather than in the Host header)
    if let Some(host) = parts.uri.authority().map(|a| a.as_str()) {
        return Some(host);
    }
    // 4. Host header
    if let Some(host) = parts
        .headers
        .get(http::header::HOST)
        .and_then(header_to_str)
    {
        return Some(host);
    }
    None
}

/// Extract the user agent from a request's headers.
pub fn req_user_agent_from_headers(headers: &http::HeaderMap) -> Option<&str> {
    headers
        .get(http::header::USER_AGENT)
        .and_then(header_to_str)
}

/// Extract the matched route from request extensions, if available.
pub fn route(extensions: &http::Extensions) -> Option<&str> {
    #[cfg(feature = "axum")]
    {
        extensions
            .get::<axum::extract::MatchedPath>()
            .map(|p| p.as_str())
    }
    #[cfg(not(feature = "axum"))]
    {
        let _ = extensions;
        None
    }
}

/// Returns the OTel span name: `{method} {route}` if a route is available,
/// otherwise just `{method}`.
///
/// <https://opentelemetry.io/docs/specs/semconv/http/http-spans/#name>
pub fn span_name<'a>(method: &'a str, route: Option<&str>) -> Cow<'a, str> {
    match route {
        Some(path) => Cow::Owned(format!("{method} {path}")),
        None => Cow::Borrowed(method),
    }
}

#[cfg(test)]
mod tests;
