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

use snafu::{Location, Snafu};

use crate::error::BoxError;

/// Credentials provider errors.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum Error {
    #[snafu(display("request to the token endpoint failed"))]
    TokenEndpointRequestFailed {
        #[snafu(source)]
        source: reqwest::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to build a token endpoint request"))]
    TokenEndpointRequestBuildFailed {
        #[snafu(source)]
        source: serde_urlencoded::ser::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected status returned in the token endpoint response {status_code}"))]
    TokenEndpointUnexpectedStatus {
        status_code: reqwest::StatusCode,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read the token endpoint response"))]
    TokenEndpointResponseReadFailed {
        #[snafu(source)]
        source: reqwest::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected token type {token_type:?}returned in the response"))]
    UnexpectedTokenType {
        token_type: String,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// Credentials provider initialization errors.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum InitError {
    #[snafu(display("header name is invalid"))]
    InvalidHeaderName {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("header value is invalid"))]
    InvalidHeaderValue {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("openid configuration request failed"))]
    OpenIdConfigRequestFailed {
        #[snafu(source)]
        source: reqwest::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected status code returned in the openid configuration response {status_code}"))]
    OpenIdConfigEndpointUnexpectedStatus {
        status_code: reqwest::StatusCode,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read the openid configuration response"))]
    OpenIdConfigResponseReadFailed {
        #[snafu(source)]
        source: reqwest::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
