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

//! Tests for the network-free durable-queue provisioning pieces:
//! `build_storage_filter_groups` (storage filter-group construction),
//! `classify_provisioning_status` (tonic-`Status` -> permanent-misconfig vs transient),
//! and `decide_provisioning_step` (the pure decision core of `ensure_durable_queue`:
//! success / already-existed, the response-id echo-check, auth short-circuit,
//! permanent-vs-transient routing, timeout handling, and the retry-budget boundary).
//!
//! The stateful I/O of `ensure_durable_queue` (client build, the `create_durable_queue`
//! RPC, token refresh, and the shutdown-aware sleep) still needs a live server and is
//! dev-cluster-tested — only the pure decision it delegates to is unit-tested here.

use crate::notifications::provisioning::{
    build_storage_filter_groups, classify_provisioning_status, decide_provisioning_step,
    EnsureQueueOutcome, ProvisioningAttempt, ProvisioningDecision, ProvisioningErrorClass,
    ProvisioningRetryPolicy,
};
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::resource_filter::FilterType;
use std::time::Duration;
use tonic::{Code, Status};

/// A distinct, non-empty prefix that is not a substring of any hardcoded event type,
/// so threading it through proves the value is not hardcoded.
const RESOURCE_PREFIX: &str = "omniverse://storage/projects/my-project/";

/// Two distinct prefixes (neither a substring of any hardcoded event type) used
/// to prove the per-prefix fan-out preserves the caller-supplied slice order
/// within every group (sorting happens at resolve, not in the builders).
const RESOURCE_PREFIX_A: &str = "omniverse://storage/projects/alpha/";
const RESOURCE_PREFIX_B: &str = "omniverse://storage/projects/beta/";

/// The two storage event types the durable queue subscribes to.
const EVENT_TYPE_CREATED: &str = "omni.storage.created";
const EVENT_TYPE_DELETED: &str = "omni.storage.deleted";

/// Proto value for the greedy prefix-match filter variant (`FILTER_TYPE_STARTS_WITH_GREEDY`).
const GREEDY_FILTER_TYPE: i32 = FilterType::StartsWithGreedy as i32;

#[test]
fn build_storage_filter_groups_produces_exactly_created_and_deleted_groups() {
    // Arrange / Act
    let groups = build_storage_filter_groups(&[RESOURCE_PREFIX.to_string()]);

    // Assert: exactly two groups, precisely the created + deleted event types, no others.
    assert_eq!(groups.len(), 2, "expected exactly two filter groups");

    let event_types: Vec<&str> = groups.iter().map(|g| g.event_type.as_str()).collect();
    assert!(
        event_types.contains(&EVENT_TYPE_CREATED),
        "missing {EVENT_TYPE_CREATED} group; got {event_types:?}"
    );
    assert!(
        event_types.contains(&EVENT_TYPE_DELETED),
        "missing {EVENT_TYPE_DELETED} group; got {event_types:?}"
    );
    // No roundtrip/thumbnail group is present.
    assert!(
        !event_types
            .iter()
            .any(|et| et.contains("roundtrip") || et.contains("thumbnail")),
        "unexpected extra group present; got {event_types:?}"
    );
}

#[test]
fn build_storage_filter_groups_uses_greedy_filter_with_supplied_prefix() {
    // Arrange / Act
    let groups = build_storage_filter_groups(&[RESOURCE_PREFIX.to_string()]);

    // Assert: every group carries exactly one greedy-prefix filter threading the prefix verbatim.
    for group in &groups {
        assert_eq!(
            group.filters.len(),
            1,
            "group {} should carry exactly one filter",
            group.event_type
        );
        let filter = &group.filters[0];
        assert_eq!(
            filter.filter_type, GREEDY_FILTER_TYPE,
            "filter for {} should use FILTER_TYPE_STARTS_WITH_GREEDY",
            group.event_type
        );
        assert_eq!(
            filter.resource_id, RESOURCE_PREFIX,
            "filter for {} should carry the caller-supplied prefix verbatim",
            group.event_type
        );
    }
}

