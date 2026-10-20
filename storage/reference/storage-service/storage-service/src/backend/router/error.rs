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

use axum::http::StatusCode;
use storage_core::traits::StorageErrorExt;
use tonic::Status;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("resource address is not supported")]
    UnsupportedAddress,

    #[error("resource identity is not supported")]
    UnsupportedIdentity,

    // Both resources are supported, but they are served by different backends. Nothing about the
    // request is malformed, so it is reported as unimplemented rather than invalid: the service
    // cannot transfer data between backends, and clients fall back to composing the operation
    // themselves.
    #[error(
        "copy between different storage backends is not supported, use a composition of Read and Write operations"
    )]
    CrossBackendCopy,

    #[error(
        "move between different storage backends is not supported, use a composition of Read, Write and Delete operations"
    )]
    CrossBackendMove,
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> StatusCode {
        match self {
            Self::UnsupportedAddress | Self::UnsupportedIdentity => StatusCode::BAD_REQUEST,
            Self::CrossBackendCopy | Self::CrossBackendMove => StatusCode::NOT_IMPLEMENTED,
        }
    }

    fn tonic_status(&self) -> Status {
        match self {
            Self::UnsupportedAddress | Self::UnsupportedIdentity => Status::invalid_argument(self.to_string()),
            Self::CrossBackendCopy | Self::CrossBackendMove => Status::unimplemented(self.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use storage_core::traits::StorageErrorExt;
    use tonic::Code;

    use crate::backend::router::error::Error;

    #[test]
    fn test_unsupported_resource_is_an_invalid_argument() {
        assert_eq!(Error::UnsupportedAddress.http_status(), StatusCode::BAD_REQUEST);
        assert_eq!(Error::UnsupportedAddress.tonic_status().code(), Code::InvalidArgument);

        assert_eq!(Error::UnsupportedIdentity.http_status(), StatusCode::BAD_REQUEST);
        assert_eq!(Error::UnsupportedIdentity.tonic_status().code(), Code::InvalidArgument);
    }

    #[test]
    fn test_cross_backend_operation_is_unimplemented() {
        // Clients pick their manual copy fallback by this status, so it is pinned here: the
        // rest test client cannot tell 501 apart from the 405 used elsewhere for unimplemented.
        assert_eq!(Error::CrossBackendCopy.http_status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(Error::CrossBackendCopy.tonic_status().code(), Code::Unimplemented);

        assert_eq!(Error::CrossBackendMove.http_status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(Error::CrossBackendMove.tonic_status().code(), Code::Unimplemented);
    }

    #[test]
    fn test_cross_backend_operation_names_the_fallback_composition() {
        assert_eq!(
            Error::CrossBackendCopy.to_string(),
            "copy between different storage backends is not supported, use a composition of Read and Write operations"
        );
        assert_eq!(
            Error::CrossBackendMove.to_string(),
            "move between different storage backends is not supported, \
             use a composition of Read, Write and Delete operations"
        );
    }
}
