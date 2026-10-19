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

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Snafu, Debug)]
#[snafu(context(suffix(false)), visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("unsupported uri scheme"))]
    UnsupportedScheme {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("hostname is missing"))]
    HostMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to resolve the host"))]
    Resolve {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to configure the connection"))]
    Configure {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("host resolution returned no records"))]
    ResolveEmptySet {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("invalid tls configuration"))]
    InvalidTlsConfig {
        source: tonic::transport::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("connect failed"))]
    Connect {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected error while parsing the origin string"))]
    UnexpectedOriginParse {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected error while parsing the endpoint uri"))]
    UnexpectedEndpointUriParse {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