#[test]
fn build_storage_filter_groups_fans_out_one_greedy_filter_per_prefix_in_slice_order() {
    // Arrange: reverse lexicographic order — builders must not re-sort; resolve does.
    let prefixes = vec![RESOURCE_PREFIX_B.to_string(), RESOURCE_PREFIX_A.to_string()];

    // Act
    let groups = build_storage_filter_groups(&prefixes);

    // Assert: still exactly the created + deleted groups — fan-out is per-prefix
    // within a group, it does not multiply the group count.
    assert_eq!(groups.len(), 2, "expected exactly two filter groups");
    let event_types: Vec<&str> = groups.iter().map(|g| g.event_type.as_str()).collect();
    assert!(
        event_types.contains(&EVENT_TYPE_CREATED),
        "missing {EVENT_TYPE_CREATED} group; got {event_types:?}"
    );
    assert!(
        event_types.contains(&EVENT_TYPE_DELETED),
        "missing {EVENT_TYPE_DELETED} group; got {event_types:?}"
    );

    // Assert: each group carries exactly one greedy-prefix filter per input
    // prefix, threading each prefix verbatim, in caller-supplied slice order.
    for group in &groups {
        assert_eq!(
            group.filters.len(),
            prefixes.len(),
            "group {} should carry one filter per configured prefix",
            group.event_type
        );
        for filter in &group.filters {
            assert_eq!(
                filter.filter_type, GREEDY_FILTER_TYPE,
                "every filter in {} should use FILTER_TYPE_STARTS_WITH_GREEDY",
                group.event_type
            );
        }
        let resource_ids: Vec<&str> = group
            .filters
            .iter()
            .map(|f| f.resource_id.as_str())
            .collect();
        assert_eq!(
            resource_ids,
            vec![RESOURCE_PREFIX_B, RESOURCE_PREFIX_A],
            "group {} should carry each prefix verbatim in slice order",
            group.event_type
        );
    }
}

#[test]
fn storage_and_generate_filters_share_resolved_prefix_order() {
    use crate::notifications::provisioning::build_generate_filter_groups;

    // The order `resolve_queue_provisioning` hands both builders: deduplicated and
    // sorted. Dedup and sorting themselves are covered by the config tests.
    let mut prefixes = vec![RESOURCE_PREFIX_B.to_string(), RESOURCE_PREFIX_A.to_string()];
    prefixes.sort();

    let storage = build_storage_filter_groups(&prefixes);
    let generate = build_generate_filter_groups(&prefixes);

    let storage_ids: Vec<&str> = storage[0]
        .filters
        .iter()
        .map(|f| f.resource_id.as_str())
        .collect();
    let generate_ids: Vec<&str> = generate[0]
        .filters
        .iter()
        .map(|f| f.resource_id.as_str())
        .collect();

    assert_eq!(
        storage_ids,
        vec![RESOURCE_PREFIX_A, RESOURCE_PREFIX_B],
        "storage filters should use lexicographic resolve order"
    );
    assert_eq!(
        generate_ids, storage_ids,
        "WorkerEventQueue generate filters must use the same ordered prefix sequence as StorageEventQueue"
    );
}

#[test]
fn classify_provisioning_status_maps_codes_to_expected_class() {
    // Arrange: the full permanent-vs-transient mapping the classifier promises.
    // `InvalidArgument`/`Unimplemented` are permanent misconfigs (malformed request or a
    // notifications server predating the ensure-exists capability); every other code, including
    // the `Internal` default-bucket representative, is transient and worth retrying.
    let cases: &[(Code, ProvisioningErrorClass)] = &[
        (
            Code::InvalidArgument,
            ProvisioningErrorClass::PermanentMisconfig,
        ),
        (
            Code::Unimplemented,
            ProvisioningErrorClass::PermanentMisconfig,
        ),
        (Code::Unavailable, ProvisioningErrorClass::Transient),
        (Code::DeadlineExceeded, ProvisioningErrorClass::Transient),
        (Code::Internal, ProvisioningErrorClass::Transient),
    ];

    for &(code, expected) in cases {
        // Act
        let got = classify_provisioning_status(&Status::new(code, "provisioning error"));

        // Assert
        assert_eq!(got, expected, "code {code:?} misclassified");
    }
}

const SAMPLE_QUEUE_ID: &str = "thumbnail-service-queue";

