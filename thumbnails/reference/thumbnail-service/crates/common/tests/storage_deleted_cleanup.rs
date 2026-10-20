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

//! Storage-deleted cleanup asks storage to delete each thumbnail blob it removed.
//!
//! Previously this asserted a `DELETE /thumbnails` call to the Python task queue.
//! After the cutover the deletion goes straight to `FileObjectService.Delete`,
//! which is what removes the last runtime dependency on that service — leaving the
//! enqueue call behind would have kept the whole Python deployment alive just to
//! service deletions.

use std::sync::Arc;

use thumbnail_common::storage::association::{disassociate_all_thumbnails, THUMBNAIL_METADATA_KEY};
use thumbnail_common::storage::AssociationTargetKind;
use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_storage, unique_test_address, FileOp,
};

#[tokio::test]
async fn disassociate_all_deletes_thumbnail_blobs_against_storage() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-storage-deleted-cleanup-test",
    );

    let source_address = unique_test_address("deleted-source.jpg");
    let thumbnail_address = unique_test_address("deleted-source-thumbnail.png");
    let source_key = "thumbnail_deleted_event";
    mock_storage().seed(
        &source_address,
        source_key,
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumbnail_address,
                "resource_identity": format!("{thumbnail_address}?version-id=v1")
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1")
            },
            "specification": {
                "type": "IMAGE_TO_IMAGE",
                "width": 256,
                "height": 256,
                "encoding_format": "PNG",
                "transform_type": "CROP",
                "crop_strategy": "SMART"
            },
            "target_type": "FILE"
        }),
    );
    mock_storage().seed(
        &thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );
    mock_storage().seed_blob(&thumbnail_address, b"generated thumbnail".to_vec());

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert!(outcome.failed.is_empty(), "failures: {:?}", outcome.failed);
    assert_eq!(outcome.succeeded.len(), 1);
    assert!(!mock_storage().has_key(&source_address, source_key));
    assert!(!mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY));

    let deletes: Vec<_> = mock_storage()
        .file_calls_for(&thumbnail_address)
        .into_iter()
        .filter(|call| call.op == FileOp::Delete)
        .collect();
    assert_eq!(
        deletes.len(),
        1,
        "the sweep should request exactly one blob deletion, against storage"
    );
    assert!(
        !mock_storage().has_blob(&thumbnail_address),
        "the generated blob should be gone once its metadata is removed"
    );
}

/// The deletion follows the metadata removal, so a blob is never deleted while
/// the association that points at it still exists.
#[tokio::test]
async fn no_blob_is_deleted_when_metadata_removal_fails() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-storage-deleted-cleanup-test",
    );

    let source_address = unique_test_address("retained-source.jpg");
    let thumbnail_address = unique_test_address("retained-thumbnail.png");
    let source_key = "thumbnail_deleted_event";
    mock_storage().seed(
        &source_address,
        source_key,
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumbnail_address,
                "resource_identity": format!("{thumbnail_address}?version-id=v1")
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1")
            },
            "specification": {
                "type": "IMAGE_TO_IMAGE",
                "width": 256,
                "height": 256,
                "encoding_format": "PNG",
                "transform_type": "CROP",
                "crop_strategy": "SMART"
            },
            "target_type": "FILE"
        }),
    );
    mock_storage().seed(
        &thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );
    mock_storage().seed_blob(&thumbnail_address, b"generated thumbnail".to_vec());

    // The thumbnail-side metadata removal fails, so the association survives.
    mock_storage().fail(
        thumbnail_common::test_fixtures::mock_storage_server::MetadataOp::Delete,
        &thumbnail_address,
        Some(THUMBNAIL_METADATA_KEY),
        tonic::Code::Internal,
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert!(
        outcome.succeeded.is_empty(),
        "nothing was disassociated, so nothing should be reported as removed"
    );
    assert_eq!(
        outcome.failed.len(),
        1,
        "the entry must be reported as failed, not quietly dropped: {outcome:?}"
    );

    // The blob surviving is only half of it. If the association metadata were
    // removed while the thumbnail-side removal failed, the blob would still be
    // there but nothing would reference it — an orphan no later sweep can find,
    // which is the outcome the two-phase ordering exists to prevent.
    assert!(
        mock_storage().has_key(&source_address, source_key),
        "the source-side association entry must survive a failed thumbnail-side removal"
    );
    assert!(
        mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "the thumbnail-side metadata must survive the removal that failed"
    );
    assert!(
        mock_storage().has_blob(&thumbnail_address),
        "the blob must survive: its association still exists"
    );
    assert!(
        mock_storage()
            .file_calls_for(&thumbnail_address)
            .iter()
            .all(|call| call.op != FileOp::Delete),
        "no deletion should be attempted while the metadata removal is unresolved"
    );
}
