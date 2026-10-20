// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Errors specific to the small-object cache.
//!
//! [`Error::Uncacheable`] is an internal control-flow sentinel and must never be
//! retained as a cache entry. The small-object cache enforces that structurally
//! by disabling error retention for body fills.

use std::sync::Arc;

use snafu::prelude::*;
use storage_core_builder::traits::error::{
    extract_error_info, status_with_default_error_info, SourceLocation, StorageError, StorageErrorExt, ERROR_DOMAIN,
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

    #[snafu(display("uncacheable response"))]
    Uncacheable,

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

/// Returns whether `e` is the never-cached uncacheable-response sentinel.
///
/// Cached error wrappers are traversed because cache lookups preserve errors in
/// [`Error::CachedError`].
pub fn is_uncacheable(e: &StorageError) -> bool {
    let error = (e.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<Error>();

    match error {
        Some(Error::Uncacheable) => true,
        Some(Error::CachedError { source }) => is_uncacheable(source.as_ref()),
        _ => {
            // Defense in depth for a sentinel that was serialized by an older
            // or manually constructed cache. The concrete type is lost during
            // serialization, but ErrorInfo preserves its stable reason/domain.
            let status = e.tonic_status();
            status.code() == tonic::Code::Internal
                && extract_error_info(&status)
                    .is_some_and(|(reason, domain, _)| reason == "Uncacheable" && domain == ERROR_DOMAIN)
        },
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            Error::InvalidArgument { .. } => {
                status_with_default_error_info(tonic::Code::InvalidArgument, self.to_string(), "InvalidArgument")
            },
            Error::CachedError { source, .. } => source.tonic_status(),
            Error::Uncacheable => {
                status_with_default_error_info(tonic::Code::Internal, self.to_string(), "Uncacheable")
            },
            Error::Internal { .. } => {
                status_with_default_error_info(tonic::Code::Internal, self.to_string(), "Internal")
            },
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            Error::InvalidArgument { .. } => http::StatusCode::BAD_REQUEST,
            Error::CachedError { source, .. } => source.http_status(),
            Error::Uncacheable => http::StatusCode::INTERNAL_SERVER_ERROR,
            Error::Internal { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            Error::InvalidArgument { message, .. } => message.len(),
            Error::CachedError { source, .. } => source.get_size(),
            Error::Uncacheable => 0,
            Error::Internal { message, .. } => message.len(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::CachedError { source, .. } => source.source_location(),
            Error::Uncacheable => None,
            Error::Internal { loc, .. } => Some(loc.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage_core_util::cache::SerializableCachedError;

    #[test]
    fn detects_uncacheable_directly_and_through_arc() {
        let direct: StorageError = Box::new(Error::Uncacheable);
        assert!(is_uncacheable(&direct));

        let error: Arc<StorageError> = Arc::new(Box::new(Error::Uncacheable));

        assert!(is_uncacheable(error.as_ref()));
        assert_eq!(error.tonic_status().code(), tonic::Code::Internal);
        assert_eq!(error.http_status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn detects_uncacheable_through_cached_error() {
        let sentinel: Arc<StorageError> = Arc::new(Box::new(Error::Uncacheable));
        let wrapped: StorageError = Box::new(Error::cached_error(sentinel));

        assert!(is_uncacheable(&wrapped));
    }

    #[test]
    fn detects_uncacheable_after_serialization_erases_concrete_type() {
        let sentinel: StorageError = Box::new(Error::Uncacheable);
        let serialized: StorageError = Box::new(SerializableCachedError::from_storage_error(&sentinel));

        assert!(is_uncacheable(&serialized));
    }

    #[test]
    fn rejects_other_errors_including_cached_errors() {
        let internal: StorageError = Box::new(Error::internal("not the sentinel"));
        assert!(!is_uncacheable(&internal));

        let cached_internal: StorageError = Box::new(Error::cached_error(Arc::new(internal)));
        assert!(!is_uncacheable(&cached_internal));
    }
}
