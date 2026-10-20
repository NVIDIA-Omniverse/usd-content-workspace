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

/// Integration tests for `disassociate_all_thumbnails` covering the coverage gaps
/// identified in the sweep function's error and edge-case branches.
///
/// All tests in this binary set the same unreachable dummy task-queue endpoint
/// before calling `init_test_runtime_config`. The `Once` guard means only the
/// first call installs the config; concurrent threads writing the same env-var
/// value are safe. Task-queue calls that happen to reach the dummy endpoint for
/// tests with successful outcomes fail silently — that best-effort tolerance is
/// covered explicitly in the separate `disassociate_all_task_queue_failure` binary.
use std::sync::Arc;

use thumbnail_common::storage::association::{
    disassociate_all_thumbnails, DisassociateAbort, DisassociateEntryError, THUMBNAIL_METADATA_KEY,
};
use thumbnail_common::storage::metadata::StorageMetadataError;
use thumbnail_common::storage::AssociationTargetKind;
use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_storage, unique_test_address, MetadataOp,
};

fn init() {
    thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-common-disassociate-all-sweep-coverage",
    );
}

fn outbound() -> Arc<thumbnail_common::OutboundClients> {
    Arc::new(thumbnail_common::OutboundClients::for_tests())
}

fn seed_association(source_address: &str, source_key: &str, thumbnail_address: &str) {
    mock_storage().seed(
        source_address,
        source_key,
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumbnail_address,
                "resource_identity": format!("{thumbnail_address}?version-id=v1"),
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1"),
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
        thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );
}

// ── Gap 3: empty source ──────────────────────────────────────────────────────

#[tokio::test]
async fn disassociate_all_empty_source_returns_empty_outcome() {
    init();

    let source_address = unique_test_address("empty-source.jpg");

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(
        outcome.aborted.is_none(),
        "should not abort: {:?}",
        outcome.aborted
    );
    assert!(
        outcome.succeeded.is_empty(),
        "unexpected successes: {:?}",
        outcome.succeeded
    );
    assert!(
        outcome.failed.is_empty(),
        "unexpected failures: {:?}",
        outcome.failed
    );
}

// ── Gap 10: non-thumbnail_* keys on the source are ignored ───────────────────

