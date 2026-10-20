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
use storage_core::traits::StorageErrorExt;

#[derive(Snafu, Debug)]
#[snafu(context(suffix(false)), visibility(pub), module)]
pub enum Error {
    #[snafu(display("resource location is missing"))]
    ResourceLocationMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource uri is invalid"))]
    ResourceUriInvalid {
        source: CanonicalizationError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
#[snafu(context(suffix(false)), visibility(pub), module)]
pub enum CanonicalizationError {
    #[snafu(display("resource uri is invalid"))]
    ResourceUriInvalid {
        source: fluent_uri::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("authority is missing"))]
    AuthorityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected userinfo"))]
    UnexpectedUserInfo {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("etag query parameter is missing"))]
    ETagMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> axum::http::StatusCode {
        axum::http::StatusCode::BAD_REQUEST
    }

    fn tonic_status(&self) -> tonic::Status {
        tonic::Status::invalid_argument(self.to_string())
    }
}
