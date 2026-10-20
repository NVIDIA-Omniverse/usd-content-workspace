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

//! `delete_thumbnail_blobs` against the storage double, directly.
//!
//! It was reached only transitively before, through the disassociation sweep, so
//! its own contract went unasserted: which addresses count as deleted, which count
//! as orphans, and that one failure does not stop the rest. That contract matters
//! because a failed deletion produces a blob no later enumeration can find — the
//! sweep walks association metadata on the source, and by the time deletion runs
//! that metadata is already gone.
//!
//! Its own integration-test binary so it exclusively controls the per-binary
//! storage double and the faults armed on it.

use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_storage, unique_test_address, FileOp,
};
use thumbnail_common::OutboundClients;

fn delete_attempts(address: &str) -> usize {
    mock_storage()
        .file_calls_for(address)
        .into_iter()
        .filter(|call| call.op == FileOp::Delete)
        .count()
}

#[tokio::test]
async fn every_address_is_attempted_and_reported() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-common-blob-deletion");

    let first = unique_test_address("blob-batch-one.png");
    let second = unique_test_address("blob-batch-two.png");
    mock_storage().seed_blob(&first, b"one".to_vec());
    mock_storage().seed_blob(&second, b"two".to_vec());

    let outcome = thumbnail_common::storage::delete_thumbnail_blobs(
        vec![first.clone(), second.clone()],
        &OutboundClients::for_tests(),
    )
    .await;

    let mut deleted = outcome.deleted.clone();
    deleted.sort();
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(deleted, expected);
    assert!(outcome.failed.is_empty(), "got {:?}", outcome.failed);

    assert!(!mock_storage().has_blob(&first));
    assert!(!mock_storage().has_blob(&second));
}

/// An address that is already gone counts as deleted.
///
/// Cleanup is idempotent by necessity: a redelivered storage-deleted event runs
/// the sweep again, and treating an absent blob as a failure would report an
/// orphan population that does not exist.
#[tokio::test]
async fn an_already_absent_blob_is_a_success() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-common-blob-deletion");

    let missing = unique_test_address("blob-never-existed.png");
    // Deliberately not seeded.

    let outcome = thumbnail_common::storage::delete_thumbnail_blobs(
        vec![missing.clone()],
        &OutboundClients::for_tests(),
    )
    .await;

    assert_eq!(outcome.deleted, vec![missing]);
    assert!(outcome.failed.is_empty(), "got {:?}", outcome.failed);
}

/// One failure does not stop the others, and the failure is named.
///
/// Stopping at the first failure would silently skip every address behind it, and
/// those are exactly the orphans nothing can enumerate afterwards.
#[tokio::test]
async fn a_failure_is_reported_without_abandoning_the_rest() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-common-blob-deletion");

    let doomed = unique_test_address("blob-delete-refused.png");
    let fine = unique_test_address("blob-delete-ok.png");
    mock_storage().seed_blob(&doomed, b"refused".to_vec());
    mock_storage().seed_blob(&fine, b"ok".to_vec());
    mock_storage().fail_file(FileOp::Delete, &doomed, tonic::Code::Internal);

    let outcome = thumbnail_common::storage::delete_thumbnail_blobs(
        vec![doomed.clone(), fine.clone()],
        &OutboundClients::for_tests(),
    )
    .await;

    assert_eq!(
        outcome.deleted,
        vec![fine.clone()],
        "the deletable address must still be deleted"
    );
    assert_eq!(outcome.failed.len(), 1, "got {:?}", outcome.failed);
    assert_eq!(outcome.failed[0].resource_address, doomed);
    assert!(
        !outcome.failed[0].reason.is_empty(),
        "an orphan with no stated reason cannot be triaged"
    );

    assert!(
        mock_storage().has_blob(&doomed),
        "the refused blob is the orphan"
    );
    assert!(!mock_storage().has_blob(&fine));
}

/// An empty list is a no-op rather than a round trip.
#[tokio::test]
async fn an_empty_list_touches_storage_not_at_all() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-common-blob-deletion");

    let untouched = unique_test_address("blob-empty-list.png");
    mock_storage().seed_blob(&untouched, b"untouched".to_vec());

    let outcome =
        thumbnail_common::storage::delete_thumbnail_blobs(vec![], &OutboundClients::for_tests())
            .await;

    assert!(outcome.deleted.is_empty());
    assert!(outcome.failed.is_empty());
    assert_eq!(delete_attempts(&untouched), 0);
    assert!(mock_storage().has_blob(&untouched));
}
