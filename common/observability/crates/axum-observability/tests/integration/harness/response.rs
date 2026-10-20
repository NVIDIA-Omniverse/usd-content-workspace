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

use super::http::CHUNKED;

/// Comparable response struct for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) status: http::StatusCode,
    pub(crate) version: http::Version,
    pub(crate) headers: http::HeaderMap,
    pub(crate) body: Vec<Consumed>,
}

impl Response {
    pub(crate) fn builder(status: http::StatusCode, version: http::Version) -> ResponseBuilder {
        ResponseBuilder(Self {
            status,
            version,
            headers: http::HeaderMap::new(),
            body: Vec::new(),
        })
    }

    fn size(&self) -> usize {
        self.body
            .iter()
            .map(|item| match item {
                Consumed::Data(s) => s.len(),
                Consumed::Bytes(b) => b.len(),
                Consumed::Error(_) | Consumed::Trailers(_) => 0,
            })
            .sum()
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct ResponseBuilder(Response);

impl ResponseBuilder {
    pub(crate) fn build(self) -> (Response, f64) {
        let Self(mut response) = self;

        let resp_size = response.size();

        if let http::header::Entry::Vacant(entry) =
            response.headers.entry(http::header::CONTENT_LENGTH)
        {
            let src = resp_size.to_string();
            let value = http::HeaderValue::from_str(&src).unwrap();
            entry.insert(value);
        }

        (response, resp_size as f64)
    }

    pub(crate) fn stream(self) -> (Response, f64) {
        let Self(mut response) = self;

        let resp_size = response.size();

        if response.version < http::Version::HTTP_2 {
            response
                .headers
                .append(http::header::TRANSFER_ENCODING, CHUNKED);
        }

        (response, resp_size as f64)
    }

    pub(crate) fn body(mut self, body: Vec<Consumed>) -> Self {
        self.0.body = body;
        self
    }

    /// Replaces headers!
    #[expect(dead_code)]
    pub(crate) fn headers(mut self, headers: http::HeaderMap) -> Self {
        self.0.headers = headers;
        self
    }

    pub(crate) fn header<V>(mut self, k: http::HeaderName, v: V) -> Self
    where
        V: TryInto<http::HeaderValue>,
        V::Error: std::fmt::Debug,
    {
        let v = v.try_into().expect("invalid header value");
        self.0.headers.append(k, v);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Consumed {
    Data(String),
    Bytes(Vec<u8>),
    Trailers(http::HeaderMap),
    Error(String),
}

impl From<&str> for Consumed {
    fn from(value: &str) -> Self {
        Self::Data(value.to_string())
    }
}

impl From<http::HeaderMap> for Consumed {
    fn from(value: http::HeaderMap) -> Self {
        Self::Trailers(value)
    }
}

impl From<Result<&str, &str>> for Consumed {
    fn from(value: Result<&str, &str>) -> Self {
        match value {
            Ok(s) => Self::Data(s.to_string()),
            Err(s) => Self::Error(s.to_string()),
        }
    }
}

macro_rules! body {
    ($($item:expr),* $(,)?) => {
        vec![$(Consumed::from($item),)*]
    };
}
pub(crate) use body;

macro_rules! assert_resp {
    ($a:expr, $b:expr) => {
        let a: &Response = &$a;
        let b: &Response = &$b;
        ::pretty_assertions::assert_eq!(a, b);
    };
}
pub(crate) use assert_resp;
