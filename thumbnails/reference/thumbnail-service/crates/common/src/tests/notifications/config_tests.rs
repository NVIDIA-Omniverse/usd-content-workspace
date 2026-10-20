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

//! Tests for notification stream runtime configuration.

// Every test in this file mutates the process environment, and clap reads the
// environment when parsing. `EnvGuard` restores what a test changed, but
// restoration is no defence against a *concurrent* reader: another test parsing
// the same runtime config while a blank
// `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` is set sees the blank and fails its
// parse. That is what happened when removing an unrelated test file changed the
// scheduling enough to overlap them.
//
// So these are serialized against each other and against the config-parsing
// tests elsewhere. `serial_test`'s default group is global, which is what makes
// the exclusion mutual across files.

use serial_test::serial;
use std::env;
use std::time::Duration;

const STREAM_ENV_KEYS: [&str; 8] = [
    "EVENT_PROCESSING_MAX_CONCURRENT",
    "MAX_RECONNECT_BACKOFF_SECONDS",
    "RECONNECT_BASE_BACKOFF_SECONDS",
    "TASK_CLEANUP_TIMEOUT_SECS",
    "RECONNECT_BACKOFF_EXPONENT_CAP",
    "HEALTH_WARNING_INTERVAL",
    "STREAM_TIMEOUT_SECS",
    "EVENT_TASK_SUBMISSION_MAX_RETRY_AGE",
];

/// Env vars mutated by the durable-queue provisioning config tests. Every such
/// test snapshots and restores this full set so it cleans up after itself.
const QUEUE_PROVISIONING_ENV_KEYS: [&str; 4] = [
    "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
    "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
    "EVENT_CONSUMER_SERVER",
    "DISABLE_EVENT_CONSUMPTION",
];

/// Env var mutated by the provisioning-retry-budget config tests.
const PROVISIONING_MAX_RETRIES_ENV_KEYS: [&str; 1] = ["NOTIFICATIONS_PROVISIONING_MAX_RETRIES"];

const GENERATED_PRINCIPAL_ENV_KEYS: [&str; 1] = ["GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY"];

/// Sample positive provisioning-retry budget used by the parse test.
const SAMPLE_PROVISIONING_MAX_RETRIES: u32 = 7;

/// `thumbnail-service-storage-` + lowercase hex sha256(`s3://bucket/thumbnails`).
const SAMPLE_DERIVED_QUEUE_ID: &str =
    "thumbnail-service-storage-7ddd996359738b306ad1581ecb63616566bdf9def4e94b750712e555397900fd";

/// `thumbnail-service-storage-` + lowercase hex sha256 of A,B,C sorted and joined.
const PREFIX_ABC_DERIVED_QUEUE_ID: &str =
    "thumbnail-service-storage-c539b6bcf71709806119c0b32eb50097cb5515c65f5bed94a9ada81833f15ae7";

/// `thumbnail-service-storage-` + lowercase hex sha256 of A,B sorted and joined.
const PREFIX_AB_DERIVED_QUEUE_ID: &str =
    "thumbnail-service-storage-e9b0bd594d22f7f0dde21156478031a0d080dd0364bd4852806b87897bb59a09";

const PREFIX_A_DERIVED_QUEUE_ID: &str =
    "thumbnail-service-storage-f04ec1202f7fa634a13e8076fc359f1bfe04ac9732a935113105cd7a3b80cbdf";

/// Sample queue resource (storage-container URL) prefix used by the tests.
const SAMPLE_RESOURCE_PREFIX: &str = "s3://bucket/thumbnails";

/// Three distinct resource prefixes used to exercise comma-separated parsing.
const RESOURCE_PREFIX_A: &str = "s3://bucket/alpha";
const RESOURCE_PREFIX_B: &str = "s3://bucket/beta";
const RESOURCE_PREFIX_C: &str = "s3://bucket/gamma";

fn snapshot_env(keys: &[&str]) -> Vec<(String, Option<String>)> {
    keys.iter()
        .map(|key| ((*key).to_string(), crate::config::raw_env_value(key)))
        .collect()
}

fn restore_env(saved: Vec<(String, Option<String>)>) {
    for (key, value) in saved {
        match value {
            Some(v) => env::set_var(&key, v),
            None => env::remove_var(&key),
        }
    }
}

