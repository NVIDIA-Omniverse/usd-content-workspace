// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Error handling traits and types for the storage service.
//!
//! This module provides the foundation for error handling across the storage service,
//! defining how errors should be converted to different protocol formats (HTTP and gRPC).
//!
//! The main components are:
//! - [`StorageErrorExt`]: A trait that all storage errors must implement
//! - [`StorageError`]: A type-erased error type for use throughout the service
//! - [`SourceLocation`]: A generic struct for tracking where errors originate
//! - [`Result`]: A convenience type alias for results with storage errors

pub use http::StatusCode;
pub use tonic::Status;
use tonic_types::StatusExt;

/// Default error domain for storage-core errors.
pub const ERROR_DOMAIN: &str = "storage-core";

/// Creates a [`tonic::Status`] with structured [`ErrorInfo`] details.
///
/// The error details are encoded in the `grpc-status-details-bin` trailer,
/// following the standard `google.rpc.Status` pattern. Clients can extract
/// them using [`tonic_types::StatusExt::check_error_details()`].
///
/// Falls back to a plain [`Status`] if encoding fails.
///
/// [`ErrorInfo`]: tonic_types::ErrorDetails
pub fn status_with_error_info(
    code: tonic::Code,
    message: impl Into<String>,
    reason: impl Into<String>,
    domain: impl Into<String>,
    metadata: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
) -> tonic::Status {
    let message = message.into();
    let metadata: std::collections::HashMap<String, String> =
        metadata.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
    let details = tonic_types::ErrorDetails::with_error_info(reason, domain, metadata);
    tonic::Status::with_error_details(code, &message, details)
}

/// Convenience wrapper around [`status_with_error_info`] using the default
/// [`ERROR_DOMAIN`] and no additional metadata.
pub fn status_with_default_error_info(
    code: tonic::Code,
    message: impl Into<String>,
    reason: impl Into<String>,
) -> tonic::Status {
    status_with_error_info(code, message, reason, ERROR_DOMAIN, Vec::<(&str, &str)>::new())
}

/// Extract the `(reason, domain, metadata)` triple from a Status's
/// [`ErrorInfo`] details, if any are attached.
///
/// Returns `None` when the Status has no `ErrorInfo` (a Status produced
/// outside of [`status_with_error_info`] / [`status_with_default_error_info`]).
///
/// [`ErrorInfo`]: tonic_types::ErrorInfo
pub fn extract_error_info(
    status: &tonic::Status,
) -> Option<(String, String, std::collections::HashMap<String, String>)> {
    let info = status.get_details_error_info()?;
    Some((info.reason, info.domain, info.metadata))
}

/// Represents the source location where an error originated.
///
/// This struct provides a generic, crate-agnostic way to track error origins.
/// Implementers can populate it from any source - `snafu::Location`, `std::panic::Location`,
/// the `file!()`, `line!()`, `column!()` macros, or any other mechanism.
///
/// # Example
///
/// ```rust
/// use storage_core_types::error::SourceLocation;
///
/// let loc = SourceLocation::new("src/lib.rs", 42, 10);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// The file path where the error was created
    pub file: &'static str,
    /// The line number where the error was created
    pub line: u32,
    /// The column number where the error was created
    pub column: u32,
}

impl SourceLocation {
    /// Create a new source location with the given file, line, and column.
    pub const fn new(file: &'static str, line: u32, column: u32) -> Self {
        Self { file, line, column }
    }
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

impl From<&'static std::panic::Location<'static>> for SourceLocation {
    fn from(loc: &'static std::panic::Location<'static>) -> Self {
        Self {
            file: loc.file(),
            line: loc.line(),
            column: loc.column(),
        }
    }
}

impl From<&snafu::Location> for SourceLocation {
    fn from(loc: &snafu::Location) -> Self {
        Self {
            file: loc.file(),
            line: loc.line(),
            column: loc.column(),
        }
    }
}

/// Captures the current source location.
///
/// This macro expands to a [`SourceLocation`] containing the file, line, and column
/// where the macro is invoked.
#[macro_export]
macro_rules! source_location {
    () => {
        $crate::error::SourceLocation::new(file!(), line!(), column!())
    };
}

/// A trait that extends [`std::error::Error`] with storage service-specific error handling.
///
/// This trait requires implementors to provide methods for converting errors into
/// both HTTP and gRPC status codes, enabling consistent error responses across
/// different service protocols.
///
/// # Required Methods
///
/// - `tonic_status`: Convert the error into a gRPC [`tonic::Status`]
/// - `http_status`: Convert the error into an HTTP [`http::StatusCode`]
/// - `get_size`: Get the size of the error in bytes
pub trait StorageErrorExt: std::error::Error + Send + Sync + 'static {
    /// Convert this error into a gRPC [`tonic::Status`].
    ///
    /// This method should map the error to an appropriate gRPC status code
    /// and include relevant error details in the status message.
    fn tonic_status(&self) -> tonic::Status;

