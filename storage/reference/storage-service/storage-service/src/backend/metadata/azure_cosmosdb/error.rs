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

use crate::backend::metadata::error::CacheError;

/// Initialization error.
#[derive(Snafu, Debug)]
#[snafu(visibility(pub))]
pub enum InitError {
    #[snafu(display("account location is missing"))]
    AccountLocationMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to build a client"))]
    HttpClientBuildFailed {
        #[snafu(source)]
        source: reqwest::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// Operation errors.
#[derive(Snafu, Debug)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("user metadata value is missing"))]
    UserMetadataValueMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to encode the metadata value"))]
    MetadataValueEncodeFailed {
        #[snafu(source)]
        source: prost::EncodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata value is not base64-encoded"))]
    MetadataValueIsNotBase64 {
        #[snafu(source)]
        source: base64::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode the metadata value"))]
    MetadataValueDecodeFailed {
        #[snafu(source)]
        source: MetadataValueDecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata was modified"))]
    MetadataModified {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create a document client"))]
    DocumentClientCreateFailed {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("update document request failed"))]
    UpdateDocumentFailed {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("delete document request failed"))]
    DeleteDocumentFailed {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("query documents request failed"))]
    QueryDocumentsFailed {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("get document request failed"))]
    GetDocumentFailed {
        #[snafu(source)]
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("collision detected"))]
    Collision {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(transparent)]
    CacheError { source: CacheError<Self> },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;

        match self {
            Self::UserMetadataValueMissing { .. } => StatusCode::BAD_REQUEST,
            Self::MetadataModified { .. } => StatusCode::PRECONDITION_FAILED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::UserMetadataValueMissing { .. } => Status::invalid_argument(self.to_string()),
            Self::MetadataModified { .. } => Status::failed_precondition(self.to_string()),
            _ => Status::internal("unexpected error"),
        }
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum MetadataKeyDecodeError {
    #[snafu(display("base64 decode failed"))]
    Base64DecodeFailed {
        #[snafu(source)]
        source: base64::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata key is not a utf-8 string"))]
    StringConvertFailed {
        #[snafu(source)]
        source: std::str::Utf8Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum MetadataValueDecodeError {
    #[snafu(display("base64 decode failed"))]
    Base64DecodeFailed {
        #[snafu(source)]
        source: base64::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("protobuf message decode failed"))]
    ProstDecodeFailed {
        #[snafu(source)]
        source: prost::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
