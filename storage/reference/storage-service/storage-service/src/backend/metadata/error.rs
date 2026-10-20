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

use std::{
    fmt::{Debug, Display},
    str::Utf8Error,
    sync::Arc,
};

use axum::http::StatusCode;
use snafu::{Location, Snafu};
use storage_core::traits::StorageErrorExt;
use thiserror::Error;
use tonic::Status;
use url::ParseError;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
#[error("Metadata modified")]
pub struct MetadataModifiedError {
    #[source]
    source: Option<BoxError>,
}

impl Default for MetadataModifiedError {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataModifiedError {
    pub fn new() -> Self {
        Self { source: None }
    }

    pub fn new_with_source(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Some(Box::new(source)),
        }
    }
}

impl StorageErrorExt for MetadataModifiedError {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::PRECONDITION_FAILED
    }

    fn tonic_status(&self) -> Status {
        Status::failed_precondition(self.to_string())
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum GetNormalizedUriError {
    #[snafu(display("failed to parse the resource uri"))]
    UriParse {
        source: fluent_uri::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("authority is missing"))]
    AuthorityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected userinfo"))]
    UnexpectedUserInfo {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource location is invalid"))]
    ResourceLocationInvalid {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("etag query parameter is missing"))]
    ETagMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module)]
pub enum GetMetadataKeyError {
    #[snafu(display("url decoding failed"))]
    UrlDecode {
        source: ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("uri is invalid"))]
    UriParse {
        source: fluent_uri::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("authority missing"))]
    AuthorityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("scheme is too long"))]
    SchemeTooLong {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("authority is too long"))]
    AuthorityTooLong {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("path is too long"))]
    PathTooLong {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("etag is too long"))]
    ETagTooLong {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("path component decoding failed"))]
    PathDecode {
        source: Utf8Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("query param decoding failed"))]
    ETagDecode {
        source: Utf8Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug)]
pub enum CacheError<T> {
    Init(Arc<T>),
    UnexpectedCacheValueType,
}

impl<T: Display> Display for CacheError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init(inner) => inner.fmt(f),
            Self::UnexpectedCacheValueType => f.write_str("unexpected cache value type"),
        }
    }
}

impl<T: std::error::Error> std::error::Error for CacheError<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Init(inner) => inner.source(),
            Self::UnexpectedCacheValueType => None,
        }
    }
}

impl<T: StorageErrorExt> StorageErrorExt for CacheError<T> {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Init(inner) => inner.get_size(),
                Self::UnexpectedCacheValueType => 0,
            }
    }

    fn http_status(&self) -> StatusCode {
        match self {
            Self::Init(inner) => inner.http_status(),
            Self::UnexpectedCacheValueType => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn tonic_status(&self) -> Status {
        match self {
            Self::Init(inner) => inner.tonic_status(),
            Self::UnexpectedCacheValueType => Status::internal("unexpected error"),
        }
    }
}
