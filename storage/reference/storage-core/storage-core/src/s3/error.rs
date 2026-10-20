// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use aws_sdk_s3::error::ProvideErrorMetadata;
use snafu::{prelude::*, ErrorCompat, IntoError};
use storage_core_builder::traits::error::{status_with_error_info, ERROR_DOMAIN};
use storage_core_util::trace_limited;
use strum::{EnumCount, EnumDiscriminants, EnumString, FromRepr, IntoStaticStr, VariantNames};
use tonic::Code;

pub use http::StatusCode;
pub use tonic::Status;
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
    AwsByteStream {
        #[snafu(source(from(aws_smithy_types::byte_stream::error::Error, Arc::new)))]
        source: Arc<aws_smithy_types::byte_stream::error::Error>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    AwsDateTimeParse {
        #[snafu(source(from(aws_smithy_types::date_time::DateTimeParseError, Arc::new)))]
        source: Arc<aws_smithy_types::date_time::DateTimeParseError>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("presigned url error for context={context} source={source}"))]
    AwsPresigningConfig {
        context: String,
        #[snafu(source(from(aws_sdk_s3::presigning::PresigningConfigError, Arc::new)))]
        source: Arc<aws_sdk_s3::presigning::PresigningConfigError>,
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

    #[snafu(display("{message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("{message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
    #[snafu(display("sdk error source={source}"))]
    Sdk {
        #[snafu(source(from(SdkError, Box::new)))]
        source: Box<SdkError>,
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
    UrlParse {
        source: url::ParseError,
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
        Sdk {}.into_error(SdkError::precondition_failed(message))
    }

    #[track_caller]
    pub fn conflict(message: impl Into<String>) -> Self {
        Sdk {}.into_error(SdkError::conflict(message))
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        Sdk {}.into_error(SdkError::not_found(message))
    }

    pub fn is_precondition_failed(&self) -> bool {
        matches!(
            self,
            Self::Sdk {
                source,
                ..
            } if matches!(source.as_ref(), SdkError::PreconditionFailed { .. })
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
                source: Box::new((*source).prepend_message(prefix)),
                loc,
            },
            // For transparent errors, just return them as-is since we can't modify their messages
            other => other,
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

        // trace backtrace. http headers have limits and backtraces seem to easily
        // go over those limits and cause internal http errors
        if let Some(backtrace) = err.backtrace() {
            let stack: Vec<_> = backtrace.to_string().lines().map(String::from).collect();
            trace_limited!(?err, ?stack);
        }

        let (code, reason, domain): (Code, &str, &str) = match err {
            Error::AddrParse { .. } => (Code::Internal, "AddrParse", ERROR_DOMAIN),
            Error::AwsByteStream { .. } => (Code::Internal, "AwsByteStream", ERROR_DOMAIN),
            Error::AwsDateTimeParse { .. } => (Code::Internal, "AwsDateTimeParse", ERROR_DOMAIN),
            Error::AwsPresigningConfig { .. } => (Code::Internal, "AwsPresigningConfig", ERROR_DOMAIN),
            Error::Json { .. } => (Code::InvalidArgument, "Json", ERROR_DOMAIN),
            Error::Internal { .. } => (Code::Internal, "Internal", ERROR_DOMAIN),
            Error::InvalidArgument { .. } => (Code::InvalidArgument, "InvalidArgument", ERROR_DOMAIN),
            Error::Sdk { source, .. } => {
                let variant_name: &str = source.as_ref().into();
                let reason = source.sdk_error_code().unwrap_or(variant_name);
                let code = match source.as_ref() {
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
                return status_with_error_info(code, message, reason, "aws.s3", [("backend", "s3")]);
            },
            Error::SystemTime { .. } => (Code::Internal, "SystemTime", ERROR_DOMAIN),
            Error::UrlParse { .. } => (Code::InvalidArgument, "UrlParse", ERROR_DOMAIN),
            Error::Util { .. } => (Code::Internal, "Util", ERROR_DOMAIN),
        };

        status_with_error_info(code, message, reason, domain, Vec::<(&str, &str)>::new())
    }
}

// map AWS/Azure errors to gRPC status codes
//        Azure errors: https://learn.microsoft.com/en-us/rest/api/storageservices/blob-service-error-codes
//          AWS errors: https://docs.aws.amazon.com/AmazonS3/latest/API/ErrorResponses.html#ErrorCodeList
//   gRPC status codes: https://grpc.github.io/grpc/core/md_doc_statuscodes.html
#[derive(Debug, Clone, Snafu, EnumDiscriminants, IntoStaticStr, VariantNames)]
#[strum_discriminants(derive(EnumCount, EnumString, FromRepr))]
#[snafu(visibility(pub), context(suffix(false)), module)]
pub enum SdkError {
    // UNKNOWN
    #[snafu(display("unknown err: {message}"))]
    Unknown {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // INVALID_ARGUMENT
    #[snafu(display("invalid argument or bad request: {message}"))]
    InvalidArgument {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // DEADLINE_EXCEEDED
    #[snafu(display("timeout: {message}"))]
    Timeout {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // NOT_FOUND
    #[snafu(display("not found: {message}"))]
    NotFound {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // ALREADY_EXISTS
    #[snafu(display("conflict: {message}"))]
    Conflict {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // PERMISSION_DENIED
    #[snafu(display("access denied: {message}"))]
    AccessDenied {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // FAILED_PRECONDITION
    #[snafu(display("precondition failed: {message}"))]
    PreconditionFailed {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // OUT_OF_RANGE
    #[snafu(display("invalid range: {message}"))]
    InvalidRange {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // UNIMPLEMENTED
    #[snafu(display("unimplemented: {message}"))]
    Unimplemented {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // INTERNAL
    #[snafu(display("internal err: {message}"))]
    Internal {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    // UNAVAILABLE
    #[snafu(display("unavailable: {message}"))]
    Unavailable {
        message: String,
        sdk_error_code: Option<String>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl<E> From<aws_sdk_s3::error::SdkError<E>> for SdkError
where
    E: std::error::Error + ProvideErrorMetadata + 'static,
{
    #[track_caller]
    fn from(err: aws_sdk_s3::error::SdkError<E>) -> Self {
        let code = err.code();
        let sdk_message = err.message().unwrap_or("no error message available");
        let mut message = format!("code: {code:?}, message: {sdk_message}, error: {err}");
        let sdk_error_code = code.map(String::from);

        match &err {
            aws_sdk_s3::error::SdkError::ConstructionFailure(_) => sdk_error::Internal {
                message,
                sdk_error_code,
            }
            .build(),
            aws_sdk_s3::error::SdkError::TimeoutError(_) => sdk_error::Timeout {
                message,
                sdk_error_code,
            }
            .build(),
            aws_sdk_s3::error::SdkError::DispatchFailure(_) => sdk_error::Internal {
                message,
                sdk_error_code,
            }
            .build(),
            aws_sdk_s3::error::SdkError::ResponseError(_) => sdk_error::Internal {
                message,
                sdk_error_code,
            }
            .build(),
            aws_sdk_s3::error::SdkError::ServiceError(err) => {
                // service errors are the most interesting, since they are things
                // like not found or access denied. aws sdk is unfortunately not
                // consistent in reporting these errors. e.g. HeadObject does not return AccessDenied,
                // but code == None, whereas GetObject/PutObject will return code == AccessDenied.
                // Because of this, we will just look at the raw http status code returned.

                let http_status_code = err.raw().status().as_u16();
                message.push_str(&format!(", http code: {http_status_code}"));

                match http_status_code {
                    // sometimes AWS is insane and returns HTTP 200 for errors
                    // @see https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html
                    // @see https://github.com/awslabs/aws-sdk-rust/blob/main/sdk/s3/src/rest_xml_unwrapped_errors.rs#L14-L18
                    200 => {
                        // https://docs.aws.amazon.com/AmazonS3/latest/API/ErrorResponses.html
                        match code {
                            // Invalid Argument
                            Some("InvalidArgument") => sdk_error::InvalidArgument {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("MissingContentLength") => sdk_error::InvalidArgument {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("InvalidPart") => sdk_error::InvalidArgument {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("InvalidPartOrder") => sdk_error::InvalidArgument {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Permission Denied
                            Some("AccessDenied") => sdk_error::AccessDenied {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Not Found
                            Some("NoSuchKey") => sdk_error::NotFound {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("NoSuchUpload") => sdk_error::NotFound {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("NoSuchVersion") => sdk_error::NotFound {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Conflict
                            Some("ConditionalRequestConflict") => sdk_error::Conflict {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Precondition Failed
                            Some("PreconditionFailed") => sdk_error::PreconditionFailed {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Invalid Range
                            Some("InvalidRange") => sdk_error::InvalidRange {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Internal
                            Some("InternalError") => sdk_error::Internal {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            // Unavailable
                            Some("ServiceUnavailable") => sdk_error::Unavailable {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("SlowDown") => sdk_error::Unavailable {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            Some("503 SlowDown") => sdk_error::Unavailable {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                            _ => sdk_error::Unknown {
                                message,
                                sdk_error_code,
                            }
                            .build(),
                        }
                    },
                    400 => sdk_error::InvalidArgument {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    403 => sdk_error::AccessDenied {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    404 => sdk_error::NotFound {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    405 => sdk_error::AccessDenied {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    409 => sdk_error::Conflict {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    411 => sdk_error::InvalidArgument {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    412 => sdk_error::PreconditionFailed {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    416 => sdk_error::InvalidRange {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    500 => sdk_error::Internal {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    501 => sdk_error::Unimplemented {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    503 => sdk_error::Unavailable {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                    _ => sdk_error::Unknown {
                        message,
                        sdk_error_code,
                    }
                    .build(),
                }
            },
            _ => sdk_error::Unknown {
                message,
                sdk_error_code,
            }
            .build(),
        }
    }
}

impl SdkError {
    #[track_caller]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        sdk_error::InvalidArgument {
            message,
            sdk_error_code: None,
        }
        .build()
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        sdk_error::Internal {
            message,
            sdk_error_code: None,
        }
        .build()
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        sdk_error::NotFound {
            message,
            sdk_error_code: None,
        }
        .build()
    }

    #[track_caller]
    pub fn precondition_failed(message: impl Into<String>) -> Self {
        sdk_error::PreconditionFailed {
            message,
            sdk_error_code: None,
        }
        .build()
    }

    #[track_caller]
    pub fn conflict(message: impl Into<String>) -> Self {
        sdk_error::Conflict {
            message,
            sdk_error_code: None,
        }
        .build()
    }

    /// Returns the raw SDK error code (e.g., "PermanentRedirect", "NoSuchKey").
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
                loc,
            } => Self::Unknown {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::InvalidArgument {
                message,
                sdk_error_code,
                loc,
            } => Self::InvalidArgument {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::Timeout {
                message,
                sdk_error_code,
                loc,
            } => Self::Timeout {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::NotFound {
                message,
                sdk_error_code,
                loc,
            } => Self::NotFound {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::Conflict {
                message,
                sdk_error_code,
                loc,
            } => Self::Conflict {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::AccessDenied {
                message,
                sdk_error_code,
                loc,
            } => Self::AccessDenied {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::PreconditionFailed {
                message,
                sdk_error_code,
                loc,
            } => Self::PreconditionFailed {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::InvalidRange {
                message,
                sdk_error_code,
                loc,
            } => Self::InvalidRange {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::Unimplemented {
                message,
                sdk_error_code,
                loc,
            } => Self::Unimplemented {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::Internal {
                message,
                sdk_error_code,
                loc,
            } => Self::Internal {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
            Self::Unavailable {
                message,
                sdk_error_code,
                loc,
            } => Self::Unavailable {
                message: format!("{}: {}", prefix, message),
                sdk_error_code,
                loc,
            },
        }
    }
}

impl storage_core_builder::traits::error::StorageErrorExt for Error {
    fn tonic_status(&self) -> Status {
        Status::from(self)
    }

    fn http_status(&self) -> StatusCode {
        match self {
            Error::AddrParse { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::AwsByteStream { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::AwsDateTimeParse { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::AwsPresigningConfig { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Json { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InvalidArgument { .. } => StatusCode::BAD_REQUEST,
            Error::Sdk { source, .. } => match source.as_ref() {
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
            Error::UrlParse { .. } => StatusCode::BAD_REQUEST,
            Error::Util { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Error::Internal { message, .. } => message.capacity(),
                Error::InvalidArgument { message, .. } => message.capacity(),
                Error::Sdk { source, .. } => match source.as_ref() {
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

    fn source_location(&self) -> Option<storage_core_builder::traits::SourceLocation> {
        match self {
            Error::AddrParse { loc, .. } => Some(loc.into()),
            Error::AwsByteStream { loc, .. } => Some(loc.into()),
            Error::AwsDateTimeParse { loc, .. } => Some(loc.into()),
            Error::AwsPresigningConfig { loc, .. } => Some(loc.into()),
            Error::Json { loc, .. } => Some(loc.into()),
            Error::Internal { loc, .. } => Some(loc.into()),
            Error::InvalidArgument { loc, .. } => Some(loc.into()),
            Error::Sdk { loc, .. } => Some(loc.into()),
            Error::SystemTime { loc, .. } => Some(loc.into()),
            Error::UrlParse { loc, .. } => Some(loc.into()),
            Error::Util { source } => Some((&source.source_location()).into()),
        }
    }
}
