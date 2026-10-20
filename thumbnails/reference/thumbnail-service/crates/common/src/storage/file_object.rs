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

//! Reading and writing object bytes over `FileObjectService`.
//!
//! Beside `stat.rs` and following the pattern it established — the shared
//! `OutboundClients` channel, the same auth headers, the same `tonic::Code`
//! mapping. Only the streaming transfers are new; nothing in the workspace had
//! used `Read` or `Write` before the worker needed them.
//!
//! Here rather than in `task-worker`, even though the worker is the only caller
//! today. The approved synchronous `createThumbnail` mode has to read a source
//! and write a thumbnail too, and leaving these in the worker binary would repeat
//! one layer down the mistake `crates/thumbnail-convert` was created to avoid:
//! an extraction owed to whoever writes the second caller.
//!
//! Both directions ask for `BODY` preference, matching the Python worker: chunks
//! inline over gRPC, no presigned-URL redirect and no multipart. A redirect
//! response is therefore a storage-side contract violation rather than a path to
//! implement, and is reported as such.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_client::FileObjectServiceClient;
use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::{
    read_response::ReplyType, write_request::WriteRequestType, write_response::WriteResponseType,
    Chunk, DownloadPreference, ReadRequest, ResourceIdentity, UploadPreference, WriteParameters,
    WriteRequest,
};
use tonic::Code;
use tracing::debug;

use crate::work_events::is_retryable_tonic_code;
use crate::OutboundClients;

/// Chunk size for uploads. Comfortably under the default 4 MiB gRPC message
/// limit while keeping the number of frames small for thumbnail-sized payloads.
const UPLOAD_CHUNK_BYTES: usize = 1024 * 1024;

/// Why a storage transfer failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageIoError {
    #[error("source object was not found in storage")]
    NotFound,

    #[error("source is larger than the configured {limit}-byte bound")]
    SourceTooLarge { limit: u64 },

    #[error("storage returned a redirect, but the transfer requested inline body delivery")]
    UnsupportedRedirect,

    #[error("storage write completed without returning a resource identity")]
    MissingWriteIdentity,

    #[error("outbound auth headers could not be built: {0}")]
    AuthHeaders(String),

    #[error("{operation} failed: {status}")]
    Rpc {
        operation: &'static str,
        status: tonic::Status,
    },
}

impl StorageIoError {
    /// Whether another delivery could succeed.
    ///
    /// A missing object, an oversized source, and a redirect the client cannot
    /// follow are all stable: retrying reproduces them. Transport and dependency
    /// failures are not.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::NotFound | Self::SourceTooLarge { .. } | Self::UnsupportedRedirect => false,
            // A token whose bytes are not a legal header value is malformed, not
            // absent; the next delivery gets the same one.
            Self::AuthHeaders(_) => false,
            // A write that reports no identity may be a transient partial
            // failure on the storage side; another attempt can produce one.
            Self::MissingWriteIdentity => true,
            Self::Rpc { status, .. } => is_retryable_tonic_code(status.code()),
        }
    }

    fn rpc(operation: &'static str, status: tonic::Status) -> Self {
        if status.code() == Code::NotFound {
            Self::NotFound
        } else {
            Self::Rpc { operation, status }
        }
    }
}

/// Read the bytes of the object named by `resource_identity`.
///
/// The byte bound is enforced as chunks arrive, not after the whole object is
/// buffered, so an oversized source stops the transfer instead of being paid for
/// in full and then rejected.
pub async fn read_object(
    outbound: &Arc<OutboundClients>,
    resource_identity: &str,
    auth_token: &str,
    max_source_bytes: u64,
    timeout: Duration,
) -> Result<Vec<u8>, StorageIoError> {
    let mut client = FileObjectServiceClient::new(outbound.traced_storage());

    let mut request = tonic::Request::new(ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: resource_identity.to_string(),
        }),
        download_preference: Some(DownloadPreference::Body as i32),
    });
    request.set_timeout(timeout);
    super::add_auth_headers(&mut request, auth_token).map_err(StorageIoError::AuthHeaders)?;

    let mut stream = client
        .read(request)
        .await
        .map_err(|status| StorageIoError::rpc("storage Read", status))?
        .into_inner();

    let mut body: Vec<u8> = Vec::new();
    while let Some(message) = stream.next().await {
        let message = message.map_err(|status| StorageIoError::rpc("storage Read", status))?;
        match message.reply_type {
            Some(ReplyType::Metadata(metadata)) => {
                // The declared size lets an oversized object be refused before a
                // single chunk is transferred.
                if let Some(size) = metadata.data_object_size {
                    if size > max_source_bytes {
                        return Err(StorageIoError::SourceTooLarge {
                            limit: max_source_bytes,
                        });
                    }
                    body.reserve(size as usize);
                }
            }
            Some(ReplyType::Chunk(chunk)) => {
                if body.len() as u64 + chunk.chunk.len() as u64 > max_source_bytes {
                    // The declared size can be absent or wrong, so the running
                    // total is the bound that actually holds.
                    return Err(StorageIoError::SourceTooLarge {
                        limit: max_source_bytes,
                    });
                }
                body.extend_from_slice(&chunk.chunk);
            }
            Some(ReplyType::Redirect(_)) => return Err(StorageIoError::UnsupportedRedirect),
            None => {}
        }
    }

    debug!(
        resource_identity,
        bytes = body.len(),
        "Read source object from storage"
    );
    Ok(body)
}

