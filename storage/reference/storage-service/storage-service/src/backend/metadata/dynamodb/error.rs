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

use crate::error::BoxError;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum Error {
    #[snafu(display("metadata value is missing"))]
    UserMetadataValueMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata was modified"))]
    MetadataModified {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to delete an item"))]
    DeleteFailed {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to put an item"))]
    PutFailed {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to get items"))]
    BatchGetFailed {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to build a batch request"))]
    BatchGetBuildFailed {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("invalid row returned in a batch get response"))]
    BatchGetInvalidRow {
        source: InvalidRow,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to query items"))]
    QueryFailed {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("invalid row returned in a query response"))]
    QueryInvalidRow {
        source: InvalidRow,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request throttled by the backend"))]
    RequestThrottled {
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl Error {
    #[track_caller]
    pub fn metadata_modified(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::MetadataModifiedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn put_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::PutFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn delete_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::DeleteFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn batch_get_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::BatchGetFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn batch_get_build_error(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::BatchGetBuildFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn query_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::QueryFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn request_throttled(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        error::RequestThrottledSnafu.into_error(Box::new(source))
    }
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
            Self::RequestThrottled { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::UserMetadataValueMissing { .. } => Status::invalid_argument(self.to_string()),
            Self::MetadataModified { .. } => Status::failed_precondition(self.to_string()),
            Self::RequestThrottled { .. } => Status::unavailable(self.to_string()),
            _ => Status::internal("unexpected error"),
        }
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum InvalidRow {
    #[snafu(display("the value for the {column:?} column is missing"))]
    ColumnMissing {
        column: &'static str,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("the value of the {column:?} column is an unexpected type"))]
    ColumnValueTypeInvalid {
        column: &'static str,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode the metadata value"))]
    ValueDecodeFailed {
        source: prost::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl InvalidRow {
    #[track_caller]
    pub fn column_missing(column: &'static str) -> Self {
        invalid_row::ColumnMissingSnafu { column }.build()
    }

    #[track_caller]
    pub fn column_value_type_invalid(column: &'static str) -> Self {
        invalid_row::ColumnValueTypeInvalidSnafu { column }.build()
    }
}
