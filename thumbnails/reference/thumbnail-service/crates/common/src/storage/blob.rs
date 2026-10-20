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

//! Deleting generated thumbnail blobs directly against storage.
//!
//! This replaces asking `thumbnail-task-queue` to `DELETE /thumbnails`. Blob
//! deletion was the second dependency on that service, so removing only the
//! enqueue call would have kept the whole Python deployment alive to service
//! deletions.
//!
//! Deletion stays best-effort and always follows a completed metadata removal.
//! It cannot change the result the caller already received: by the time we get
//! here the association is gone, and reporting a late failure would turn a
//! successful disassociation into an error the caller cannot act on. What it
//! must not do is fail silently — a blob whose metadata is already removed is
//! not rediscoverable by any later enumeration, so every failure is counted and
//! logged with its address.

use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_client::FileObjectServiceClient;
use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::DeleteRequest;
use tonic::Code;
use tracing::{error, info, warn};

use crate::current_runtime_config;

/// What a best-effort blob deletion pass achieved.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BlobDeletionOutcome {
    /// Addresses storage confirmed gone, including those already absent.
    pub deleted: Vec<String>,
    /// Addresses still present, each with the reason. These are orphans.
    pub failed: Vec<BlobDeletionFailure>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BlobDeletionFailure {
    pub resource_address: String,
    pub reason: String,
}

/// Delete every listed thumbnail blob, reporting per-address outcomes.
///
/// `NOT_FOUND` counts as deleted: the blob is gone, which is the postcondition
/// the caller wanted. Every other failure leaves an orphan, so it is logged at
/// error level naming the address — that log line is the only remaining trace of
/// a blob nothing will enumerate again.
///
/// Takes `outbound` rather than a token because the token has to be the service
/// account's. Generated thumbnails are written by `task-worker` under the service
/// identity, and a caller disassociating one need not have delete rights on the
/// thumbnail path at all — passing the caller's bearer here would fail deletion
/// for exactly the callers most likely to use it. Fetching it inside means no
/// call site can pass the wrong one.
pub async fn delete_thumbnail_blobs(
    addresses: Vec<String>,
    outbound: &crate::outbound::OutboundClients,
) -> BlobDeletionOutcome {
    let mut outcome = BlobDeletionOutcome::default();
    if addresses.is_empty() {
        return outcome;
    }

    let mut auth_token = match outbound.get_auth_token() {
        Ok(token) => token,
        Err(e) => {
            // Nothing to retry against: the metadata is already gone, so these
            // blobs are orphaned whatever happens next. Say so per address.
            for resource_address in addresses {
                record_orphan(
                    &mut outcome,
                    resource_address,
                    format!("service credential unavailable: {e}"),
                );
            }
            return outcome;
        }
    };

    // Bounded per request, explicitly. The channel below already carries
    // `storage_grpc_request_timeout` as an endpoint-level deadline, so this is
    // belt-and-braces — but a deadline you have to read the channel builder to
    // discover is one a later edit can drop without noticing, and this loop runs
    // *after* the metadata is gone: a hang here holds the caller's response open
    // with the association already removed. Whichever bound is shorter wins, and
    // an expired one arrives as a `tonic::Status` that the error arm below
    // records as an orphan like any other failure.
    let request_timeout = current_runtime_config()
        .storage_channel
        .storage_grpc_request_timeout;

    // The same channel the metadata calls beside this one use, rather than the
    // `OutboundClients` channel `stat.rs` takes. Deletion runs inside the
    // association paths, immediately after those metadata removals, and reaching
    // for a second channel there would give one logical operation two different
    // connection lifetimes and two different endpoints in tests.
    let channel = match super::create_traced_storage_channel().await {
        Ok(channel) => channel,
        Err(e) => {
            for resource_address in addresses {
                record_orphan(&mut outcome, resource_address, format!("connect: {e}"));
            }
            return outcome;
        }
    };
    let mut client = FileObjectServiceClient::new(channel);

    for resource_address in addresses {
        let request = match delete_request(&resource_address, request_timeout, &auth_token) {
            Ok(request) => request,
            Err(e) => {
                record_orphan(&mut outcome, resource_address, e);
                continue;
            }
        };

        let response = match client.delete(request).await {
            Err(status) if status.code() == Code::Unauthenticated => {
                info!(
                    resource_address,
                    "Blob delete got Unauthenticated; refreshing and retrying once"
                );
                outbound.refresher.refresh().await;
                auth_token = match outbound.get_auth_token() {
                    Ok(token) => token,
                    Err(e) => {
                        record_orphan(
                            &mut outcome,
                            resource_address,
                            format!("service credential unavailable after refresh: {e}"),
                        );
                        continue;
                    }
                };
                let request = match delete_request(&resource_address, request_timeout, &auth_token)
                {
                    Ok(request) => request,
                    Err(e) => {
                        record_orphan(&mut outcome, resource_address, e);
                        continue;
                    }
                };
                client.delete(request).await
            }
            response => response,
        };

        match response {
            Ok(_) => {
                info!(resource_address, "Deleted generated thumbnail blob");
                outcome.deleted.push(resource_address);
            }
            Err(status) if status.code() == Code::NotFound => {
                // Already gone. The caller wanted the blob absent, and it is.
                info!(
                    resource_address,
                    "Generated thumbnail blob was already absent"
                );
                outcome.deleted.push(resource_address);
            }
            Err(status) => {
                record_orphan(
                    &mut outcome,
                    resource_address,
                    format!("{} ({})", status.message(), status.code()),
                );
            }
        }
    }

    if !outcome.failed.is_empty() {
        warn!(
            orphaned = outcome.failed.len(),
            deleted = outcome.deleted.len(),
            "Some generated thumbnail blobs could not be deleted and are now unreferenced"
        );
    }

    outcome
}

fn delete_request(
    resource_address: &str,
    request_timeout: std::time::Duration,
    auth_token: &str,
) -> Result<tonic::Request<DeleteRequest>, String> {
    let mut request = tonic::Request::new(DeleteRequest {
        resource_address: resource_address.to_string(),
        previous_version: None,
    });
    request.set_timeout(request_timeout);
    super::add_auth_headers(&mut request, auth_token).map_err(|e| format!("auth headers: {e}"))?;
    Ok(request)
}

fn record_orphan(outcome: &mut BlobDeletionOutcome, resource_address: String, reason: String) {
    error!(
        resource_address,
        reason,
        "Generated thumbnail blob deletion failed after its metadata was removed; \
         the blob is orphaned and no enumeration will find it again"
    );
    outcome.failed.push(BlobDeletionFailure {
        resource_address,
        reason,
    });
}
