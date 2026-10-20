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

//! A blob this service did not generate survives the sweep.
//!
//! Sweep blob deletion runs under the service identity, which holds delete rights
//! the caller may not. So the address it deletes cannot simply be whatever an
//! association record points at: a caller who can write association metadata
//! naming an address they do not own would otherwise get the service to delete it
//! for them, by registering it as the "thumbnail" of a resource they do own and
//! then deleting that resource. Metadata-write becomes delete.
//!
//! What separates the two cases is recorded at association time. A `MANUAL`
//! association means the caller supplied pre-existing bytes; `IMAGE_TO_IMAGE`
//! means this service converted them. The path template is the second signal, for
//! records written before the specification was stored.
//!
//! Its own integration-test binary so it exclusively controls the per-binary
//! storage double.

use std::sync::Arc;

use thumbnail_common::storage::association::{disassociate_all_thumbnails, THUMBNAIL_METADATA_KEY};
use thumbnail_common::storage::AssociationTargetKind;
use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_storage, unique_test_address, FileOp,
};

/// Seed one association on `source_address` whose stored specification is
/// `specification_type`, plus the thumbnail-side metadata and the blob itself.
fn seed_association(source_address: &str, thumbnail_address: &str, specification_type: &str) {
    let specification = if specification_type == "MANUAL" {
        serde_json::json!({
            "type": "MANUAL",
            "width": 256,
            "height": 256,
            "encoding_format": "PNG"
        })
    } else {
        serde_json::json!({
            "type": "IMAGE_TO_IMAGE",
            "width": 256,
            "height": 256,
            "encoding_format": "PNG",
            "transform_type": "CROP",
            "crop_strategy": "SMART"
        })
    };

    mock_storage().seed(
        source_address,
        "thumbnail_deleted_event",
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumbnail_address,
                "resource_identity": format!("{thumbnail_address}?version-id=v1")
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1")
            },
            "specification": specification,
            "target_type": "FILE"
        }),
    );
    mock_storage().seed(
        thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );
    mock_storage().seed_blob(thumbnail_address, b"thumbnail bytes".to_vec());
}

/// How many `Delete` calls the double saw for `address`.
fn delete_attempts(address: &str) -> usize {
    mock_storage()
        .file_calls_for(address)
        .into_iter()
        .filter(|call| call.op == FileOp::Delete)
        .count()
}

#[tokio::test]
async fn a_manually_associated_blob_survives_the_sweep() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-caller-owned-blob-test",
    );

    let source_address = unique_test_address("callerowned-source.jpg");
    // Deliberately not an address the configured template renders: this stands in
    // for a resource the caller already owned and registered as a thumbnail.
    let thumbnail_address = unique_test_address("victim-container/master.usd");

    seed_association(&source_address, &thumbnail_address, "MANUAL");

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert!(outcome.aborted.is_none(), "the sweep must not abort");
    assert!(
        outcome.failed.is_empty(),
        "leaving a caller-owned blob alone is not a failure; got {:?}",
        outcome.failed
    );
    assert_eq!(
        outcome.succeeded.len(),
        1,
        "the association metadata is still removed"
    );

    assert_eq!(
        delete_attempts(&thumbnail_address),
        0,
        "a MANUAL association's blob must never be deleted under the service identity — \
         that is the privilege escalation this check exists to stop"
    );
    assert!(
        mock_storage().has_blob(&thumbnail_address),
        "the caller's object must still be in storage"
    );
    assert!(
        !mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "the thumbnail-side association metadata is still removed"
    );
}

#[tokio::test]
async fn a_generated_blob_is_still_deleted() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-caller-owned-blob-test",
    );

    let source_address = unique_test_address("generated-source.jpg");
    let thumbnail_address = unique_test_address("generated-thumb.png");

    seed_association(&source_address, &thumbnail_address, "IMAGE_TO_IMAGE");

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert!(outcome.failed.is_empty(), "got {:?}", outcome.failed);
    assert_eq!(outcome.succeeded.len(), 1);

    assert_eq!(
        delete_attempts(&thumbnail_address),
        1,
        "an association recording a conversion by this service is still cleaned up"
    );
    assert!(
        !mock_storage().has_blob(&thumbnail_address),
        "the generated blob is gone"
    );
}