    /// Convert this error into an HTTP [`http::StatusCode`].
    ///
    /// This method should map the error to an appropriate HTTP status code
    /// that represents the nature of the error for HTTP clients.
    fn http_status(&self) -> http::StatusCode;

    /// Get the size of the error in bytes.
    ///
    /// This method should return the size of the error (both stack and heap) in bytes.
    ///
    /// There are Rust crates that define a similar trait, such as
    /// [get_size2](https://docs.rs/get-size2/latest/get_size2/). However, that trait requires
    /// the error to be `Sized`, which makes it not object safe. Implementers are encouraged to
    /// look at the [get_size_derive2](https://docs.rs/get-size-derive2/latest/get_size_derive2/) crate
    /// to help implement this method though.
    fn get_size(&self) -> usize;

    /// Get the source location where this error was created.
    ///
    /// This method returns the file, line, and column where the error originated,
    /// which is useful for debugging. Returns `None` by default if the error
    /// implementation doesn't track source location.
    ///
    /// Implementers can capture location using various mechanisms:
    /// - The [`source_location!`] macro
    /// - `snafu::Location` (convert via `.into()`)
    /// - `std::panic::Location` with `#[track_caller]`
    /// - Manual construction via [`SourceLocation::new`]
    fn source_location(&self) -> Option<SourceLocation> {
        None
    }
}

/// A type-erased error type for use throughout the storage service.
///
/// This type alias represents any error that implements [`StorageErrorExt`],
/// providing a uniform way to handle errors across different storage operations
/// without needing to know the specific error type at compile time.
pub type StorageError = Box<dyn StorageErrorExt>;

/// Automatic conversion from any [`StorageErrorExt`] implementor to [`StorageError`].
impl<T: StorageErrorExt> From<T> for StorageError {
    fn from(error: T) -> Self {
        Box::new(error) as StorageError
    }
}

/// A convenience type alias for [`std::result::Result`] with [`StorageError`].
pub type Result<T, E = StorageError> = std::result::Result<T, E>;

impl snafu::AsErrorSource for StorageError {
    fn as_error_source(&self) -> &(dyn std::error::Error + 'static) {
        self.as_ref()
    }
}

pub struct UnimplementedError {
    location: SourceLocation,
    message: Option<String>,
}

impl UnimplementedError {
    #[track_caller]
    pub fn new() -> Self {
        let caller = std::panic::Location::caller();
        Self {
            location: SourceLocation::from(caller),
            message: None,
        }
    }

    /// Like [`new`](Self::new) but with a caller-supplied message describing what
    /// is unimplemented (surfaced in the gRPC status and Display).
    #[track_caller]
    pub fn with_message(message: impl Into<String>) -> Self {
        let caller = std::panic::Location::caller();
        Self {
            location: SourceLocation::from(caller),
            message: Some(message.into()),
        }
    }
}

impl Default for UnimplementedError {
    #[track_caller]
    fn default() -> Self {
        Self::new()
    }
}

impl std::error::Error for UnimplementedError {}

impl std::fmt::Display for UnimplementedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.message {
            Some(message) => write!(f, "{message}"),
            None => write!(f, "Not implemented"),
        }
    }
}

impl std::fmt::Debug for UnimplementedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.message {
            Some(message) => write!(f, "{message}"),
            None => write!(f, "Not implemented"),
        }
    }
}

impl StorageErrorExt for UnimplementedError {
    fn tonic_status(&self) -> tonic::Status {
        status_with_default_error_info(
            tonic::Code::Unimplemented,
            self.message.as_deref().unwrap_or("Not implemented"),
            "Unimplemented",
        )
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::METHOD_NOT_ALLOWED
    }

    fn source_location(&self) -> Option<SourceLocation> {
        Some(self.location.clone())
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<UnimplementedError>()
    }
}
