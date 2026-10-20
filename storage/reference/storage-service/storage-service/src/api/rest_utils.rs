// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use axum::{
    body::Body,
    http::Error as HttpError,
    http::{header::CONTENT_TYPE, response::Builder, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use serde::Serialize;
use tracing::error;

/// A response builder.
pub struct ResponseBuilder(Builder);

impl Default for ResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseBuilder {
    /// Create a response builder.
    pub fn new() -> Self {
        Self(Builder::new())
    }

    /// Set the response status.
    pub fn status(self, status: impl Into<StatusCode>) -> Self {
        Self(self.0.status(status))
    }

    /// Add a response header.
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        K: TryInto<HeaderName>,
        <K as TryInto<HeaderName>>::Error: Into<HttpError>,
        V: TryInto<HeaderValue>,
        <V as TryInto<HeaderValue>>::Error: Into<HttpError>,
    {
        Self(self.0.header(key, value))
    }

    /// Create a response with an arbitrary response body.
    pub fn body(self, body: impl Into<Body>) -> Result<Response, StatusCode> {
        self.0
            .body(body.into())
            .inspect_err(|error| error!(?error, "failed to build a response"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Create an empty response.
    pub fn empty(self) -> Result<Response, StatusCode> {
        self.body(Body::empty())
    }

    /// Create a JSON response.
    pub fn json(self, response: &impl Serialize) -> Result<Response, StatusCode> {
        self.header(CONTENT_TYPE, "application/json").body(Body::from(
            serde_json::to_vec(&response)
                .inspect_err(|error| error!(?error, "failed to serialize a response"))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        ))
    }
}
