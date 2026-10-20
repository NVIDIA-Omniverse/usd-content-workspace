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

//! Shared storage-service access helpers.
//!
//! The common crate talks to storage for metadata reads/writes used by
//! thumbnail associations and query paths.

pub mod association;
pub mod blob;
pub mod file_object;
pub mod metadata;
pub mod stat;

pub use blob::{delete_thumbnail_blobs, BlobDeletionFailure, BlobDeletionOutcome};
pub use file_object::{read_object, write_object, StorageIoError};

pub use association::{
    disassociate_thumbnail, store_association, AssociationError, AssociationFailure,
    AssociationInvariant, AssociationMessageParams, AssociationRequest, AssociationResource,
    AssociationSpec, AssociationTargetKind, AssociationValidationError, DisassociateAbort,
    DisassociateEntryError, DisassociateEntryFailure, DisassociateOutcome, DisassociateRequest,
    DisassociateValidationError, DisassociatedThumbnail,
};
pub use metadata::{
    get_all_storage_metadata, remove_storage_metadata, remove_storage_metadata_batch,
    update_storage_metadata, StorageMetadataBatchEntryOutcome, StorageMetadataBatchOutcome,
    StorageMetadataError,
};
pub use stat::{stat_file_object, StatError};

use crate::current_runtime_config;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static STORAGE_CHANNEL: OnceLock<tonic::transport::Channel> = OnceLock::new();

/// Whether to skip the process-wide channel cache. See [`bypass_channel_cache_for_tests`].
static BYPASS_CHANNEL_CACHE: AtomicBool = AtomicBool::new(false);

/// Stop caching the storage channel process-wide, for the rest of this process.
///
/// A cached lazy channel's connection task belongs to the runtime that first used it.
/// The service runs one runtime for the process, so caching is free there — but every
/// `#[tokio::test]` builds its own runtime and drops it at the end of the test, which
/// leaves the cached channel serving "runtime dropped the dispatch task" to every test
/// that follows. Tests therefore take a fresh channel per call.
///
/// A `#[cfg(test)]` gate cannot express this: `thumbnail-service` depends on
/// `thumbnail-common` as a regular dependency, so the gate would be off exactly when
/// `thumbnail-service`'s own tests run. Called by `test_fixtures::init_test_runtime_config`.
#[doc(hidden)]
#[cfg(any(test, feature = "test-utils"))]
pub fn bypass_channel_cache_for_tests() {
    BYPASS_CHANNEL_CACHE.store(true, Ordering::SeqCst);
}

fn build_storage_endpoint() -> Result<tonic::transport::Endpoint, String> {
    let runtime = current_runtime_config();
    let storage_endpoint = runtime.endpoints.storage_grpc_endpoint.clone();
    let channel = &runtime.storage_channel;

    // Prepend only when there is no scheme. Prepending unconditionally turned an
    // `https://host:443` setting into `http://https://host:443`; the endpoint
    // parser now refuses that setting outright, and this keeps the builder honest
    // if it is ever reached another way.
    let endpoint =
        if storage_endpoint.starts_with("http://") || storage_endpoint.starts_with("https://") {
            storage_endpoint.clone()
        } else {
            format!("http://{}", storage_endpoint)
        };

    tonic::transport::Endpoint::from_shared(endpoint.clone())
        .map_err(|e| format!("Invalid storage endpoint {}: {}", endpoint, e))
        .map(|endpoint| {
            endpoint
                .connect_timeout(channel.storage_grpc_connect_timeout)
                .timeout(channel.storage_grpc_request_timeout)
                // HTTP/2 keepalive: detect dead connections and force reconnect
                .http2_keep_alive_interval(channel.storage_grpc_http2_keepalive_interval)
                .keep_alive_timeout(channel.storage_grpc_keepalive_timeout)
                .keep_alive_while_idle(true)
                // TCP keepalive for OS-level dead connection detection
                .tcp_keepalive(Some(channel.storage_grpc_tcp_keepalive))
        })
}

/// Helper: Create a gRPC channel to the storage service with keepalive settings
///
/// Keepalive configuration ensures the client detects dead connections quickly
/// and reconnects when the storage service restarts (gets a new pod IP).
pub(super) async fn create_storage_channel() -> Result<tonic::transport::Channel, String> {
    if BYPASS_CHANNEL_CACHE.load(Ordering::SeqCst) {
        return build_storage_endpoint()?
            .connect()
            .await
            .map_err(|e| format!("Failed to connect to storage service: {}", e));
    }

    if let Some(channel) = STORAGE_CHANNEL.get() {
        // tonic channels are internally pooled/reconnecting; cloning a cached channel
        // keeps connection reuse while still allowing transport-level reconnect on drops.
        return Ok(channel.clone());
    }

    let channel = build_storage_endpoint()?.connect_lazy();
    let _ = STORAGE_CHANNEL.set(channel.clone());
    Ok(channel)
}

/// The same channel as [`create_storage_channel`], with client-side trace
/// propagation applied.
///
/// This builder is the one the association and metadata paths use, so without it
/// they would be the only storage calls invisible to the caller's trace — and
/// they are the ones that run in the middle of a request, where an unattributed
/// stall is hardest to explain. Auth is per request here, as everywhere in this
/// module, so there is no interceptor in the stack.
pub(super) async fn create_traced_storage_channel() -> Result<crate::outbound::TracedChannel, String>
{
    Ok(crate::outbound::trace_layer(
        create_storage_channel().await?,
    ))
}

/// Add the authorization and cache-control headers every storage call carries.
///
/// Public so callers outside this module — `task-worker`'s file transfers — use
/// this one rather than writing their own. The copy that used to live there
/// dropped the header-parse failure on the floor, which would have sent an
/// unauthenticated request instead of failing.
pub fn add_auth_headers<T>(
    request: &mut tonic::Request<T>,
    auth_token: &str,
) -> Result<(), String> {
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", auth_token)
            .parse()
            .map_err(|e| format!("Failed to create authorization header: {}", e))?,
    );
    request.metadata_mut().insert(
        "cache-control",
        "no-cache, no-store"
            .parse()
            .map_err(|e| format!("Failed to create cache-control header: {}", e))?,
    );
    Ok(())
}
