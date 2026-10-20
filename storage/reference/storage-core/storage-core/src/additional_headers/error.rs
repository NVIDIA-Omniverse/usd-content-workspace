// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use snafu::prelude::*;
use storage_core_builder::traits::error::{status_with_default_error_info, SourceLocation, StorageErrorExt};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(transparent)]
    InvalidMetadataValueBytes {
        source: tonic::metadata::errors::InvalidMetadataValueBytes,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    InvalidMetadataValue {
        source: tonic::metadata::errors::InvalidMetadataValue,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    ToStr {
        source: tonic::metadata::errors::ToStrError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Utf8 {
        source: std::str::Utf8Error,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        let reason = match self {
            Error::InvalidMetadataValueBytes { .. } => "InvalidMetadataValueBytes",
            Error::InvalidMetadataValue { .. } => "InvalidMetadataValue",
            Error::ToStr { .. } => "ToStr",
            Error::Utf8 { .. } => "Utf8",
        };
        status_with_default_error_info(tonic::Code::InvalidArgument, self.to_string(), reason)
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::BAD_REQUEST
    }

    fn get_size(&self) -> usize {
        std::mem::size_of_val(self)
    }

    fn source_location(&self) -> Option<SourceLocation> {
        let loc = match self {
            Error::InvalidMetadataValueBytes { loc, .. } => loc,
            Error::InvalidMetadataValue { loc, .. } => loc,
            Error::ToStr { loc, .. } => loc,
            Error::Utf8 { loc, .. } => loc,
        };
        Some(loc.into())
    }
}