/// Builds a retry policy with explicit knobs so the decision core is exercised
/// without constructing a full `StreamRuntimeArgs`.
fn policy(
    max_retries: u32,
    base_backoff_secs: u64,
    exponent_cap: u32,
    max_backoff_secs: u64,
) -> ProvisioningRetryPolicy {
    ProvisioningRetryPolicy {
        max_retries,
        base_backoff_secs,
        exponent_cap,
        max_backoff_secs,
    }
}

#[test]
fn decide_responded_created_with_matching_id_succeeds_as_created() {
    let attempt = ProvisioningAttempt::Responded {
        created: true,
        queue_id: SAMPLE_QUEUE_ID.to_string(),
    };
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Succeed(outcome) => assert_eq!(outcome, EnsureQueueOutcome::Created),
        other => panic!("expected Succeed(Created), got {other:?}"),
    }
}

#[test]
fn decide_responded_not_created_with_matching_id_succeeds_as_already_existed() {
    let attempt = ProvisioningAttempt::Responded {
        created: false,
        queue_id: SAMPLE_QUEUE_ID.to_string(),
    };
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Succeed(outcome) => {
            assert_eq!(outcome, EnsureQueueOutcome::AlreadyExisted)
        }
        other => panic!("expected Succeed(AlreadyExisted), got {other:?}"),
    }
}

#[test]
fn decide_responded_with_mismatched_queue_id_fails() {
    // A conformant server echoes the bare id we sent; a different id means the
    // queue was not provisioned under the id the consumer will attach to.
    let attempt = ProvisioningAttempt::Responded {
        created: true,
        queue_id: "some-other-id".to_string(),
    };
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Fail(status) => {
            assert_eq!(status.code(), Code::Internal);
            assert!(status.message().contains(SAMPLE_QUEUE_ID));
            assert!(status.message().contains("some-other-id"));
            assert!(status.message().contains("consumer will not attach"));
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn decide_permission_denied_fails_without_consuming_retry_budget() {
    // PermissionDenied is terminal even with the full retry budget available
    // (retry=0, budget=4): it is the consume-only-identity signal the runtime
    // turns into warn-and-attach.
    let attempt =
        ProvisioningAttempt::Failed(Status::new(Code::PermissionDenied, "no create perm"));
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Fail(status) => assert_eq!(status.code(), Code::PermissionDenied),
        other => panic!("expected Fail(PermissionDenied), got {other:?}"),
    }
}

#[test]
fn decide_persistent_unauthenticated_fails() {
    // A `Unauthenticated` reaching the decision means the refresh-and-retry-once
    // already failed upstream, so it is terminal (not transient).
    let attempt = ProvisioningAttempt::Failed(Status::new(Code::Unauthenticated, "bad token"));
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Fail(status) => assert_eq!(status.code(), Code::Unauthenticated),
        other => panic!("expected Fail(Unauthenticated), got {other:?}"),
    }
}

#[test]
fn decide_invalid_argument_and_unimplemented_fail_fast() {
    // Even with the full retry budget available, permanent codes do not retry.
    for code in [Code::InvalidArgument, Code::Unimplemented] {
        let attempt = ProvisioningAttempt::Failed(Status::new(code, "permanent"));
        match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
            ProvisioningDecision::Fail(status) => assert_eq!(status.code(), code),
            other => panic!("expected Fail for {code:?}, got {other:?}"),
        }
    }
}

#[test]
fn decide_transient_below_budget_retries_with_exponential_backoff() {
    // base=1, cap=4: retry=0 -> next=1 -> 1 * 2^1 = 2s.
    let attempt = ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down"));
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(4, 1, 4, 60)) {
        ProvisioningDecision::Retry(delay) => assert_within_jitter_band(delay, 2),
        other => panic!("expected Retry, got {other:?}"),
    }
}

