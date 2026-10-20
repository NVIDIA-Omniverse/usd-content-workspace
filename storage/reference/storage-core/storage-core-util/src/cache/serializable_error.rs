// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Serializable projection of a [`StorageError`] suitable for round-tripping
//! through the L2 (Redis) cache. Captures the gRPC code, message, and the
//! full `ErrorInfo` details (reason / domain / metadata) so the value can be
//! reconstructed into a `StorageError` after a deserialize without losing
//! the upstream domain-specific information.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use storage_core_types::error::{
    extract_error_info, status_with_error_info, SourceLocation, Status, StatusCode, StorageError, StorageErrorExt,
    ERROR_DOMAIN,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCachedError {
    pub code: i32,
    pub http_code: u16,
    pub message: String,
    pub reason: String,
    pub domain: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl SerializableCachedError {
    pub fn from_storage_error(err: &StorageError) -> Self {
        let status = err.tonic_status();
        let code = status.code() as i32;
        let message = status.message().to_owned();
        let http_code = err.http_status().as_u16();

        // Prefer the upstream `ErrorInfo` if present so domain-specific
        // reasons (e.g. "NoSuchKey" + domain "s3") survive the cache. Fall
        // back to a generic gRPC-code mapping so a Status without ErrorInfo
        // still has a non-empty reason.
        let (reason, domain, metadata) = extract_error_info(&status).unwrap_or_else(|| {
            (
                tonic_code_to_reason(status.code()).to_owned(),
                ERROR_DOMAIN.to_owned(),
                HashMap::new(),
            )
        });

        Self {
            code,
            http_code,
            message,
            reason,
            domain,
            metadata,
        }
    }
}

fn tonic_code_to_reason(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "Ok",
        tonic::Code::Cancelled => "Cancelled",
        tonic::Code::Unknown => "Unknown",
        tonic::Code::InvalidArgument => "InvalidArgument",
        tonic::Code::DeadlineExceeded => "DeadlineExceeded",
        tonic::Code::NotFound => "NotFound",
        tonic::Code::AlreadyExists => "AlreadyExists",
        tonic::Code::PermissionDenied => "PermissionDenied",
        tonic::Code::ResourceExhausted => "ResourceExhausted",
        tonic::Code::FailedPrecondition => "FailedPrecondition",
        tonic::Code::Aborted => "Aborted",
        tonic::Code::OutOfRange => "OutOfRange",
        tonic::Code::Unimplemented => "Unimplemented",
        tonic::Code::Internal => "Internal",
        tonic::Code::Unavailable => "Unavailable",
        tonic::Code::DataLoss => "DataLoss",
        tonic::Code::Unauthenticated => "Unauthenticated",
    }
}

impl std::fmt::Display for SerializableCachedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason, self.message)
    }
}

impl std::error::Error for SerializableCachedError {}

impl StorageErrorExt for SerializableCachedError {
    fn tonic_status(&self) -> Status {
        let code = tonic::Code::from(self.code);
        status_with_error_info(
            code,
            self.message.clone(),
            self.reason.clone(),
            self.domain.clone(),
            self.metadata.iter().map(|(k, v)| (k.clone(), v.clone())),
        )
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::from_u16(self.http_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn get_size(&self) -> usize {
        let metadata_size: usize = self.metadata.iter().map(|(k, v)| k.capacity() + v.capacity()).sum();
        std::mem::size_of::<Self>()
            + self.message.capacity()
            + self.reason.capacity()
            + self.domain.capacity()
            + metadata_size
    }

    fn source_location(&self) -> Option<SourceLocation> {
        None
    }
}
