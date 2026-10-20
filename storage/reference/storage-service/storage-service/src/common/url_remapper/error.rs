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
use storage_core::traits::{StorageError, StorageErrorExt};
use thiserror::Error;
use tonic::Status;

use crate::error::BoxError;

#[derive(Error, Debug)]
#[error("checkpoint metadata is missing for #{id}")]
pub struct CheckpointNotFoundError {
    id: u64,
}

impl CheckpointNotFoundError {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

impl StorageErrorExt for CheckpointNotFoundError {
    fn tonic_status(&self) -> Status {
        Status::not_found(self.to_string())
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::NOT_FOUND
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[derive(Error, Debug)]
#[error("checkpoint metadata is missing for #{id}")]
pub struct InvalidCheckpointError {
    id: u64,
}

impl InvalidCheckpointError {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

impl StorageErrorExt for InvalidCheckpointError {
    fn tonic_status(&self) -> Status {
        Status::invalid_argument(self.to_string())
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[derive(Error, Debug)]
pub enum UrlRewriteError {
    #[error("checkpoint metadata is missing for #{0}")]
    MissingCheckpointMetadata(u64),

    #[error("failed to parse checkpoint metadata")]
    CheckpointMetadataParseFailed(#[source] CheckpointMetadataParseError),

    #[error("failed to fetch checkpoint metadata: {0}")]
    FetchFailed(#[source] BoxError),

    #[error("response entry is missing")]
    BatchResponseEntryMissing,

    #[error("response entry outcome is missing")]
    BatchResponseEntryOutcomeMissing,

    #[error("response entry value is missing")]
    BatchResponseEntryValueMissing,

    #[error("batch request failed with code {0}: {1}")]
    BatchRequestFailed(i32, String),
}

impl UrlRewriteError {
    pub fn missing_checkpoint_metadata(checkpoint: u64) -> Self {
        Self::MissingCheckpointMetadata(checkpoint)
    }

    pub fn fetch_failed(source: StorageError) -> Self {
        Self::FetchFailed(source as BoxError)
    }

    pub fn into_not_found_error(self) -> StorageError {
        match self {
            Self::MissingCheckpointMetadata(id) => StorageError::from(CheckpointNotFoundError::new(id)),
            other => StorageError::from(other),
        }
    }

    pub fn into_invalid_argument_error(self) -> StorageError {
        match self {
            Self::MissingCheckpointMetadata(id) => StorageError::from(InvalidCheckpointError::new(id)),
            other => StorageError::from(other),
        }
    }
}

impl StorageErrorExt for UrlRewriteError {
    fn tonic_status(&self) -> Status {
        Status::internal(self.to_string())
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::FetchFailed(source) => source.get_heap_size(),
                Self::CheckpointMetadataParseFailed(_) => 0,
                Self::MissingCheckpointMetadata(checkpoint) => checkpoint.get_heap_size(),
                Self::BatchResponseEntryMissing => 0,
                Self::BatchResponseEntryOutcomeMissing => 0,
                Self::BatchResponseEntryValueMissing => 0,
                Self::BatchRequestFailed(_, message) => message.get_heap_size(),
            }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CheckpointMetadataParseError {
    #[error("$ is expected to be a list")]
    RootListExpected,

    #[error("$[{0}] is expected to be a struct")]
    StructExpected(usize),

    #[error("$[{0}].id is missing")]
    IdMissing(usize),

    #[error("id is expected to be a number")]
    IdInvalid(usize),

    #[error("$[{0}].resource_identity is missing")]
    ResourceIdentityMissing(usize),

    #[error("$[{0}].resource_identity is expected to be a string")]
    ResourceIdentityInvalid(usize),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum UrlParseError {
    #[error("scheme separator is missing")]
    MissingSchemeSeparator,

    #[error("scheme is empty")]
    EmptyScheme,

    #[error("authority is empty")]
    EmptyAuthority,

    #[error("unexpected query")]
    UnexpectedQuery,

    #[error("unexpected fragment")]
    UnexpectedFragment,

    #[error("url prefix path must have a trailing slash")]
    TrailingSlashMissing,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to parse the source url prefix")]
    SourcePrefixParseFailed(#[source] UrlParseError),

    #[error("failed to parse the target url prefix")]
    TargetPrefixParseFailed(#[source] UrlParseError),

    #[error(
        "rewrite rule source {rule_source:?} is a non-top-level mount but its parent {missing_parent:?} is neither a rewrite-rule source nor a configured backend endpoint; mounts must nest under an existing remapping or a configured endpoint"
    )]
    RuleParentNotMounted {
        rule_source: String,
        missing_parent: String,
    },
}