/// A jittered backoff is asserted as a band, not a value.
///
/// `decide_transient` spreads its delay over the upper half of the computed
/// interval so replicas provisioning against the same Notification deployment do
/// not retry in lock-step. The computed value is still the ceiling, which is what
/// these tests are really about.
#[track_caller]
fn assert_within_jitter_band(actual: Duration, ceiling_secs: u64) {
    let ceiling = Duration::from_secs(ceiling_secs);
    assert!(
        actual <= ceiling,
        "waited {actual:?}, over its {ceiling_secs}s ceiling"
    );
    assert!(
        actual >= ceiling / 2,
        "waited {actual:?}, under half its {ceiling_secs}s ceiling"
    );
}

#[test]
fn decide_transient_backoff_grows_and_is_capped_by_exponent_and_ceiling() {
    // Exponent grows with retry: retry=1 -> next=2 -> 2^2 = 4s (base=1).
    match decide_provisioning_step(
        ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down")),
        SAMPLE_QUEUE_ID,
        1,
        &policy(10, 1, 4, 60),
    ) {
        ProvisioningDecision::Retry(d) => assert_within_jitter_band(d, 4),
        other => panic!("expected Retry(4s), got {other:?}"),
    }
    // Exponent cap: retry=5 -> next=6 -> 2^min(6,4) = 2^4 = 16s.
    match decide_provisioning_step(
        ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down")),
        SAMPLE_QUEUE_ID,
        5,
        &policy(10, 1, 4, 60),
    ) {
        ProvisioningDecision::Retry(d) => assert_within_jitter_band(d, 16),
        other => panic!("expected Retry(16s), got {other:?}"),
    }
    // Ceiling: base=1, cap=10, retry=3 -> next=4 -> 2^4 = 16, capped to max_backoff=8s.
    match decide_provisioning_step(
        ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down")),
        SAMPLE_QUEUE_ID,
        3,
        &policy(10, 1, 10, 8),
    ) {
        ProvisioningDecision::Retry(d) => assert_within_jitter_band(d, 8),
        other => panic!("expected Retry(8s capped), got {other:?}"),
    }
}

#[test]
fn decide_transient_retry_budget_boundary_off_by_one() {
    // max_retries=4: retry=3 (< budget) still retries; retry=4 (== budget) fails.
    let below = decide_provisioning_step(
        ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down")),
        SAMPLE_QUEUE_ID,
        3,
        &policy(4, 1, 4, 60),
    );
    assert!(
        matches!(below, ProvisioningDecision::Retry(_)),
        "retry=3 with budget=4 should retry, got {below:?}"
    );

    let at = decide_provisioning_step(
        ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down")),
        SAMPLE_QUEUE_ID,
        4,
        &policy(4, 1, 4, 60),
    );
    match at {
        ProvisioningDecision::Fail(status) => assert_eq!(status.code(), Code::Unavailable),
        other => panic!("retry=4 with budget=4 should fail, got {other:?}"),
    }
}

#[test]
fn decide_zero_budget_transient_fails_immediately() {
    // max_retries=0 disables retries: the first transient failure is terminal.
    let attempt = ProvisioningAttempt::Failed(Status::new(Code::Unavailable, "down"));
    match decide_provisioning_step(attempt, SAMPLE_QUEUE_ID, 0, &policy(0, 1, 4, 60)) {
        ProvisioningDecision::Fail(status) => assert_eq!(status.code(), Code::Unavailable),
        other => panic!("expected immediate Fail with zero budget, got {other:?}"),
    }
}

#[test]
fn decide_timeout_below_budget_retries() {
    match decide_provisioning_step(
        ProvisioningAttempt::TimedOut,
        SAMPLE_QUEUE_ID,
        0,
        &policy(4, 1, 4, 60),
    ) {
        ProvisioningDecision::Retry(delay) => assert_within_jitter_band(delay, 2),
        other => panic!("expected Retry, got {other:?}"),
    }
}

#[test]
fn decide_timeout_at_budget_fails_as_deadline_exceeded() {
    // A hung server that never responds, once retries are exhausted, fails fast
    // as DeadlineExceeded rather than hanging startup.
    match decide_provisioning_step(
        ProvisioningAttempt::TimedOut,
        SAMPLE_QUEUE_ID,
        4,
        &policy(4, 1, 4, 60),
    ) {
        ProvisioningDecision::Fail(status) => assert_eq!(status.code(), Code::DeadlineExceeded),
        other => panic!("expected Fail(DeadlineExceeded), got {other:?}"),
    }
}
