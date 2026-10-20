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

//! Error types for the thumbnail service - provides structured error types using thiserror for better error handling, debugging, and type safety across the codebase

use thiserror::Error;

/// Errors related to storage service operations
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Failed to connect to storage service at {endpoint}: {source}")]
    ConnectionFailed {
        endpoint: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("Invalid storage endpoint: {endpoint}")]
    InvalidEndpoint { endpoint: String },

    #[error("Storage service call failed: {0}")]
    ServiceCallFailed(#[from] tonic::Status),

    #[error("Resource not found: {resource_identity}")]
    ResourceNotFound { resource_identity: String },

    #[error("No metadata found for resource: {resource_identity}")]
    NoMetadataFound { resource_identity: String },

    #[error("Failed to parse metadata value: {0}")]
    MetadataParseError(String),

    #[error("Failed to create authorization header: {0}")]
    AuthorizationHeaderError(String),

    #[error("Authentication token required for {operation}")]
    AuthenticationRequired { operation: String },

    #[error("Metadata conversion failed: {0}")]
    MetadataConversion(String),

    #[error("File validation failed for {resource_identity}: {reason}")]
    FileValidationFailed {
        resource_identity: String,
        reason: String,
    },
}

/// Errors related to thumbnail operations
#[derive(Debug, Error)]
pub enum ThumbnailError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("No thumbnails found for resource {resource_identity}")]
    NoThumbnailsFound { resource_identity: String },

    #[error("Failed to parse thumbnail key: {key}")]
    InvalidThumbnailKey { key: String },

    #[error("Thumbnail filtering error: {0}")]
    FilterError(String),

    #[error("Failed to extract resource identity from metadata")]
    ResourceIdentityExtractionFailed,

    #[error("All {count} thumbnails were filtered out by criteria (width: {width:?}, height: {height:?}, format: {format:?})")]
    AllThumbnailsFiltered {
        count: usize,
        width: Option<u32>,
        height: Option<u32>,
        format: Option<String>,
    },
}

/// Errors related to authentication and authorization
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Failed to acquire authentication token: {0}")]
    TokenAcquisitionFailed(String),

    #[error("Token manager not configured")]
    TokenManagerNotConfigured,

    #[error("Authentication token expired")]
    TokenExpired,

    #[error("Authentication token required for {operation}")]
    TokenRequired { operation: String },

    #[error("Failed to refresh token: {0}")]
    TokenRefreshFailed(String),

    #[error("OAuth2 error: {0}")]
    OAuth2Error(String),
}

/// Errors related to notification/event processing
#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("Failed to connect to notification service at {endpoint}: {source}")]
    ConnectionFailed {
        endpoint: String,
        #[source]
        source: tonic::Status,
    },

    #[error("Event stream error: {0}")]
    StreamError(String),

    #[error("Failed to parse event: {0}")]
    EventParseError(String),

    #[error("Queue operation failed: {operation} - {reason}")]
    QueueOperationFailed { operation: String, reason: String },

    #[error("Failed to provision durable queue '{queue_id}': {status}")]
    QueueProvisioningFailed {
        queue_id: String,
        #[source]
        status: tonic::Status,
    },

    #[error("Authentication error: {0}")]
    AuthenticationError(#[from] AuthError),

    #[error("Storage error while processing event: {0}")]
    StorageError(#[from] StorageError),
}

// Conversion helpers for backward compatibility with Result<T, String>
impl From<StorageError> for String {
    fn from(err: StorageError) -> String {
        err.to_string()
    }
}

impl From<ThumbnailError> for String {
    fn from(err: ThumbnailError) -> String {
        err.to_string()
    }
}

impl From<AuthError> for String {
    fn from(err: AuthError) -> String {
        err.to_string()
    }
}

impl From<crate::auth::MissingAuthToken> for String {
    fn from(err: crate::auth::MissingAuthToken) -> String {
        err.to_string()
    }
}

impl From<NotificationError> for String {
    fn from(err: NotificationError) -> String {
        err.to_string()
    }
}
