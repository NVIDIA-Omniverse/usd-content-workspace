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

use std::str::FromStr;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use reqwest::header::AUTHORIZATION;
use tonic::metadata::MetadataMap;

pub mod grpc;
pub mod rest;

pub fn get_headers() -> HeaderMap<HeaderValue> {
    let mut headers = HeaderMap::new();
    for (name, value) in crate::common::headers::get_headers() {
        headers.insert(
            HeaderName::from_str(name.as_ref()).unwrap(),
            HeaderValue::from_str(value.as_ref()).unwrap(),
        );
    }
    headers
}

pub struct Request<T> {
    pub message: T,
    pub headers: HeaderMap,
}

impl<T> Request<T> {
    pub fn new_with_headers(message: T, headers: impl IntoIterator<Item = (HeaderName, HeaderValue)>) -> Self {
        Self {
            message,
            headers: headers.into_iter().collect(),
        }
    }

    pub fn into_parts(self) -> (T, HeaderMap) {
        (self.message, self.headers)
    }
}

impl<T> From<Request<T>> for tonic::Request<T> {
    fn from(value: Request<T>) -> Self {
        let metadata_map = MetadataMap::from_headers(value.headers);
        Self::from_parts(metadata_map, Default::default(), value.message)
    }
}

impl<T> From<T> for Request<T> {
    /// Create a request with default headers.
    fn from(message: T) -> Self {
        Self::new_with_headers(
            message,
            [(AUTHORIZATION, HeaderValue::from_static("Bearer valid.token"))],
        )
    }
}

pub struct WithoutAuthorization<T>(pub T);

impl<T> From<WithoutAuthorization<T>> for Request<T> {
    fn from(value: WithoutAuthorization<T>) -> Self {
        Self {
            message: value.0,
            headers: Default::default(),
        }
    }
}

pub struct WithInvalidAuthorization<T>(pub T);

impl<T> From<WithInvalidAuthorization<T>> for Request<T> {
    fn from(value: WithInvalidAuthorization<T>) -> Self {
        Self::new_with_headers(
            value.0,
            [(AUTHORIZATION, HeaderValue::from_static("Bearer invalid.token"))],
        )
    }
}