struct EnvRestoreGuard(Option<Vec<(String, Option<String>)>>);

impl EnvRestoreGuard {
    fn new(keys: &[&str]) -> Self {
        Self(Some(snapshot_env(keys)))
    }
}

impl Drop for EnvRestoreGuard {
    fn drop(&mut self) {
        restore_env(
            self.0
                .take()
                .expect("environment restore guard must run only once"),
        );
    }
}

#[test]
#[serial]
fn test_stream_runtime_default_values() {
    let saved = snapshot_env(&STREAM_ENV_KEYS);
    for key in STREAM_ENV_KEYS {
        env::remove_var(key);
    }

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let stream = runtime.stream;
    assert_eq!(stream.event_processing_max_concurrent, 100);
    assert_eq!(stream.max_reconnect_backoff_seconds, 60);
    assert_eq!(stream.reconnect_base_backoff_seconds, 5);
    assert_eq!(stream.task_cleanup_timeout_secs, 30);
    assert_eq!(stream.reconnect_backoff_exponent_cap, 4);
    assert_eq!(stream.health_warning_interval, 10);
    assert_eq!(stream.stream_timeout_secs, 90);
    assert_eq!(
        stream.event_task_submission_max_retry_age,
        Duration::from_secs(3600)
    );

    restore_env(saved);
}

#[test]
#[serial]
fn generated_thumbnail_principal_identity_is_optional() {
    let _restore = EnvRestoreGuard::new(&GENERATED_PRINCIPAL_ENV_KEYS);
    env::remove_var("GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();

    assert_eq!(
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity,
        None
    );
}

#[test]
#[serial_test::serial]
fn configured_generated_thumbnail_principal_identity_is_trimmed() {
    let runtime = crate::tests::try_parse_runtime_config_for_tests(&[
        "--generated-thumbnail-principal-identity",
        "  thumbnail-task-worker  ",
    ])
    .expect("the optional principal should accept a non-empty value");

    assert_eq!(
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity
            .as_deref(),
        Some("thumbnail-task-worker")
    );
}

/// `StreamRuntimeArgs::default()` exists so a caller without a command line —
/// `task-worker`, passing it to `ensure_durable_queue` — does not have to parse
/// an empty argv, which would read the ambient environment. That only helps if
/// it agrees with what clap would have produced, and nothing but this test keeps
/// the two lists in step.
#[test]
#[serial]
fn stream_runtime_defaults_match_the_clap_attributes() {
    let _restore = EnvRestoreGuard::new(&STREAM_ENV_KEYS);
    for key in STREAM_ENV_KEYS {
        env::remove_var(key);
    }
    env::remove_var("NOTIFICATIONS_PROVISIONING_MAX_RETRIES");

    let parsed = crate::tests::parse_runtime_config_from_env_for_tests().stream;
    let constructed = crate::config::StreamRuntimeArgs::default();

    assert_eq!(
        format!("{parsed:?}"),
        format!("{constructed:?}"),
        "every field must match, or a caller using Default silently gets \
         different behavior from one that parsed"
    );
}

#[test]
#[serial]
fn test_stream_runtime_custom_values() {
    let saved = snapshot_env(&STREAM_ENV_KEYS);
    env::set_var("EVENT_PROCESSING_MAX_CONCURRENT", "200");
    env::set_var("MAX_RECONNECT_BACKOFF_SECONDS", "240");
    env::set_var("RECONNECT_BASE_BACKOFF_SECONDS", "7");
    env::set_var("TASK_CLEANUP_TIMEOUT_SECS", "41");
    env::set_var("RECONNECT_BACKOFF_EXPONENT_CAP", "5");
    env::set_var("HEALTH_WARNING_INTERVAL", "6");
    env::set_var("STREAM_TIMEOUT_SECS", "45");
    env::set_var("EVENT_TASK_SUBMISSION_MAX_RETRY_AGE", "2h");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let stream = runtime.stream;
    assert_eq!(stream.event_processing_max_concurrent, 200);
    assert_eq!(stream.max_reconnect_backoff_seconds, 240);
    assert_eq!(stream.reconnect_base_backoff_seconds, 7);
    assert_eq!(stream.task_cleanup_timeout_secs, 41);
    assert_eq!(stream.reconnect_backoff_exponent_cap, 5);
    assert_eq!(stream.health_warning_interval, 6);
    assert_eq!(stream.stream_timeout_secs, 45);
    assert_eq!(
        stream.event_task_submission_max_retry_age,
        Duration::from_secs(7200)
    );

    restore_env(saved);
}

/// Rejected values are passed on the argument list rather than exported.
///
/// Exporting them made the invalid value visible to every concurrently running
/// test that parses runtime config, and `#[serial]` does not prevent that — it
/// orders serial tests against each other, not against the rest of the suite.
/// This surfaced in CI as unrelated tests failing on a retry age of `0s` that
/// none of them had set.
#[test]
#[serial_test::serial]
fn test_stream_runtime_invalid_values_are_rejected() {
    assert!(
        crate::tests::try_parse_runtime_config_for_tests(&[
            "--reconnect-base-backoff-seconds",
            "0"
        ])
        .is_err(),
        "RECONNECT_BASE_BACKOFF_SECONDS=0 must fail clap parsing"
    );

    assert!(
        crate::tests::try_parse_runtime_config_for_tests(&[
            "--event-task-submission-max-retry-age",
            "0s"
        ])
        .is_err(),
        "EVENT_TASK_SUBMISSION_MAX_RETRY_AGE=0s must fail clap parsing"
    );
}

#[test]
#[serial]
fn test_stream_backoff_calculation_respects_runtime_caps() {
    let saved = snapshot_env(&STREAM_ENV_KEYS);
    env::set_var("RECONNECT_BASE_BACKOFF_SECONDS", "5");
    env::set_var("RECONNECT_BACKOFF_EXPONENT_CAP", "4");
    env::set_var("MAX_RECONNECT_BACKOFF_SECONDS", "60");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let stream = runtime.stream;

    for attempt in 1..=12 {
        let computed = stream.reconnect_base_backoff_seconds
            * 2u64.pow(attempt.min(stream.reconnect_backoff_exponent_cap));
        let bounded = computed.min(stream.max_reconnect_backoff_seconds);
        assert!(bounded <= stream.max_reconnect_backoff_seconds);
    }

    restore_env(saved);
}

/// Marks this binary as the assigned, enabled event consumer so the
/// `resolve_queue_provisioning` fail-fast contract applies. The helper itself
/// does not re-check these gates (they live in the runtime supervisor), but the
/// tests set them so the scenario mirrors real startup.
fn set_enabled_consumer_env() {
    env::set_var("EVENT_CONSUMER_SERVER", "HTTP");
    env::set_var("DISABLE_EVENT_CONSUMPTION", "false");
    env::set_var(
        "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
        "http://notifications:50051",
    );
}

/// Case 3: `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` unset resolves to `None`.
#[test]
#[serial]
fn test_notifications_queue_resource_prefixes_unset_is_none() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    env::remove_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.endpoints.notifications_queue_resource_prefixes, None,
        "unset NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES must resolve to None"
    );

    restore_env(saved);
}

