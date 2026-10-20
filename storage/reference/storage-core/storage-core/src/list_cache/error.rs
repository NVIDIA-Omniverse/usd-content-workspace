// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use get_size2::GetSize;
use snafu::prelude::*;
use storage_core_builder::traits::error::status_with_default_error_info;
use storage_core_builder::traits::{SourceLocation, StorageError, StorageErrorExt};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), context(suffix(false)))]
pub(crate) enum Error {
    #[snafu(display("{source}"))]
    Cache { source: Arc<StorageError> },

    #[snafu(display("invalid argument: {message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("not found: {message}"))]
    NotFound {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl Error {
    #[track_caller]
    pub fn cache(source: Arc<StorageError>) -> Self {
        Self::Cache { source }
    }

    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        InvalidArgument { message }.build()
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Internal { message }.build()
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        NotFound { message }.build()
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            Error::Cache { source, .. } => source.tonic_status(),
            Error::InvalidArgument { .. } => {
                status_with_default_error_info(tonic::Code::InvalidArgument, self.to_string(), "InvalidArgument")
            },
            Error::Internal { .. } => {
                status_with_default_error_info(tonic::Code::Internal, self.to_string(), "Internal")
            },
            Error::NotFound { .. } => {
                status_with_default_error_info(tonic::Code::NotFound, self.to_string(), "NotFound")
            },
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            Error::Cache { source, .. } => source.http_status(),
            Error::InvalidArgument { .. } => http::StatusCode::BAD_REQUEST,
            Error::Internal { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
            Error::NotFound { .. } => http::StatusCode::NOT_FOUND,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            Error::Cache { source, .. } => source.get_size(),
            Error::InvalidArgument { message, .. } => message.get_size(),
            Error::Internal { message, .. } => message.get_size(),
            Error::NotFound { message, .. } => message.get_size(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::Cache { source, .. } => source.source_location(),
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::Internal { loc, .. } => Some(loc.into()),
            Error::NotFound { loc, .. } => Some(loc.into()),
        }
    }
}