#[tokio::test]
async fn disassociate_all_non_thumbnail_keys_are_ignored() {
    init();

    let source_address = unique_test_address("config-only-source.jpg");

    mock_storage().seed(
        &source_address,
        "other_metadata",
        serde_json::json!({ "some": "value" }),
    );
    mock_storage().seed(
        &source_address,
        "config",
        serde_json::json!({ "setting": true }),
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(
        outcome.aborted.is_none(),
        "should not abort: {:?}",
        outcome.aborted
    );
    assert!(
        outcome.succeeded.is_empty(),
        "unexpected successes: {:?}",
        outcome.succeeded
    );
    assert!(
        outcome.failed.is_empty(),
        "unexpected failures: {:?}",
        outcome.failed
    );

    assert!(
        mock_storage().has_key(&source_address, "other_metadata"),
        "other_metadata key should not have been removed"
    );
    assert!(
        mock_storage().has_key(&source_address, "config"),
        "config key should not have been removed"
    );
}

// ── Gap 2: malformed entry produces a failed outcome, no storage changes ──────

#[tokio::test]
async fn disassociate_all_malformed_entry_produces_failed_outcome_no_storage_changes() {
    init();

    let source_address = unique_test_address("malformed-source.jpg");
    let source_key = "thumbnail_malformed_entry";

    // A thumbnail_* key whose value has no thumbnail.resource_address field —
    // StoredAssociationEntry::read produces thumbnail_address: None, which the
    // sweep classifies as MalformedEntry without issuing any delete RPCs.
    mock_storage().seed(
        &source_address,
        source_key,
        serde_json::json!({ "target_type": "FILE" }),
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert!(
        outcome.succeeded.is_empty(),
        "expected no successes, got: {:?}",
        outcome.succeeded
    );
    assert_eq!(
        outcome.failed.len(),
        1,
        "expected exactly one failure, got: {:?}",
        outcome.failed
    );
    assert!(
        matches!(
            &outcome.failed[0].error,
            DisassociateEntryError::MalformedEntry { key } if key == source_key
        ),
        "expected MalformedEntry with key '{}', got: {:?}",
        source_key,
        outcome.failed[0].error
    );
    assert!(
        mock_storage().has_key(&source_address, source_key),
        "source key should still be present after a malformed-entry failure"
    );
}

// ── Gap 1: multiple thumbnails with mixed outcomes ────────────────────────────

#[tokio::test]
async fn disassociate_all_mixed_outcomes_one_succeeds_one_fails_thumbnail_side() {
    init();

    let source_address = unique_test_address("mixed-source.usd");
    let thumb_a_address = unique_test_address("mixed-thumb-a.png");
    let thumb_b_address = unique_test_address("mixed-thumb-b.png");

    mock_storage().seed(
        &source_address,
        "thumbnail_aaa",
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumb_a_address,
                "resource_identity": format!("{thumb_a_address}?version-id=v1"),
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1"),
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
        &source_address,
        "thumbnail_bbb",
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumb_b_address,
                "resource_identity": format!("{thumb_b_address}?version-id=v1"),
            },
            "association_target": {
                "resource_address": source_address,
                "resource_identity": format!("{source_address}?version-id=v1"),
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
        &thumb_a_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );
    mock_storage().seed(
        &thumb_b_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );

    // Phase-1 deletion of thumb B fails permanently; thumb A goes through both phases.
    mock_storage().fail(
        MetadataOp::Delete,
        &thumb_b_address,
        Some(THUMBNAIL_METADATA_KEY),
        tonic::Code::Internal,
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert_eq!(
        outcome.succeeded.len(),
        1,
        "expected exactly one success, got: {:?}",
        outcome.succeeded
    );
    assert_eq!(
        outcome.failed.len(),
        1,
        "expected exactly one failure, got: {:?}",
        outcome.failed
    );
    assert_eq!(
        outcome.succeeded[0].resource_address, thumb_a_address,
        "succeeded entry should be thumb A"
    );
    assert!(
        matches!(
            outcome.failed[0].error,
            DisassociateEntryError::ThumbnailSideRemoval { .. }
        ),
        "expected ThumbnailSideRemoval, got: {:?}",
        outcome.failed[0].error
    );

    // Thumb A: both keys removed.
    assert!(!mock_storage().has_key(&thumb_a_address, THUMBNAIL_METADATA_KEY));
    assert!(!mock_storage().has_key(&source_address, "thumbnail_aaa"));

    // Thumb B: both keys intact for a retry.
    assert!(
        mock_storage().has_key(&thumb_b_address, THUMBNAIL_METADATA_KEY),
        "thumb_b thumbnail_metadata should still be present after phase-1 failure"
    );
    assert!(
        mock_storage().has_key(&source_address, "thumbnail_bbb"),
        "source key thumbnail_bbb should still be present after phase-1 failure"
    );
}

// ── Gap 7: token rejection aborts sweep during enumeration ───────────────────

#[tokio::test]
async fn disassociate_all_thumbnails_aborts_on_rejected_token_during_enumeration() {
    init();

    let source_address = unique_test_address("sweep-rejected-source.jpg");
    let thumbnail_address = unique_test_address("sweep-rejected-thumbnail.png");
    let source_key = "thumbnail_sweep_rejected";

    seed_association(&source_address, source_key, &thumbnail_address);

    // The rejection fires on the very first RPC (GetMetadata / enumeration),
    // aborting the sweep before any delete is issued.
    mock_storage().reject_token("sweep-rejected-token");

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "sweep-rejected-token",
        outbound(),
    )
    .await;

    assert!(
        matches!(
            &outcome.aborted,
            Some(DisassociateAbort::Enumeration(
                StorageMetadataError::Unauthenticated(_)
            ))
        ),
        "expected an enumeration abort carrying Unauthenticated, got {:?}",
        outcome.aborted
    );
    assert!(
        mock_storage().has_key(&source_address, source_key),
        "a token rejection during enumeration must not have removed anything"
    );
    assert!(
        outcome.succeeded.is_empty(),
        "no entries should have succeeded when enumeration is aborted"
    );
    assert!(
        outcome.failed.is_empty(),
        "no entries should have failed individually when enumeration is aborted"
    );
}

// ── Gap 4: all phase-1 per-entry failures leave source keys intact ────────────
//
// Note: the code path where `remove_storage_metadata_batch` itself returns `Err`
// (lines 1231–1249 in association.rs) requires a broken transport connection and
// cannot be triggered via the mock storage server. The closest achievable coverage
// is an address-wide per-entry Delete fault (`key=None`), which makes every entry
// in the phase-1 batch return `StorageMetadataBatchEntryOutcome::Failed`. All
// entries land in `DisassociateEntryError::ThumbnailSideRemoval`, and no source
// keys are modified — functionally equivalent from the caller's perspective.

#[tokio::test]
async fn disassociate_all_phase1_per_entry_failures_leave_source_keys_intact() {
    init();

    let source_address = unique_test_address("phase1-fail-source.jpg");
    let thumbnail_address = unique_test_address("phase1-fail-thumbnail.png");
    let source_key = "thumbnail_phase1_fail_event";

    seed_association(&source_address, source_key, &thumbnail_address);

    // key=None matches any Delete on this address, so every phase-1 batch entry
    // comes back as StorageMetadataBatchEntryOutcome::Failed(ThumbnailSideRemoval).
    mock_storage().fail(
        MetadataOp::Delete,
        &thumbnail_address,
        None,
        tonic::Code::Internal,
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    // The batch RPC itself returned Ok (per-entry failures, not a transport error).
    assert!(outcome.aborted.is_none());
    assert!(
        outcome.succeeded.is_empty(),
        "expected no successes, got: {:?}",
        outcome.succeeded
    );
    assert_eq!(
        outcome.failed.len(),
        1,
        "expected exactly one failure, got: {:?}",
        outcome.failed
    );
    assert!(
        matches!(
            &outcome.failed[0].error,
            DisassociateEntryError::ThumbnailSideRemoval { .. }
        ),
        "expected ThumbnailSideRemoval, got: {:?}",
        outcome.failed[0].error
    );
    // Phase 1 failed → source keys intact for retry.
    assert!(
        mock_storage().has_key(&source_address, source_key),
        "source key must still be present after a phase-1 failure"
    );
    assert!(
        mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "thumbnail_metadata key must still be present after a phase-1 per-entry failure"
    );
}

// ── Gap 5: phase-2 source-side per-entry failure ─────────────────────────────
//
// Phase 1 (thumbnail-side batch delete) succeeds, removing `thumbnail_metadata`.
// Phase 2 (source-side batch delete) encounters a per-entry Internal error on
// the source key, landing in the `AssociationTargetSideRemoval` path. The source
// key survives so a retry can re-find the entry (phase 1 is idempotent on
// NOT_FOUND, so re-running the sweep completes the removal).

#[tokio::test]
async fn disassociate_all_source_side_per_entry_failure_lands_in_association_target_side_removal() {
    init();

    let source_address = unique_test_address("gap5-source.jpg");
    let thumbnail_address = unique_test_address("gap5-thumbnail.png");
    let source_key = "thumbnail_gap5_event";

    seed_association(&source_address, source_key, &thumbnail_address);

    // Fault only on the source-side key: phase 1 succeeds (thumbnail_metadata
    // removed), phase 2 fails for this entry.
    mock_storage().fail(
        MetadataOp::Delete,
        &source_address,
        Some(source_key),
        tonic::Code::Internal,
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert!(
        outcome.succeeded.is_empty(),
        "expected no successes, got: {:?}",
        outcome.succeeded
    );
    assert_eq!(
        outcome.failed.len(),
        1,
        "expected one failure, got: {:?}",
        outcome.failed
    );
    assert!(
        matches!(
            &outcome.failed[0].error,
            DisassociateEntryError::AssociationTargetSideRemoval { .. }
        ),
        "expected AssociationTargetSideRemoval, got: {:?}",
        outcome.failed[0].error
    );
    // Source key survives phase-2 failure — retry can re-find this entry.
    assert!(
        mock_storage().has_key(&source_address, source_key),
        "source key must survive a phase-2 failure so the entry is retryable"
    );
    // Phase 1 already succeeded.
    assert!(
        !mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "thumbnail_metadata must be removed by the successful phase 1"
    );
}

// ── Gap 6: delete-batch stream error after all entries resolved ───────────────
//
// `fail_delete_batch_stream` sends a stream-level error AFTER the single batch
// response message, so all entry results arrive as Succeeded before
// `terminal_error` is set. The function logs the warning (lines 1252–1257) but
// does not abort — successful entries count as succeeded.
//
// True `Unresolved` entries (from a mid-stream failure before all results are
// delivered) cannot be triggered via the current mock server, which always sends
// all results in one message before appending the error frame.

#[tokio::test]
async fn disassociate_all_stream_error_after_entries_does_not_abort_successful_entries() {
    init();

    let source_address = unique_test_address("gap6-source.jpg");
    let thumbnail_address = unique_test_address("gap6-thumbnail.png");
    let source_key = "thumbnail_gap6_event";

    seed_association(&source_address, source_key, &thumbnail_address);

    // Arms a stream fault that fires AFTER the batch results are sent, so all
    // entries already have Succeeded outcomes when the terminal_error is observed.
    mock_storage().fail_delete_batch_stream(&thumbnail_address, tonic::Code::Internal);

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert_eq!(
        outcome.succeeded.len(),
        1,
        "entries resolved before the stream error must be counted as succeeded; failures: {:?}",
        outcome.failed
    );
    assert!(
        outcome.failed.is_empty(),
        "unexpected failures: {:?}",
        outcome.failed
    );
    assert!(
        !mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
        "thumbnail_metadata must be removed despite the subsequent stream error"
    );
    assert!(
        !mock_storage().has_key(&source_address, source_key),
        "source key must be removed by the successful phase 2"
    );
}

// ── Gap 9: AssociationTargetKind::Folder sweep ───────────────────────────────

#[tokio::test]
async fn disassociate_all_folder_target_sweep_succeeds() {
    init();

    let source_address = unique_test_address("deleted-folder");
    let thumbnail_address = unique_test_address("deleted-folder-thumbnail.png");
    let source_key = "thumbnail_256x256_PNG_abcdef012345";

    // Folder entries omit association_target.resource_identity —
    // build_association_message skips that field when target_kind is Folder;
    // StoredAssociationEntry::read handles the absent field via unwrap_or_default().
    mock_storage().seed(
        &source_address,
        source_key,
        serde_json::json!({
            "thumbnail": {
                "resource_address": thumbnail_address,
                "resource_identity": format!("{thumbnail_address}?version-id=v1"),
            },
            "association_target": {
                "resource_address": source_address
                // resource_identity intentionally absent for FOLDER targets
            },
            "specification": {
                "type": "IMAGE_TO_IMAGE",
                "width": 256,
                "height": 256,
                "encoding_format": "PNG",
                "transform_type": "CROP",
                "crop_strategy": "SMART"
            },
            "target_type": "FOLDER"
        }),
    );
    mock_storage().seed(
        &thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::Folder,
        "test-token",
        outbound(),
    )
    .await;

    assert!(outcome.aborted.is_none());
    assert!(outcome.failed.is_empty(), "failures: {:?}", outcome.failed);
    assert_eq!(outcome.succeeded.len(), 1);
    assert!(!mock_storage().has_key(&source_address, source_key));
    assert!(!mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY));
}

// ── Regression: two source-side entries sharing one thumbnail address must not
// ── send that address twice in the same phase-1 DeleteMetadataBatch request ───
//
// Storage rejects a batch request that names the same (address, key) more than
// once with a "duplicate batch entry" error, which `remove_storage_metadata_batch`
// surfaces as a per-entry `Failed`, not `NotFound` — so it is not idempotent-safe
// like an already-removed key. `DisassociateEntryError::is_retryable()` marks that
// failure retryable, so a real deployment that let this dedup regress would NACK
// the same `omni.storage.deleted` event forever: the failure is permanent (the
// duplicate is a structural property of the stored data) but classified as
// transient, so it can never clear on redelivery. The double's `DeleteMetadataBatch`
// does not itself reject duplicate entries (it resolves each independently against
// a plain key-value map, so a duplicate address would round-trip through it as a
// harmless second `NotFound`/success) — so this test does not assert against a
// simulated storage rejection. It instead pins the actual contract: the sweep must
// never place the same thumbnail address in the batch twice in the first place.
// Asserting on the recorded call count is what catches a regression here even
// though the double is lenient about the very condition production storage rejects.
#[tokio::test]
async fn disassociate_all_dedupes_repeated_thumbnail_address_in_phase_one_batch() {
    init();

    let source_address = unique_test_address("dup-thumb-source.jpg");
    let thumbnail_address = unique_test_address("dup-thumb-shared.png");

    // Two distinct source-side thumbnail_* keys (as if the target had been
    // associated twice, e.g. under two deterministic-key variants) both point at
    // the same underlying thumbnail resource address.
    for source_key in ["thumbnail_variant_one", "thumbnail_variant_two"] {
        mock_storage().seed(
            &source_address,
            source_key,
            serde_json::json!({
                "thumbnail": {
                    "resource_address": thumbnail_address,
                    "resource_identity": format!("{thumbnail_address}?version-id=v1"),
                },
                "association_target": {
                    "resource_address": source_address,
                    "resource_identity": format!("{source_address}?version-id=v1"),
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
    }
    mock_storage().seed(
        &thumbnail_address,
        THUMBNAIL_METADATA_KEY,
        serde_json::json!({ "status": "SUCCESS" }),
    );

    let outcome = disassociate_all_thumbnails(
        &source_address,
        AssociationTargetKind::File,
        "test-token",
        outbound(),
    )
    .await;

    assert!(
        outcome.aborted.is_none(),
        "unexpected abort: {:?}",
        outcome.aborted
    );
    assert!(
        outcome.failed.is_empty(),
        "expected no failures, got: {:?}",
        outcome.failed
    );
    assert_eq!(
        outcome.succeeded.len(),
        2,
        "both source-side entries should succeed, got: {:?}",
        outcome.succeeded
    );

    // The real regression guard: exactly one Delete call for the shared thumbnail
    // address's THUMBNAIL_METADATA_KEY, not two. Two would mean the phase-1 batch
    // sent the duplicate that production storage rejects as "duplicate batch entry".
    let thumbnail_deletes = mock_storage()
        .calls_for(&thumbnail_address)
        .into_iter()
        .filter(|call| {
            call.op == MetadataOp::Delete && call.key.as_deref() == Some(THUMBNAIL_METADATA_KEY)
        })
        .count();
    assert_eq!(
        thumbnail_deletes, 1,
        "phase-1 batch must dedupe the shared thumbnail address to a single delete call"
    );

    assert!(!mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY));
    assert!(!mock_storage().has_key(&source_address, "thumbnail_variant_one"));
    assert!(!mock_storage().has_key(&source_address, "thumbnail_variant_two"));
}
