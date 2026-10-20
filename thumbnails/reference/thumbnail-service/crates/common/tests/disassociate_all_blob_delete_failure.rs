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

//! A failed blob deletion must not change the disassociation the caller already got.
//!
//! Blob deletion happens after both metadata removals have succeeded, so by the
//! time it runs the association is gone and the caller's outcome is decided.
//! Reporting a late storage failure would turn a completed disassociation into an
//! error nobody can act on. What the sweep must still do is attempt the delete and
//! leave a record when it fails, because a blob whose metadata is already removed
//! is not rediscoverable by any later enumeration.
//!
//! This test lives in its own integration-test binary so it exclusively controls
//! the per-binary storage double and the faults armed on it.

use std::sync::Arc;

use thumbnail_common::storage::association::{disassociate_all_thumbnails, THUMBNAIL_METADATA_KEY};
use thumbnail_common::storage::AssociationTargetKind;
use thumbnail_common::storage::AssociationTargetKind as _TargetKind;
use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_storage, unique_test_address, FileOp,
};

#[tokio::test]
async fn blob_delete_failure_is_tolerated_but_recorded() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-blob-delete-failure-tolerance-test",
    );

    let source_address = unique_test_address("blobfail-source.jpg");
    let thumbnail_address = unique_test_address("blobfail-thumb.png");
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
    mock_storage().seed_blob(&thumbnail_address, b"thumbnail bytes".to_vec());

    // Storage refuses to delete the blob, permanently.
    mock_storage().fail_file(FileOp::Delete, &thumbnail_address, tonic::Code::Internal);

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert!(
        outcome.aborted.is_none(),
        "a blob deletion failure must not abort the sweep"
    );
    assert!(
        outcome.failed.is_empty(),
        "the disassociation itself succeeded; got failures: {:?}",
        outcome.failed
    );
    assert_eq!(
        outcome.succeeded.len(),
        1,
        "the seeded association should be reported as removed despite the blob failure"
    );

    // Metadata cleanup completed regardless of the storage delete error.
    assert!(
        !mock_storage().has_key(&source_address, source_key),
        "source association metadata key must be removed"
    );
    assert!(
        !mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "thumbnail metadata key must be removed"
    );

    // The deletion was attempted exactly once and the blob is now an orphan.
    let deletes: Vec<_> = mock_storage()
        .file_calls_for(&thumbnail_address)
        .into_iter()
        .filter(|call| call.op == FileOp::Delete)
        .collect();
    assert_eq!(
        deletes.len(),
        1,
        "exactly one blob delete should have been attempted"
    );
    assert!(
        mock_storage().has_blob(&thumbnail_address),
        "the blob is still present, which is what makes it an orphan worth logging"
    );
}

#[tokio::test]
async fn successful_sweep_deletes_the_blob() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-blob-delete-failure-tolerance-test",
    );

    let source_address = unique_test_address("blobok-source.jpg");
    let thumbnail_address = unique_test_address("blobok-thumb.png");
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
    mock_storage().seed_blob(&thumbnail_address, b"thumbnail bytes".to_vec());

    let outcome = disassociate_all_thumbnails(
        &source_address,
        _TargetKind::File,
        "test-token",
        Arc::new(thumbnail_common::OutboundClients::for_tests()),
    )
    .await;

    assert_eq!(outcome.succeeded.len(), 1);
    assert!(
        !mock_storage().has_blob(&thumbnail_address),
        "the generated blob must be deleted against storage once its metadata is gone"
    );
}

#[tokio::test]
async fn unauthenticated_blob_delete_refreshes_and_retries_once() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-blob-delete-failure-tolerance-test",
    );

    let thumbnail_address = unique_test_address("blob-auth-refresh-thumb.png");
    mock_storage().seed_blob(&thumbnail_address, b"thumbnail bytes".to_vec());
    mock_storage().fail_file_times(
        FileOp::Delete,
        &thumbnail_address,
        tonic::Code::Unauthenticated,
        1,
    );

    let outcome = thumbnail_common::storage::delete_thumbnail_blobs(
        vec![thumbnail_address.clone()],
        &thumbnail_common::OutboundClients::for_tests(),
    )
    .await;

    assert_eq!(outcome.deleted, vec![thumbnail_address.clone()]);
    assert!(outcome.failed.is_empty());
    assert_eq!(
        mock_storage()
            .file_calls_for(&thumbnail_address)
            .into_iter()
            .filter(|call| call.op == FileOp::Delete)
            .count(),
        2,
        "Unauthenticated gets one refreshed-token reissue"
    );
    assert!(!mock_storage().has_blob(&thumbnail_address));
}
