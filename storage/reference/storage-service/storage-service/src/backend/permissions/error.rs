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

use snafu::{IntoError, Location, Snafu};
use storage_core::traits::StorageErrorExt;
use storage_service_permissions::resource::ParseError;
use tonic::metadata::errors::ToStrError;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum InitError {
    #[snafu(display("failed to initialize the client"))]
    ClientInit {
        #[snafu(source)]
        source: storage_service_permissions::client::error::InitError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("request filtering failed"))]
    Filter {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request credentials are missing"))]
    RequestCredentialsMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode request credentials"))]
    RequestCredentialsDecode {
        #[snafu(source)]
        source: ToStrError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("bearer request credentials expected"))]
    RequestCredentialsBearerExpected {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request credentials are invalid"))]
    RequestCredentialsInvalid {
        #[snafu(source)]
        source: storage_service_permissions::client::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource uri is invalid"))]
    RequestResourceUrlInvalid {
        #[snafu(source)]
        source: ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("an invalid resource url encountered in the response"))]
    ResponseResourceUrlInvalid {
        #[snafu(source)]
        source: ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("permission service request failed"))]
    PermissionRequestFailed {
        #[snafu(source)]
        source: storage_service_permissions::client::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("response size mismatch (expected {expected}, got {actual})"))]
    ResponseSizeMismatch {
        expected: usize,
        actual: usize,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource identity is missing from the request"))]
    RequestResourceIdentityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("folder address is missing from the request"))]
    RequestFolderAddressMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("access to the resource is forbidden"))]
    Forbidden {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata key is not editable"))]
    MetadataKeyNotEditable {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource location is missing"))]
    RequestResourceLocationMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("continuation handle is invalid"))]
    InvalidContinuationHandle {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl Error {
    #[track_caller]
    pub fn request_resource_identity_missing() -> Self {
        RequestCredentialsMissingSnafu.build()
    }

    #[track_caller]
    pub fn request_folder_address_missing() -> Self {
        RequestFolderAddressMissingSnafu.build()
    }

    #[track_caller]
    pub fn request_resource_url_invalid(source: ParseError) -> Self {
        RequestResourceUrlInvalidSnafu.into_error(source)
    }

    #[track_caller]
    pub fn response_resource_url_invalid(source: ParseError) -> Self {
        ResponseResourceUrlInvalidSnafu.into_error(source)
    }
}

impl StorageErrorExt for Error {
    fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;

        match self {
            Self::RequestCredentialsMissing { .. }
            | Self::RequestCredentialsDecode { .. }
            | Self::RequestCredentialsBearerExpected { .. }
            | Self::RequestCredentialsInvalid { .. } => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::RequestResourceUrlInvalid { .. }
            | Self::RequestResourceIdentityMissing { .. }
            | Self::RequestFolderAddressMissing { .. }
            | Self::MetadataKeyNotEditable { .. }
            | Self::RequestResourceLocationMissing { .. }
            | Self::InvalidContinuationHandle { .. } => StatusCode::BAD_REQUEST,
            Self::ResponseResourceUrlInvalid { .. }
            | Self::PermissionRequestFailed { .. }
            | Self::ResponseSizeMismatch { .. }
            | Self::Filter { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::RequestCredentialsMissing { .. }
            | Self::RequestCredentialsDecode { .. }
            | Self::RequestCredentialsBearerExpected { .. }
            | Self::RequestCredentialsInvalid { .. } => Status::unauthenticated("unauthenticated"),
            Self::Forbidden { .. } => Status::permission_denied(self.to_string()),
            Self::RequestResourceUrlInvalid { .. }
            | Self::RequestResourceIdentityMissing { .. }
            | Self::RequestFolderAddressMissing { .. }
            | Self::MetadataKeyNotEditable { .. }
            | Self::RequestResourceLocationMissing { .. }
            | Self::InvalidContinuationHandle { .. } => Status::invalid_argument(self.to_string()),
            Self::ResponseResourceUrlInvalid { .. }
            | Self::PermissionRequestFailed { .. }
            | Self::ResponseSizeMismatch { .. }
            | Self::Filter { .. } => Status::internal("unexpected error"),
        }
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}