/// Case 4: a non-empty prefix (with surrounding whitespace) parses to
/// `Some(vec![trimmed])`, matching the `parse_non_empty` convention.
#[test]
#[serial]
fn test_notifications_queue_resource_prefixes_trims_whitespace() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("  {SAMPLE_RESOURCE_PREFIX}  "),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.endpoints.notifications_queue_resource_prefixes,
        Some(vec![SAMPLE_RESOURCE_PREFIX.to_string()]),
        "whitespace-padded prefix must parse to Some(vec![trimmed])"
    );

    restore_env(saved);
}

/// Case 5: empty and whitespace-only `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`
/// values are rejected at parse time.
#[test]
#[serial]
fn test_notifications_queue_resource_prefixes_empty_is_rejected() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);

    for value in ["", "   "] {
        env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", value);
        let result =
            std::panic::catch_unwind(crate::tests::parse_runtime_config_from_env_for_tests);
        assert!(
            result.is_err(),
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES={value:?} must fail parsing"
        );
    }

    restore_env(saved);
}

/// Case 6: fail-fast — queue id present, prefix missing ->
/// error naming `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`.
#[test]
#[serial]
fn test_resolve_queue_provisioning_errors_when_prefix_missing() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::remove_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    let err = result.expect_err("missing resource prefix must be an error");
    assert_eq!(
        err,
        crate::ProvisioningError::MissingResourcePrefixes,
        "error must name the missing variable, got: {err}"
    );

    restore_env(saved);
}

