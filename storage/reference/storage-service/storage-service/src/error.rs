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

use axum::http::StatusCode;
use get_size2::GetSize;
use storage_core::traits::StorageErrorExt;
use thiserror::Error;
use tonic::Status;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct InvalidArgumentError {
    message: String,
    #[source]
    source: Option<BoxError>,
}

impl InvalidArgumentError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn new_with_source(message: impl Into<String>, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn resource_uri_invalid(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::new_with_source("resource uri is invalid", source)
    }

    pub fn user_metadata_value_missing() -> Self {
        Self::new("user metadata value is missing")
    }

    pub fn folder_address_missing() -> Self {
        Self::new("folder address is missing")
    }

    pub fn resource_identity_missing() -> Self {
        Self::new("resource identity is missing")
    }

    pub fn resource_address_is_not_url(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::new_with_source("resource address is not a valid url", source)
    }

    pub fn metadata_key_is_not_editable(key: impl std::fmt::Debug) -> Self {
        Self::new(format!("metadata key {:?} is not editable", key))
    }

    pub fn resource_location_missing() -> Self {
        Self::new("resource location is missing")
    }
}

impl StorageErrorExt for InvalidArgumentError {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.message.get_heap_size()
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    fn tonic_status(&self) -> Status {
        Status::invalid_argument(self.to_string())
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct UnexpectedError {
    message: String,
    #[source]
    source: Option<BoxError>,
}

impl UnexpectedError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn new_with_source(message: impl Into<String>, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn etag_response_header_missing() -> Self {
        Self::new("etag response header is missing")
    }

    pub fn metadata_encode_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::new_with_source("failed to encode metadata", source)
    }

    pub fn metadata_decode_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::new_with_source("failed to decode metadata", source)
    }

    pub fn resource_info_missing() -> Self {
        Self::new("resource info is missing from the response")
    }

    pub fn resource_identity_missing() -> Self {
        Self::new("resource identity is missing from the response")
    }

    pub fn url_rewrite_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::new_with_source("url rewrite failed", source)
    }
}

impl StorageErrorExt for UnexpectedError {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.message.get_heap_size()
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn tonic_status(&self) -> Status {
        Status::internal("unexpected error")
    }
}

#[derive(Error, Debug)]
#[error("{message}")]
pub struct ConfigError {
    message: String,
    #[source]
    source: Option<BoxError>,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn new_with_source(message: impl Into<String>, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}
