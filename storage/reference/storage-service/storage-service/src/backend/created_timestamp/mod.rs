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

//! Records the `created_timestamp` extended-metadata entry when a resource is first created.
//!
//! The client-library reads a resource's creation time from the extended-metadata key
//! `created_timestamp` (seconds since the Unix epoch). Nothing set it before — folders in
//! particular were left without a creation time. This layer sets it, once, when a resource is
//! first created via `write` / `copy` / `move` / `create_folder`, and never overwrites it
//! afterwards so the original creation time survives later writes. Setting it is best-effort: a
//! metadata failure never fails the write the client just made.
//!
//! When a resource is deleted the layer symmetrically removes its `created_timestamp` entry, again
//! best-effort: the cleanup runs only after the underlying delete succeeds, and a metadata failure
//! never fails the delete the client just made.

use std::time::{SystemTime, UNIX_EPOCH};

use storage_core::layer::Layer;
use storage_core::traits::Request;
use storage_service_protos::google::protobuf::{value::Kind, Value};
use storage_service_protos::nvidia::omniverse::storage::filefolder::v1alpha::FolderAddress;
use tracing::warn;

use crate::backend::metadata::{
    any::AnyMetadata, batch_error_entry::Code, delete_metadata_batch_response_entry::Outcome as DeleteOutcome,
    get_metadata_batch_response_entry::Outcome, resource_location::Resource, DeleteMetadataBatchRequest,
    DeleteMetadataBatchRequestEntry, GetMetadataBatchRequest, GetMetadataBatchRequestEntry, Metadata, ResourceLocation,
    UpdateMetadataBatchRequest, UpdateMetadataBatchRequestEntry,
};

mod filefolder;
mod fileobject;

/// The extended-metadata key the client-library reads as an object's creation time (seconds).
const CREATED_TIMESTAMP_KEY: &str = "created_timestamp";

pub struct CreatedTimestampLayer {
    metadata: AnyMetadata,
}

impl CreatedTimestampLayer {
    pub fn new(metadata: AnyMetadata) -> Self {
        Self { metadata }
    }
}

impl<S> Layer<S> for CreatedTimestampLayer {
    type Service = CreatedTimestampService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CreatedTimestampService {
            inner,
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CreatedTimestampService<S> {
    inner: S,
    metadata: AnyMetadata,
}

/// The metadata resource location of a just-written file/object at `uri`.
fn resource_address_location(uri: String) -> ResourceLocation {
    ResourceLocation {
        resource: Some(Resource::ResourceAddress(uri)),
    }
}

/// The metadata resource location of a just-created folder at `uri`.
fn folder_address_location(uri: String) -> ResourceLocation {
    ResourceLocation {
        resource: Some(Resource::FolderAddress(FolderAddress { uri })),
    }
}

/// Record `created_timestamp = now` (seconds) for a just-created resource, unless it is already
/// set. Set-once and best-effort: it never overwrites an existing value — so an object's creation
/// time is fixed at its first write and preserved across later overwrites — and never fails the
/// caller.
///
/// The read-then-write is not atomic, but the only race is two concurrent first-creates of the
/// same resource both writing; their timestamps then differ by at most the race window, which is
/// immaterial for a display-only creation time.
async fn record_created_timestamp(metadata: &AnyMetadata, location: ResourceLocation) {
    // Set-once: only write when the key is definitively absent, so an overwrite never resets the
    // creation time. On any ambiguity (read failure, unexpected error) leave the value untouched.
    match metadata
        .get_metadata_batch(Request::new(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(location.clone()),
                user_metadata_key: CREATED_TIMESTAMP_KEY.to_owned(),
                namespace: None,
            }],
        }))
        .await
    {
        Ok(response) => match response.results.into_iter().next().and_then(|entry| entry.outcome) {
            Some(Outcome::Success(_)) => return,
            Some(Outcome::Error(error)) if error.code == Code::NotFound as i32 => {},
            other => {
                warn!(?other, "created_timestamp: could not confirm absence, leaving it unset");
                return;
            },
        },
        Err(error) => {
            warn!(%error, "created_timestamp: metadata read failed, leaving it unset");
            return;
        },
    }

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default();
    let request = Request::new(UpdateMetadataBatchRequest {
        requests: vec![UpdateMetadataBatchRequestEntry {
            resource_location: Some(location),
            user_metadata_key: CREATED_TIMESTAMP_KEY.to_owned(),
            user_metadata: Some(Value {
                kind: Some(Kind::NumberValue(seconds)),
            }),
            expected_etag: None,
            namespace: None,
        }],
    });
    if let Err(error) = metadata.update_metadata_batch(request).await {
        warn!(%error, "created_timestamp: metadata write failed");
    }
}

/// Best-effort removal of the `created_timestamp` entry for a just-deleted resource. This is the
/// symmetric counterpart of [`record_created_timestamp`] and must be called only after the
/// underlying delete has succeeded.
///
/// Best-effort: a metadata failure never fails the delete the client just made. On a transport
/// error, or a per-entry error other than `NotFound`, we log a warning (with the address and error)
/// and return. A `NotFound` outcome is expected and benign — the resource may predate this layer,
/// or have been created through a write path that does not record the timestamp — so it is treated
/// as success.
async fn delete_created_timestamp(metadata: &AnyMetadata, location: ResourceLocation) {
    let request = Request::new(DeleteMetadataBatchRequest {
        requests: vec![DeleteMetadataBatchRequestEntry {
            resource_location: Some(location.clone()),
            user_metadata_key: CREATED_TIMESTAMP_KEY.to_owned(),
            expected_etag: None,
            namespace: None,
        }],
    });
    match metadata.delete_metadata_batch(request).await {
        Ok(response) => match response.results.into_iter().next().and_then(|entry| entry.outcome) {
            Some(DeleteOutcome::Error(error)) if error.code != Code::NotFound as i32 => {
                warn!(?location, ?error, "created_timestamp: metadata delete failed");
            },
            _ => {},
        },
        Err(error) => {
            warn!(?location, %error, "created_timestamp: metadata delete call failed");
        },
    }
}