/// Write `bytes` to `destination_resource_address` and return the resource
/// identity storage assigned.
///
/// That returned identity is what the association carries. Synthesizing one
/// instead would associate a thumbnail nobody has confirmed exists, which is why
/// a write that completes without one is a failure.
pub async fn write_object(
    outbound: &Arc<OutboundClients>,
    destination_resource_address: &str,
    bytes: Vec<u8>,
    auth_token: &str,
    timeout: Duration,
) -> Result<String, StorageIoError> {
    let mut client = FileObjectServiceClient::new(outbound.traced_storage());

    let mut messages = Vec::with_capacity(2 + bytes.len() / UPLOAD_CHUNK_BYTES);
    messages.push(WriteRequest {
        write_request_type: Some(WriteRequestType::Params(WriteParameters {
            destination_resource_address: destination_resource_address.to_string(),
            // No optimistic locking: a duplicate delivery for the same
            // destination should overwrite and converge, not fail on a version
            // check. That convergence is the whole idempotency story.
            previous_version: None,
            data_object_size: bytes.len() as u64,
            upload_preference: Some(UploadPreference::Body as i32),
            // Both fields postdate the schema this crate used to compile, and
            // both are left at the value that reproduces the previous wire
            // request: unset `fail_if_exists` keeps last-writer-wins, which is
            // the convergence the absent optimistic locking above relies on,
            // and generated thumbnails carry no native storage metadata.
            fail_if_exists: None,
            native_metadata: Default::default(),
        })),
    });
    for chunk in bytes.chunks(UPLOAD_CHUNK_BYTES) {
        messages.push(WriteRequest {
            write_request_type: Some(WriteRequestType::Chunk(Chunk {
                chunk: chunk.to_vec(),
            })),
        });
    }

    let mut request = tonic::Request::new(tokio_stream::iter(messages));
    request.set_timeout(timeout);
    super::add_auth_headers(&mut request, auth_token).map_err(StorageIoError::AuthHeaders)?;

    let mut stream = client
        .write(request)
        .await
        .map_err(|status| StorageIoError::rpc("storage Write", status))?
        .into_inner();

    let mut identity = None;
    while let Some(message) = stream.next().await {
        let message = message.map_err(|status| StorageIoError::rpc("storage Write", status))?;
        match message.write_response_type {
            Some(WriteResponseType::WriteChunksAccepted(_)) => {}
            Some(WriteResponseType::ResourceInfo(info)) => {
                identity = info
                    .resource_identity
                    .map(|identity| identity.encoded_identity)
                    .filter(|identity| !identity.trim().is_empty());
            }
            Some(WriteResponseType::WriteRedirect(_))
            | Some(WriteResponseType::MultipartUpload(_)) => {
                return Err(StorageIoError::UnsupportedRedirect)
            }
            None => {}
        }
    }

    let identity = identity.ok_or(StorageIoError::MissingWriteIdentity)?;
    debug!(
        destination_resource_address,
        bytes = bytes.len(),
        "Wrote thumbnail object to storage"
    );
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_failures_are_not_retried() {
        assert!(!StorageIoError::NotFound.is_retryable());
        assert!(!StorageIoError::SourceTooLarge { limit: 10 }.is_retryable());
        assert!(!StorageIoError::UnsupportedRedirect.is_retryable());
    }

    #[test]
    fn dependency_failures_are_retried() {
        for code in [Code::Unavailable, Code::DeadlineExceeded, Code::Internal] {
            assert!(
                StorageIoError::Rpc {
                    operation: "storage Read",
                    status: tonic::Status::new(code, "injected"),
                }
                .is_retryable(),
                "{code:?} should be retried"
            );
        }
    }

    #[test]
    fn a_rejected_request_is_not_retried() {
        assert!(!StorageIoError::Rpc {
            operation: "storage Write",
            status: tonic::Status::invalid_argument("bad address"),
        }
        .is_retryable());
    }

    /// `NotFound` on any operation collapses to the dedicated variant, so the
    /// pipeline can label it `source_missing` rather than a generic RPC error.
    #[test]
    fn not_found_collapses_to_its_own_variant() {
        assert!(matches!(
            StorageIoError::rpc("storage Read", tonic::Status::not_found("gone")),
            StorageIoError::NotFound
        ));
    }
}
