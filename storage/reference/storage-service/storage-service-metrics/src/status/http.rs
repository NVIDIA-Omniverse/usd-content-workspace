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

use storage_core_util::observability::metrics::{BoundUIntCounter, UIntCounter};

use crate::{
    status::utils::BindResultExt,
    types::{AsObservable, Observe},
};

/// HTTP status value.
#[derive(Debug, Clone, Copy)]
pub struct Status(pub u16);

impl Status {
    pub const fn unexpected() -> Self {
        Self(0)
    }
}

/// HTTP operation result metrics.
pub struct ResultMetrics {
    status_200: BoundUIntCounter,
    status_201: BoundUIntCounter,
    status_202: BoundUIntCounter,
    status_204: BoundUIntCounter,
    status_206: BoundUIntCounter,
    status_2xx: BoundUIntCounter,
    status_300: BoundUIntCounter,
    status_301: BoundUIntCounter,
    status_302: BoundUIntCounter,
    status_304: BoundUIntCounter,
    status_307: BoundUIntCounter,
    status_308: BoundUIntCounter,
    status_3xx: BoundUIntCounter,
    status_400: BoundUIntCounter,
    status_401: BoundUIntCounter,
    status_403: BoundUIntCounter,
    status_404: BoundUIntCounter,
    status_405: BoundUIntCounter,
    status_406: BoundUIntCounter,
    status_408: BoundUIntCounter,
    status_409: BoundUIntCounter,
    status_410: BoundUIntCounter,
    status_412: BoundUIntCounter,
    status_416: BoundUIntCounter,
    status_422: BoundUIntCounter,
    status_429: BoundUIntCounter,
    status_4xx: BoundUIntCounter,
    status_500: BoundUIntCounter,
    status_501: BoundUIntCounter,
    status_502: BoundUIntCounter,
    status_503: BoundUIntCounter,
    status_504: BoundUIntCounter,
    status_5xx: BoundUIntCounter,
    status_unexpected: BoundUIntCounter,
}

impl ResultMetrics {
    /// Create a metric set for the specified base counter.
    pub fn new(base: &UIntCounter) -> Self {
        Self::new_with_bind(|result| base.bind_result(result))
    }

    /// Create a metrics set using custom bind callback.
    pub fn new_with_bind(bind: impl Fn(&'static str) -> BoundUIntCounter) -> Self {
        Self {
            status_200: bind("200"),
            status_201: bind("201"),
            status_202: bind("202"),
            status_204: bind("204"),
            status_206: bind("206"),
            status_2xx: bind("2xx"),
            status_300: bind("300"),
            status_301: bind("301"),
            status_302: bind("302"),
            status_304: bind("304"),
            status_307: bind("307"),
            status_308: bind("308"),
            status_3xx: bind("3xx"),
            status_400: bind("400"),
            status_401: bind("401"),
            status_403: bind("403"),
            status_404: bind("404"),
            status_405: bind("405"),
            status_406: bind("406"),
            status_408: bind("408"),
            status_409: bind("409"),
            status_410: bind("410"),
            status_412: bind("412"),
            status_416: bind("416"),
            status_422: bind("422"),
            status_429: bind("429"),
            status_4xx: bind("4xx"),
            status_500: bind("500"),
            status_501: bind("501"),
            status_502: bind("502"),
            status_503: bind("503"),
            status_504: bind("504"),
            status_5xx: bind("5xx"),
            status_unexpected: bind("Unexpected"),
        }
    }

    /// Return the counter for an HTTP status code.
    fn get_counter_for_status(&self, code: u16) -> &BoundUIntCounter {
        match code {
            200 => &self.status_200,
            201 => &self.status_201,
            202 => &self.status_202,
            204 => &self.status_204,
            206 => &self.status_206,
            300 => &self.status_300,
            301 => &self.status_301,
            302 => &self.status_302,
            304 => &self.status_304,
            307 => &self.status_307,
            308 => &self.status_308,
            400 => &self.status_400,
            401 => &self.status_401,
            403 => &self.status_403,
            404 => &self.status_404,
            405 => &self.status_405,
            406 => &self.status_406,
            408 => &self.status_408,
            409 => &self.status_409,
            410 => &self.status_410,
            412 => &self.status_412,
            416 => &self.status_416,
            422 => &self.status_422,
            429 => &self.status_429,
            500 => &self.status_500,
            501 => &self.status_501,
            502 => &self.status_502,
            503 => &self.status_503,
            504 => &self.status_504,
            s if (200..300).contains(&s) => &self.status_2xx,
            s if (300..400).contains(&s) => &self.status_3xx,
            s if (400..500).contains(&s) => &self.status_4xx,
            s if (500..600).contains(&s) => &self.status_5xx,
            _ => &self.status_unexpected,
        }
    }
}

impl Observe for &ResultMetrics {
    type Value = Status;

    fn observe(self, value: &Status) {
        self.get_counter_for_status(value.0).add(1);
    }
}

impl std::fmt::Debug for ResultMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultMetrics").finish_non_exhaustive()
    }
}

impl AsObservable<Status> for aws_smithy_runtime_api::http::StatusCode {
    fn as_observable(&self) -> Status {
        Status(self.as_u16())
    }
}

impl AsObservable<Status> for aws_smithy_runtime_api::client::orchestrator::HttpResponse {
    fn as_observable(&self) -> Status {
        self.status().as_observable()
    }
}

impl<E, R: AsObservable<Status>> AsObservable<Status> for aws_smithy_runtime_api::client::result::SdkError<E, R> {
    fn as_observable(&self) -> Status {
        if let Some(response) = self.raw_response() {
            response.as_observable()
        } else {
            Status::unexpected()
        }
    }
}

impl AsObservable<Status> for u16 {
    fn as_observable(&self) -> Status {
        Status(*self)
    }
}

impl AsObservable<Status> for azure_core::StatusCode {
    fn as_observable(&self) -> Status {
        Status(*self as u16)
    }
}

impl AsObservable<Status> for azure_core::Error {
    fn as_observable(&self) -> Status {
        if let Some(http_error) = self.as_http_error() {
            http_error.status().as_observable()
        } else {
            Status::unexpected()
        }
    }
}

impl<T: AsObservable<Status>> AsObservable<Status> for Result<T, azure_core::Error> {
    fn as_observable(&self) -> Status {
        match self {
            Ok(result) => result.as_observable(),
            Err(error) => error.as_observable(),
        }
    }
}

impl AsObservable<Status> for reqwest::StatusCode {
    fn as_observable(&self) -> Status {
        Status(self.as_u16())
    }
}

impl AsObservable<Status> for reqwest::Response {
    fn as_observable(&self) -> Status {
        self.status().as_observable()
    }
}
