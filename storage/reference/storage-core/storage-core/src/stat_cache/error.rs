// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use snafu::prelude::*;
use storage_core_builder::traits::error::{
    status_with_default_error_info, SourceLocation, StorageError, StorageErrorExt,
};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(display("invalid argument: {message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("{source}"))]
    CachedError { source: Arc<StorageError> },

    #[snafu(display("internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl Error {
    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        InvalidArgument { message }.build()
    }

    #[track_caller]
    pub fn cached_error(source: Arc<StorageError>) -> Self {
        Self::CachedError { source }
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Internal { message }.build()
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            Error::InvalidArgument { .. } => {
                status_with_default_error_info(tonic::Code::InvalidArgument, self.to_string(), "InvalidArgument")
            },
            Error::CachedError { source, .. } => source.tonic_status(),
            Error::Internal { .. } => {
                status_with_default_error_info(tonic::Code::Internal, self.to_string(), "Internal")
            },
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            Error::InvalidArgument { .. } => http::StatusCode::BAD_REQUEST,
            Error::CachedError { source, .. } => source.http_status(),
            Error::Internal { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            Error::InvalidArgument { message, .. } => message.len(),
            Error::CachedError { source, .. } => source.get_size(),
            Error::Internal { message, .. } => message.len(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::CachedError { source, .. } => source.source_location(),
            Error::Internal { loc, .. } => Some(loc.into()),
        }
    }
}
