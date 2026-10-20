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

use std::{fmt::Display, future::Future, time::Duration};

use percent_encoding::{utf8_percent_encode, AsciiSet, PercentEncode};
use reqwest::{
    header::{ETAG, IF_MATCH},
    Error, RequestBuilder, Response,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::time::sleep;
use url_escape::{PATH, QUERY};

const PATH_SEGMENT: AsciiSet = PATH.add(b'/').add(b'%');
const QUERY_PARAMETER: AsciiSet = QUERY.add(b'&').add(b'%');

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct OffsetDateTimeWrapper(#[serde(with = "time::serde::iso8601")] pub OffsetDateTime);

impl OffsetDateTimeWrapper {
    pub fn new(value: OffsetDateTime) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> OffsetDateTime {
        self.0
    }
}

impl From<OffsetDateTimeWrapper> for OffsetDateTime {
    fn from(value: OffsetDateTimeWrapper) -> Self {
        value.into_inner()
    }
}

pub trait RequestBuilderExt {
    fn maybe_if_match_header(self, value: Option<&str>) -> Self;
    fn send_with_retry(self) -> impl Future<Output = Result<Response, Error>>;
}

impl RequestBuilderExt for RequestBuilder {
    fn maybe_if_match_header(self, value: Option<&str>) -> Self {
        if let Some(value) = value {
            self.header(IF_MATCH, value)
        } else {
            self
        }
    }

    async fn send_with_retry(self) -> Result<Response, Error> {
        let mut retries = 4usize;
        loop {
            let response_or_error = self.try_clone().expect("failed to clone a request").send().await;
            match response_or_error {
                Ok(response) => return Ok(response),
                Err(error) => {
                    if retries == 0 {
                        return Err(error);
                    } else {
                        retries -= 1;
                        sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
    }
}

pub trait ResponseExt {
    fn to_tonic_status(self) -> tonic::Status;
    fn maybe_etag_header(&self) -> Option<String>;

    #[track_caller]
    fn etag_header(&self) -> String {
        self.maybe_etag_header().expect("etag header value is missing")
    }
}

impl ResponseExt for Response {
    fn to_tonic_status(self) -> tonic::Status {
        use reqwest::StatusCode;
        use tonic::Status;

        match self.status() {
            StatusCode::OK => Status::ok(""),
            StatusCode::CREATED => Status::ok(""),
            StatusCode::ACCEPTED => Status::ok(""),
            StatusCode::NO_CONTENT => Status::ok(""),
            StatusCode::MULTIPLE_CHOICES => Status::ok(""),
            StatusCode::NOT_FOUND => Status::not_found(""),
            StatusCode::PRECONDITION_FAILED => Status::failed_precondition(""),
            StatusCode::CONFLICT => Status::already_exists(""),
            StatusCode::RANGE_NOT_SATISFIABLE => Status::out_of_range(""),
            StatusCode::REQUEST_TIMEOUT => Status::deadline_exceeded(""),
            StatusCode::SERVICE_UNAVAILABLE => Status::unavailable(""),
            StatusCode::INTERNAL_SERVER_ERROR => Status::internal(""),
            StatusCode::BAD_REQUEST => Status::invalid_argument(""),
            StatusCode::UNAUTHORIZED => Status::unauthenticated(""),
            StatusCode::FORBIDDEN => Status::permission_denied(""),
            StatusCode::TOO_MANY_REQUESTS => Status::resource_exhausted(""),
            StatusCode::NOT_IMPLEMENTED => Status::unimplemented(""),
            StatusCode::METHOD_NOT_ALLOWED => Status::unimplemented(""),
            _ => Status::unknown(""),
        }
    }

    #[track_caller]
    fn maybe_etag_header(&self) -> Option<String> {
        self.headers()
            .get(ETAG)
            .map(|value| value.to_str().expect("etag header value is malformed").into())
    }
}

pub struct QueryParams<'a, T> {
    items: &'a [T],
}

impl<'a, T: AsRef<str>> Display for QueryParams<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (idx, item) in self.items.iter().enumerate() {
            if idx == 0 {
                write!(f, "?{}", utf8_percent_encode(item.as_ref(), &QUERY_PARAMETER))?;
            } else {
                write!(f, "&{}", utf8_percent_encode(item.as_ref(), &QUERY_PARAMETER))?;
            }
        }

        Ok(())
    }
}

pub fn encode_path_segment<'a>(value: &'a str) -> PercentEncode<'a> {
    utf8_percent_encode(value, &PATH_SEGMENT)
}

pub fn encode_query_params<'a, T>(items: &'a [T]) -> QueryParams<'a, T> {
    QueryParams { items }
}
