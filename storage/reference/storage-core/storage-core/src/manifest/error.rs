// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use snafu::prelude::*;

use crate::traits::error::{status_with_default_error_info, SourceLocation, StorageError, StorageErrorExt};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(display("{source}"))]
    CachedError { source: Arc<StorageError> },

    #[snafu(display("internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("Invalid argument: {message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("invalid manifest entry: {message}"))]
    InvalidManifestEntry {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("invalid resource identity: {message}"))]
    InvalidResourceIdentity {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("Method not allowed: {message}"))]
    MethodNotAllowed {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("invalid manifest path: {message}"))]
    NonManifestPath {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("object path not found in manifest: {message}"))]
    ObjectPathNotFound {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("unsupported manifest format version: expected {expected}, got {actual}"))]
    UnsupportedFormatVersion {
        expected: String,
        actual: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    UrlParse {
        source: url::ParseError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl Error {
    #[track_caller]
    pub fn cached_error(source: Arc<StorageError>) -> Self {
        Self::CachedError { source }
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Internal { message }.build()
    }

    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        InvalidArgument { message }.build()
    }

    #[track_caller]
    pub fn invalid_manifest_entry(message: impl Into<String>) -> Self {
        InvalidManifestEntry { message }.build()
    }

    #[track_caller]
    pub fn invalid_resource_identity(message: impl Into<String>) -> Self {
        InvalidResourceIdentity { message }.build()
    }

    #[track_caller]
    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        MethodNotAllowed { message }.build()
    }

    #[track_caller]
    pub fn non_manifest_path(message: impl Into<String>) -> Self {
        NonManifestPath { message }.build()
    }

    #[track_caller]
    pub fn object_path_not_found(message: impl Into<String>) -> Self {
        ObjectPathNotFound { message }.build()
    }

    #[track_caller]
    pub fn unsupported_format_version(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        UnsupportedFormatVersion { expected, actual }.build()
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        let (code, reason) = match self {
            Error::CachedError { source, .. } => return source.tonic_status(),
            Error::Internal { .. } => (tonic::Code::Internal, "Internal"),
            Error::InvalidArgument { .. } => (tonic::Code::InvalidArgument, "InvalidArgument"),
            Error::InvalidManifestEntry { .. } => (tonic::Code::InvalidArgument, "InvalidManifestEntry"),
            Error::InvalidResourceIdentity { .. } => (tonic::Code::InvalidArgument, "InvalidResourceIdentity"),
            Error::MethodNotAllowed { .. } => (tonic::Code::PermissionDenied, "MethodNotAllowed"),
            Error::NonManifestPath { .. } => (tonic::Code::InvalidArgument, "NonManifestPath"),
            Error::ObjectPathNotFound { .. } => (tonic::Code::NotFound, "ObjectPathNotFound"),
            Error::UnsupportedFormatVersion { .. } => (tonic::Code::InvalidArgument, "UnsupportedFormatVersion"),
            Error::UrlParse { .. } => (tonic::Code::InvalidArgument, "UrlParse"),
        };
        status_with_default_error_info(code, self.to_string(), reason)
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            Error::CachedError { source, .. } => source.http_status(),
            Error::Internal { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidArgument { .. } => http::StatusCode::BAD_REQUEST,
            Error::InvalidManifestEntry { .. } => http::StatusCode::BAD_REQUEST,
            Error::InvalidResourceIdentity { .. } => http::StatusCode::BAD_REQUEST,
            Error::MethodNotAllowed { .. } => http::StatusCode::FORBIDDEN,
            Error::NonManifestPath { .. } => http::StatusCode::BAD_REQUEST,
            Error::ObjectPathNotFound { .. } => http::StatusCode::NOT_FOUND,
            Error::UnsupportedFormatVersion { .. } => http::StatusCode::BAD_REQUEST,
            Error::UrlParse { .. } => http::StatusCode::BAD_REQUEST,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            Error::CachedError { source, .. } => source.get_size(),
            Error::Internal { message, .. } => message.len(),
            Error::InvalidArgument { message, .. } => message.len(),
            Error::InvalidManifestEntry { message, .. } => message.len(),
            Error::InvalidResourceIdentity { message, .. } => message.len(),
            Error::MethodNotAllowed { message, .. } => message.len(),
            Error::NonManifestPath { message, .. } => message.len(),
            Error::ObjectPathNotFound { message, .. } => message.len(),
            Error::UnsupportedFormatVersion { expected, actual, .. } => expected.len() + actual.len(),
            Error::UrlParse { .. } => std::mem::size_of::<Self>(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::CachedError { source, .. } => source.source_location(),
            Error::Internal { loc, .. } => Some(loc.into()),
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::InvalidManifestEntry { loc, .. } => Some(loc.into()),
            Error::InvalidResourceIdentity { loc, .. } => Some(loc.into()),
            Error::MethodNotAllowed { loc, .. } => Some(loc.into()),
            Error::NonManifestPath { loc, .. } => Some(loc.into()),
            Error::ObjectPathNotFound { loc, .. } => Some(loc.into()),
            Error::UnsupportedFormatVersion { loc, .. } => Some(loc.into()),
            Error::UrlParse { loc, .. } => Some(loc.into()),
        }
    }
}