/// StorageEventQueue id is derived from prefixes; `EndpointArgs` has no queue-id env.
#[test]
#[serial]
fn test_resolve_queue_provisioning_derives_queue_id_from_prefixes() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        SAMPLE_RESOURCE_PREFIX,
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: SAMPLE_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![SAMPLE_RESOURCE_PREFIX.to_string()],
        }),
        "StorageEventQueue's id must be derived from prefixes"
    );

    restore_env(saved);
}

/// `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` unset defaults to `4`.
#[test]
#[serial]
fn test_notifications_provisioning_max_retries_defaults_to_four() {
    let saved = snapshot_env(&PROVISIONING_MAX_RETRIES_ENV_KEYS);
    env::remove_var("NOTIFICATIONS_PROVISIONING_MAX_RETRIES");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.stream.notifications_provisioning_max_retries, 4,
        "unset NOTIFICATIONS_PROVISIONING_MAX_RETRIES must default to 4"
    );

    restore_env(saved);
}

/// A set positive `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` parses through.
#[test]
#[serial]
fn test_notifications_provisioning_max_retries_parses_positive_value() {
    let saved = snapshot_env(&PROVISIONING_MAX_RETRIES_ENV_KEYS);
    env::set_var(
        "NOTIFICATIONS_PROVISIONING_MAX_RETRIES",
        SAMPLE_PROVISIONING_MAX_RETRIES.to_string(),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.stream.notifications_provisioning_max_retries, SAMPLE_PROVISIONING_MAX_RETRIES,
        "a set positive NOTIFICATIONS_PROVISIONING_MAX_RETRIES must parse through"
    );

    restore_env(saved);
}

/// Zero is an allowed `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` value, distinct
/// from the backoff exponent cap: it disables provisioning retries deliberately.
#[test]
#[serial]
fn test_notifications_provisioning_max_retries_allows_zero() {
    let saved = snapshot_env(&PROVISIONING_MAX_RETRIES_ENV_KEYS);
    env::set_var("NOTIFICATIONS_PROVISIONING_MAX_RETRIES", "0");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.stream.notifications_provisioning_max_retries, 0,
        "NOTIFICATIONS_PROVISIONING_MAX_RETRIES=0 must parse to 0"
    );

    restore_env(saved);
}

/// Case 9: happy path — both queue id and prefix present resolves to a
/// `QueueProvisioning { .. }`.
#[test]
#[serial]
fn test_resolve_queue_provisioning_succeeds_when_both_present() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        SAMPLE_RESOURCE_PREFIX,
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: SAMPLE_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![SAMPLE_RESOURCE_PREFIX.to_string()],
        }),
        "both values present must resolve to QueueProvisioning"
    );

    restore_env(saved);
}

/// A comma-separated `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` splits into one
/// `resource_prefixes` entry per prefix, then sorted lexicographically.
#[test]
#[serial]
fn test_resolve_queue_provisioning_splits_comma_separated_prefixes() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("{RESOURCE_PREFIX_A},{RESOURCE_PREFIX_B},{RESOURCE_PREFIX_C}"),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: PREFIX_ABC_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![
                RESOURCE_PREFIX_A.to_string(),
                RESOURCE_PREFIX_B.to_string(),
                RESOURCE_PREFIX_C.to_string(),
            ],
        }),
        "a comma-separated list must split into one prefix per entry, then sorted"
    );

    restore_env(saved);
}

/// Two permutations of the same prefix set resolve to the same ordered list.
#[test]
#[serial]
fn test_resolve_queue_provisioning_prefix_order_is_stable_across_permutations() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();

    let expected = Ok(crate::config::QueueProvisioning {
        queue_id: PREFIX_ABC_DERIVED_QUEUE_ID.to_string(),
        resource_prefixes: vec![
            RESOURCE_PREFIX_A.to_string(),
            RESOURCE_PREFIX_B.to_string(),
            RESOURCE_PREFIX_C.to_string(),
        ],
    });

    for value in [
        format!("{RESOURCE_PREFIX_C},{RESOURCE_PREFIX_A},{RESOURCE_PREFIX_B}"),
        format!("{RESOURCE_PREFIX_B},{RESOURCE_PREFIX_C},{RESOURCE_PREFIX_A}"),
        format!("{RESOURCE_PREFIX_A},{RESOURCE_PREFIX_B},{RESOURCE_PREFIX_C}"),
    ] {
        env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", &value);
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let result = runtime.endpoints.resolve_queue_provisioning();
        assert_eq!(
            result, expected,
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES={value:?} must resolve to lexicographic order"
        );
    }

    restore_env(saved);
}

