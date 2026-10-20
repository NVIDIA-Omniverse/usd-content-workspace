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
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum Error {
    #[snafu(display("dfa lookup failed"))]
    DFALookup {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob resource identity is missing"))]
    ResourceIdentityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob resource location is missing"))]
    ResourceLocationMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob folder address is missing"))]
    FolderAddressMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob resource address is invalid: {source}"))]
    ResourceAddressInvalid {
        source: InvalidUrl,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob folder address is invalid: {source}"))]
    FolderAddressInvalid {
        source: InvalidUrl,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("azure blob resource identity is invalid: {source}"))]
    ResourceIdentityInvalid {
        source: InvalidUrl,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> http::StatusCode {
        use http::StatusCode;

        match self {
            Self::DFALookup { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ResourceIdentityMissing { .. }
            | Self::ResourceLocationMissing { .. }
            | Self::FolderAddressMissing { .. }
            | Self::ResourceAddressInvalid { .. }
            | Self::FolderAddressInvalid { .. }
            | Self::ResourceIdentityInvalid { .. } => StatusCode::BAD_REQUEST,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::DFALookup { .. } => Status::internal("unexpected error"),
            Self::ResourceIdentityMissing { .. }
            | Self::ResourceLocationMissing { .. }
            | Self::FolderAddressMissing { .. }
            | Self::ResourceAddressInvalid { .. }
            | Self::FolderAddressInvalid { .. }
            | Self::ResourceIdentityInvalid { .. } => Status::invalid_argument(self.to_string()),
        }
    }
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum InitError {
    #[snafu(display("endpoint path must be a single slash"))]
    EndpointPathInvalid {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("dfa build failed"))]
    DFABuild {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum InvalidUrl {
    #[snafu(display("path contains an unsafe character"))]
    PathCharacterInvalid {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("decoded path contains a control character"))]
    ControlCharacter {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("decoded path contains a backslash"))]
    Backslash {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("decoded path segment ends in a dot"))]
    TrailingDot {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("percent-encoded sequence is incomplete"))]
    PercentEncodingIncomplete {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("percent-encoded sequence contains an invalid hexadecimal digit"))]
    HexDigitInvalid {
        #[snafu(provide, implicit)]
        location: Location,
    },
}
