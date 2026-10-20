// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use http::StatusCode;
use snafu::prelude::*;
use storage_core_builder::traits::error::{status_with_error_info, ERROR_DOMAIN};
use storage_core_builder::traits::{SourceLocation, StorageErrorExt};
use storage_core_util::debug_limited;
use strum::{EnumCount, EnumDiscriminants, EnumString, FromRepr, IntoStaticStr, VariantNames};
use tonic::{Code, Status};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(transparent)]
    AddrParse {
        source: std::net::AddrParseError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Json {
        #[snafu(source(from(serde_json::Error, Arc::new)))]
        source: Arc<serde_json::Error>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    IntParse {
        source: std::num::ParseIntError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("invalid argument: {message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    InvalidMetadataValueBytes {
        #[snafu(source(from(tonic::metadata::errors::InvalidMetadataValueBytes, Arc::new)))]
        source: Arc<tonic::metadata::errors::InvalidMetadataValueBytes>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    InvalidMetadataValue {
        #[snafu(source(from(tonic::metadata::errors::InvalidMetadataValue, Arc::new)))]
        source: Arc<tonic::metadata::errors::InvalidMetadataValue>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    InvalidHeaderValueHttp {
        #[snafu(source(from(http::header::InvalidHeaderValue, Arc::new)))]
        source: Arc<http::header::InvalidHeaderValue>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Io {
        #[snafu(source(from(std::io::Error, Arc::new)))]
        source: Arc<std::io::Error>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Sdk {
        source: SdkError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    SystemTime {
        source: std::time::SystemTimeError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    TimeComponentRange {
        source: time::error::ComponentRange,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    TimeParse {
        source: time::error::Parse,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    UrlParse {
        source: url::ParseError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Utf8 {
        source: std::str::Utf8Error,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    FromUtf8 {
        source: std::string::FromUtf8Error,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Util { source: storage_core_util::error::Error },
}

impl Error {
    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Internal { message }.build()
    }

    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        InvalidArgument { message }.build()
    }

    #[track_caller]
    pub fn precondition_failed(message: impl Into<String>) -> Self {
        SdkError::precondition_failed(message).into()
    }

    #[track_caller]
    pub fn conflict(message: impl Into<String>) -> Self {
        SdkError::conflict(message).into()
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        SdkError::not_found(message).into()
    }

    pub fn is_precondition_failed(&self) -> bool {
        matches!(
            self,
            Self::Sdk {
                source,
                ..
            } if matches!(source, SdkError::PreconditionFailed { .. })
        )
    }

    /// Prepends a message to the error while preserving the error type
    #[track_caller]
    pub fn prepend_message(self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        match self {
            Self::Internal { message, loc } => Self::Internal {
                message: format!("{}: {}", prefix, message),
                loc,
            },
            Self::InvalidArgument { message, loc } => Self::InvalidArgument {
                message: format!("{}: {}", prefix, message),
                loc,
            },
            Self::Sdk { source, loc } => Self::Sdk {
                source: source.prepend_message(prefix),
                loc,
            },
            // For transparent errors, just return them as-is since we can't modify their messages
            other => other,
        }
    }
}

impl From<azure_core::Error> for Error {
    #[track_caller]
    fn from(err: azure_core::Error) -> Self {
        // Convert azure_core::Error → SdkError → Error::Sdk with location tracking
        SdkError::from(err).into()
    }
}

#[derive(Debug, Clone, Snafu, EnumDiscriminants, IntoStaticStr, VariantNames)]
#[strum_discriminants(derive(EnumCount, EnumString, FromRepr))]
pub enum SdkError {
    // UNKNOWN
    #[snafu(display("unknown err: {message}"))]
    Unknown {
        message: String,
        sdk_error_code: Option<String>,
    },

    // INVALID_ARGUMENT
    #[snafu(display("invalid argument or bad request: {message}"))]
    InvalidArgument {
        message: String,
        sdk_error_code: Option<String>,
    },

    // DEADLINE_EXCEEDED
    #[snafu(display("timeout: {message}"))]
    Timeout {
        message: String,
        sdk_error_code: Option<String>,
    },

    // NOT_FOUND
    #[snafu(display("not found: {message}"))]
    NotFound {
        message: String,
        sdk_error_code: Option<String>,
    },

    // ALREADY_EXISTS
    #[snafu(display("conflict: {message}"))]
    Conflict {
        message: String,
        sdk_error_code: Option<String>,
    },

    // PERMISSION_DENIED
    #[snafu(display("access denied: {message}"))]
    AccessDenied {
        message: String,
        sdk_error_code: Option<String>,
    },

    // FAILED_PRECONDITION
    #[snafu(display("precondition failed: {message}"))]
    PreconditionFailed {
        message: String,
        sdk_error_code: Option<String>,
    },

    // OUT_OF_RANGE
    #[snafu(display("invalid range: {message}"))]
    InvalidRange {
        message: String,
        sdk_error_code: Option<String>,
    },

    // UNIMPLEMENTED
    #[snafu(display("unimplemented: {message}"))]
    Unimplemented {
        message: String,
        sdk_error_code: Option<String>,
    },

    // INTERNAL
    #[snafu(display("internal err: {message}"))]
    Internal {
        message: String,
        sdk_error_code: Option<String>,
    },

    // UNAVAILABLE
    #[snafu(display("unavailable: {message}"))]
    Unavailable {
        message: String,
        sdk_error_code: Option<String>,
    },
}

impl From<azure_core::Error> for SdkError {
    #[track_caller]
    fn from(err: azure_core::Error) -> Self {
        debug_limited!("azure_core::Error: {:?}", err);
        match err.kind() {
            azure_storage::ErrorKind::HttpResponse { status, error_code } => {
                let message = error_code.as_ref().map_or(String::new(), |e| e.clone());
                let sdk_error_code = error_code.clone();
                match status {
                    // 400
                    azure_core::StatusCode::BadRequest => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::Unauthorized => SdkError::AccessDenied {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::PaymentRequired => SdkError::Unknown {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::Forbidden => SdkError::AccessDenied {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::NotFound => SdkError::NotFound {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::MethodNotAllowed => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::NotAcceptable => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::ProxyAuthenticationRequired => SdkError::AccessDenied {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::RequestTimeout => SdkError::Timeout {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::Conflict => SdkError::PreconditionFailed {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::Gone => SdkError::NotFound {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::LengthRequired => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::PreconditionFailed => SdkError::PreconditionFailed {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::PayloadTooLarge => SdkError::InvalidRange {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::UriTooLong => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::UnsupportedMediaType => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::RequestedRangeNotSatisfiable => SdkError::InvalidRange {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::ExpectationFailed => SdkError::PreconditionFailed {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::ImATeapot => SdkError::Unknown {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::MisdirectedRequest => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::UnprocessableEntity => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::Locked => SdkError::Unavailable {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::FailedDependency => SdkError::PreconditionFailed {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::TooEarly => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::UpgradeRequired => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::PreconditionRequired => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::TooManyRequests => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::RequestHeaderFieldsTooLarge => SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::UnavailableForLegalReasons => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },

                    // 500
                    azure_core::StatusCode::InternalServerError => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::NotImplemented => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::BadGateway => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::ServiceUnavailable => SdkError::Unavailable {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::GatewayTimeout => SdkError::Timeout {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::HttpVersionNotSupported => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::VariantAlsoNegotiates => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::InsufficientStorage => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::LoopDetected => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::NotExtended => SdkError::Internal {
                        message,
                        sdk_error_code,
                    },
                    azure_core::StatusCode::NetworkAuthenticationRequired => SdkError::AccessDenied {
                        message,
                        sdk_error_code,
                    },

                    // Any other status code would not be an error, so we should return an internal error
                    _ => SdkError::Internal {
                        message: format!("Non-error status code ({status}) in err: {message}"),
                        sdk_error_code,
                    },
                }
            },
            azure_storage::ErrorKind::Io => SdkError::Internal {
                message: "I/O error".to_string(),
                sdk_error_code: None,
            },
            azure_storage::ErrorKind::DataConversion => SdkError::Internal {
                message: "data conversion error".to_string(),
                sdk_error_code: None,
            },
            azure_storage::ErrorKind::Credential => SdkError::AccessDenied {
                message: "credential error".to_string(),
                sdk_error_code: None,
            },
            azure_storage::ErrorKind::MockFramework => SdkError::Unknown {
                message: "mock framework error".to_string(),
                sdk_error_code: None,
            },
            azure_storage::ErrorKind::Other => SdkError::Unknown {
                message: "unknown error".to_string(),
                sdk_error_code: None,
            },
        }
    }
}

impl SdkError {
    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
            sdk_error_code: None,
        }
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            sdk_error_code: None,
        }
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
            sdk_error_code: None,
        }
    }

    #[track_caller]
    pub fn precondition_failed(message: impl Into<String>) -> Self {
        Self::PreconditionFailed {
            message: message.into(),
            sdk_error_code: None,
        }
    }

    #[track_caller]
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
            sdk_error_code: None,
        }
    }

    /// Returns the raw SDK error code (e.g., "BlobNotFound", "AuthorizationFailure").
    pub fn sdk_error_code(&self) -> Option<&str> {
        match self {
            Self::Unknown { sdk_error_code, .. }
            | Self::InvalidArgument { sdk_error_code, .. }
            | Self::Timeout { sdk_error_code, .. }
            | Self::NotFound { sdk_error_code, .. }
            | Self::Conflict { sdk_error_code, .. }
            | Self::AccessDenied { sdk_error_code, .. }
            | Self::PreconditionFailed { sdk_error_code, .. }
            | Self::InvalidRange { sdk_error_code, .. }
            | Self::Unimplemented { sdk_error_code, .. }
            | Self::Internal { sdk_error_code, .. }
            | Self::Unavailable { sdk_error_code, .. } => sdk_error_code.as_deref(),
        }
    }

    /// Prepends a message to the error while preserving the error type
    #[track_caller]
    pub fn prepend_message(self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        match self {
            Self::Unknown {
                message,
                sdk_error_code,
            } => Self::Unknown {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::InvalidArgument {
                message,
                sdk_error_code,
            } => Self::InvalidArgument {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::Timeout {
                message,
                sdk_error_code,
            } => Self::Timeout {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::NotFound {
                message,
                sdk_error_code,
            } => Self::NotFound {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::Conflict {
                message,
                sdk_error_code,
            } => Self::Conflict {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::AccessDenied {
                message,
                sdk_error_code,
            } => Self::AccessDenied {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::PreconditionFailed {
                message,
                sdk_error_code,
            } => Self::PreconditionFailed {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::InvalidRange {
                message,
                sdk_error_code,
            } => Self::InvalidRange {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::Unimplemented {
                message,
                sdk_error_code,
            } => Self::Unimplemented {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::Internal {
                message,
                sdk_error_code,
            } => Self::Internal {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
            Self::Unavailable {
                message,
                sdk_error_code,
            } => Self::Unavailable {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
            },
        }
    }
}

impl From<Error> for Status {
    #[track_caller]
    fn from(err: Error) -> Self {
        Self::from(&err)
    }
}

impl From<&Error> for Status {
    #[track_caller]
    fn from(err: &Error) -> Self {
        let message = err.to_string();

        let (code, reason, domain): (Code, &str, &str) = match err {
            Error::AddrParse { .. } => (Code::Internal, "AddrParse", ERROR_DOMAIN),
            Error::Json { .. } => (Code::Internal, "Json", ERROR_DOMAIN),
            Error::Internal { .. } => (Code::Internal, "Internal", ERROR_DOMAIN),
            Error::IntParse { .. } => (Code::InvalidArgument, "IntParse", ERROR_DOMAIN),
            Error::InvalidArgument { .. } => (Code::InvalidArgument, "InvalidArgument", ERROR_DOMAIN),
            Error::InvalidMetadataValueBytes { .. } => {
                (Code::InvalidArgument, "InvalidMetadataValueBytes", ERROR_DOMAIN)
            },
            Error::InvalidMetadataValue { .. } => (Code::InvalidArgument, "InvalidMetadataValue", ERROR_DOMAIN),
            Error::InvalidHeaderValueHttp { .. } => (Code::InvalidArgument, "InvalidHeaderValueHttp", ERROR_DOMAIN),
            Error::Io { .. } => (Code::Internal, "Io", ERROR_DOMAIN),
            Error::Sdk { source, .. } => {
                let variant_name: &str = source.into();
                let reason = source.sdk_error_code().unwrap_or(variant_name);
                let code = match source {
                    SdkError::Unknown { .. } => Code::Unknown,
                    SdkError::InvalidArgument { .. } => Code::InvalidArgument,
                    SdkError::Timeout { .. } => Code::DeadlineExceeded,
                    SdkError::NotFound { .. } => Code::NotFound,
                    SdkError::Conflict { .. } => Code::AlreadyExists,
                    SdkError::AccessDenied { .. } => Code::PermissionDenied,
                    SdkError::PreconditionFailed { .. } => Code::FailedPrecondition,
                    SdkError::InvalidRange { .. } => Code::OutOfRange,
                    SdkError::Unimplemented { .. } => Code::Unimplemented,
                    SdkError::Internal { .. } => Code::Internal,
                    SdkError::Unavailable { .. } => Code::Unavailable,
                };
                return status_with_error_info(code, message, reason, "azure.blob", [("backend", "azure")]);
            },

            Error::SystemTime { .. } => (Code::Internal, "SystemTime", ERROR_DOMAIN),
            Error::TimeComponentRange { .. } => (Code::Internal, "TimeComponentRange", ERROR_DOMAIN),
            Error::TimeParse { .. } => (Code::InvalidArgument, "TimeParse", ERROR_DOMAIN),
            Error::UrlParse { .. } => (Code::InvalidArgument, "UrlParse", ERROR_DOMAIN),
            Error::Utf8 { .. } => (Code::InvalidArgument, "Utf8", ERROR_DOMAIN),
            Error::FromUtf8 { .. } => (Code::Internal, "FromUtf8", ERROR_DOMAIN),
            Error::Util { .. } => (Code::Internal, "Util", ERROR_DOMAIN),
        };

        status_with_error_info(code, message, reason, domain, Vec::<(&str, &str)>::new())
    }
}

impl From<Error> for http::StatusCode {
    #[track_caller]
    fn from(err: Error) -> Self {
        Self::from(&err)
    }
}

impl From<&Error> for http::StatusCode {
    #[track_caller]
    fn from(err: &Error) -> Self {
        match err {
            Error::AddrParse { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Json { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::IntParse { .. } => StatusCode::BAD_REQUEST,
            Error::InvalidArgument { .. } => StatusCode::BAD_REQUEST,
            Error::InvalidMetadataValueBytes { .. } => StatusCode::BAD_REQUEST,
            Error::InvalidMetadataValue { .. } => StatusCode::BAD_REQUEST,
            Error::InvalidHeaderValueHttp { .. } => StatusCode::BAD_REQUEST,
            Error::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Sdk { source, .. } => match source {
                SdkError::Unknown { .. } => StatusCode::INTERNAL_SERVER_ERROR,
                SdkError::InvalidArgument { .. } => StatusCode::BAD_REQUEST,
                SdkError::Timeout { .. } => StatusCode::REQUEST_TIMEOUT,
                SdkError::NotFound { .. } => StatusCode::NOT_FOUND,
                SdkError::Conflict { .. } => StatusCode::CONFLICT,
                SdkError::AccessDenied { .. } => StatusCode::FORBIDDEN,
                SdkError::PreconditionFailed { .. } => StatusCode::PRECONDITION_FAILED,
                SdkError::InvalidRange { .. } => StatusCode::RANGE_NOT_SATISFIABLE,
                SdkError::Unimplemented { .. } => StatusCode::NOT_IMPLEMENTED,
                SdkError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
                SdkError::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            },
            Error::SystemTime { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::TimeComponentRange { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::TimeParse { .. } => StatusCode::BAD_REQUEST,
            Error::UrlParse { .. } => StatusCode::BAD_REQUEST,
            Error::Utf8 { .. } => StatusCode::BAD_REQUEST,
            Error::FromUtf8 { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Util { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> Status {
        Status::from(self)
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::from(self)
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Error::Internal { message, .. } => message.capacity(),
                Error::InvalidArgument { message, .. } => message.capacity(),
                Error::Sdk { source, .. } => match source {
                    SdkError::Unknown {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::InvalidArgument {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::Timeout {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::NotFound {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::Conflict {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::AccessDenied {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::PreconditionFailed {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::InvalidRange {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::Unimplemented {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::Internal {
                        message,
                        sdk_error_code,
                        ..
                    }
                    | SdkError::Unavailable {
                        message,
                        sdk_error_code,
                        ..
                    } => message.capacity() + sdk_error_code.as_ref().map_or(0, |c| c.capacity()),
                },
                _ => 0,
            }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::AddrParse { loc, .. } => Some(loc.into()),
            Error::Json { loc, .. } => Some(loc.into()),
            Error::Internal { loc, .. } => Some(loc.into()),
            Error::IntParse { loc, .. } => Some(loc.into()),
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::InvalidMetadataValueBytes { loc, .. } => Some(loc.into()),
            Error::InvalidMetadataValue { loc, .. } => Some(loc.into()),
            Error::InvalidHeaderValueHttp { loc, .. } => Some(loc.into()),
            Error::Io { loc, .. } => Some(loc.into()),
            Error::Sdk { loc, .. } => Some(loc.into()),
            Error::SystemTime { loc, .. } => Some(loc.into()),
            Error::TimeComponentRange { loc, .. } => Some(loc.into()),
            Error::TimeParse { loc, .. } => Some(loc.into()),
            Error::UrlParse { loc, .. } => Some(loc.into()),
            Error::Utf8 { loc, .. } => Some(loc.into()),
            Error::FromUtf8 { loc, .. } => Some(loc.into()),
            Error::Util { source } => Some((&source.source_location()).into()),
        }
    }
}