/// Surrounding whitespace on each entry is trimmed by clap's `parse_non_empty`
/// value parser (applied per delimited entry).
#[test]
#[serial]
fn test_resolve_queue_provisioning_trims_prefix_entries() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("  {RESOURCE_PREFIX_A} , {RESOURCE_PREFIX_B}  "),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: PREFIX_AB_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![RESOURCE_PREFIX_A.to_string(), RESOURCE_PREFIX_B.to_string(),],
        }),
        "whitespace must be trimmed per delimited entry"
    );

    restore_env(saved);
}

/// Blank delimited entries (adjacent commas, trailing commas, whitespace-only
/// slots) are rejected at clap parse time by per-entry `parse_non_empty`.
#[test]
#[serial]
fn test_notifications_queue_resource_prefixes_rejects_blank_delimited_entries() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();

    for value in [
        format!("{RESOURCE_PREFIX_A},,{RESOURCE_PREFIX_B}"),
        format!("  {RESOURCE_PREFIX_A} , , {RESOURCE_PREFIX_B} ,"),
        " , , ".to_string(),
    ] {
        env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", &value);
        let result =
            std::panic::catch_unwind(crate::tests::parse_runtime_config_from_env_for_tests);
        assert!(
            result.is_err(),
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES={value:?} must fail parsing on blank entries"
        );
    }

    restore_env(saved);
}

/// Duplicate prefixes are collapsed, then the survivors are sorted
/// lexicographically. Trimming happens before de-duplication so `"a"` and
/// `" a "` count as the same prefix.
#[test]
#[serial]
fn test_resolve_queue_provisioning_dedups_duplicate_prefixes() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    // `A` repeats around `B`; the second `A` must be dropped, then sorted.
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("{RESOURCE_PREFIX_B},{RESOURCE_PREFIX_A},{RESOURCE_PREFIX_B}"),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: PREFIX_AB_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![RESOURCE_PREFIX_A.to_string(), RESOURCE_PREFIX_B.to_string(),],
        }),
        "duplicate prefixes must collapse, then sort lexicographically"
    );

    // Trimming happens before de-duplication, so ` A `, `A`, and `A` collapse
    // to a single `A` alongside a distinct `B`.
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("{RESOURCE_PREFIX_B}, {RESOURCE_PREFIX_A} , {RESOURCE_PREFIX_A}"),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: PREFIX_AB_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![RESOURCE_PREFIX_A.to_string(), RESOURCE_PREFIX_B.to_string(),],
        }),
        "trimming must precede de-duplication so whitespace-only differences collapse"
    );

    restore_env(saved);
}

/// A list consisting solely of repeats of one prefix collapses to a
/// single-element result and is NOT treated as an empty-prefix error.
#[test]
#[serial]
fn test_resolve_queue_provisioning_all_duplicates_resolve_to_single_prefix() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var(
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        format!("{RESOURCE_PREFIX_A},{RESOURCE_PREFIX_A}"),
    );

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let result = runtime.endpoints.resolve_queue_provisioning();
    assert_eq!(
        result,
        Ok(crate::config::QueueProvisioning {
            queue_id: PREFIX_A_DERIVED_QUEUE_ID.to_string(),
            resource_prefixes: vec![RESOURCE_PREFIX_A.to_string()],
        }),
        "an all-duplicate prefix list must resolve to a single prefix, not an error"
    );

    restore_env(saved);
}

/// A value that contains only blank delimited entries (e.g. only commas /
/// whitespace) is rejected at clap parse time by per-entry `parse_non_empty`.
#[test]
#[serial]
fn test_notifications_queue_resource_prefixes_rejects_comma_only_list() {
    let saved = snapshot_env(&QUEUE_PROVISIONING_ENV_KEYS);
    set_enabled_consumer_env();
    env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", " , , ");

    let result = std::panic::catch_unwind(crate::tests::parse_runtime_config_from_env_for_tests);
    assert!(
        result.is_err(),
        "a comma-only NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES must fail parsing"
    );

    restore_env(saved);
}
