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

use http::Method;
use std::convert::Infallible;
use std::pin::Pin;
use testing::body::TestBody;
use testing::stream::TestStream;

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Body {
    Text(String),
    Body(Box<dyn TestBody<Infallible>>),
    Stream(Pin<Box<dyn TestStream<&'static str>>>),
}

impl Body {
    fn len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Body(s) => s.len(),
            Self::Stream(s) => s.len(),
        }
    }
}

impl From<Body> for reqwest::Body {
    fn from(value: Body) -> Self {
        match value {
            Body::Text(s) => reqwest::Body::from(s),
            Body::Body(inner) => reqwest::Body::wrap(inner),
            Body::Stream(stream) => reqwest::Body::wrap_stream(stream),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Request {
    pub(super) method: http::Method,
    pub(super) path: String,
    pub(super) fragment: Option<String>,
    pub(super) headers: http::HeaderMap,
    pub(super) body: Option<Body>,
}

impl Request {
    fn new(method: Method, path: &str) -> Self {
        Self {
            method,
            path: path.to_string(),
            fragment: None,
            headers: http::HeaderMap::new(),
            body: None,
        }
    }

    pub(crate) fn builder(method: Method, path: &str) -> RequestBuilder {
        RequestBuilder(Self::new(method, path))
    }

    pub(crate) fn get(path: &str) -> RequestBuilder {
        Self::builder(Method::GET, path)
    }

    pub(crate) fn post(path: &str) -> RequestBuilder {
        Self::builder(Method::POST, path)
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct RequestBuilder(Request);

impl RequestBuilder {
    pub(crate) fn build(self) -> (Request, f64) {
        let Self(request) = self;

        let req_size = request.body.as_ref().map(Body::len).unwrap_or_default();

        (request, req_size as f64)
    }

    pub(crate) fn text(mut self, s: &str) -> Self {
        self.0.body = Some(Body::Text(s.to_string()));
        self
    }

    pub(crate) fn body(mut self, body: impl TestBody<Infallible>) -> Self {
        self.0.body = Some(Body::Body(Box::new(body)));
        self
    }

    pub(crate) fn stream(mut self, stream: impl TestStream<&'static str>) -> Self {
        self.0.body = Some(Body::Stream(Box::pin(stream)));
        self
    }

    #[expect(dead_code)]
    pub(crate) fn fragment(mut self, fragment: Option<&str>) -> Self {
        self.0.fragment = fragment.map(str::to_string);
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
