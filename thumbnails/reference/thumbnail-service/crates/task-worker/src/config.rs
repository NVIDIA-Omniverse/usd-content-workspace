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

//! Worker startup configuration.
//!
//! The worker reads no `THUMBNAIL_*` specification settings: every processing
//! field arrives on the generate message, and substituting a local default for
//! one the publisher omitted is exactly the drift the shared contract exists to
//! prevent. What it does configure is where to talk, how hard to work, and how
//! much decode it will accept.

use std::time::Duration;

use clap::{ArgAction, Args, Parser};
use thumbnail_common::OutboundAuthArgs;
use thumbnail_convert::DecodeLimits;

#[derive(Parser, Debug)]
#[command(name = "task-worker", about = "Thumbnail generation worker")]
pub struct WorkerCli {
    #[command(flatten)]
    pub auth: OutboundAuthArgs,

    #[command(flatten)]
    pub worker: WorkerArgs,
}

/// The one argument `--probe-ready` needs, sharing the definition below so the
/// probe and the server it probes cannot resolve the port differently.
#[derive(Parser, Debug)]
#[command(name = "task-worker", disable_help_flag = true)]
pub struct ProbeCli {
    #[arg(
        long = "health-port",
        env = "HEALTH_PORT",
        default_value_t = DEFAULT_HEALTH_PORT,
        value_parser = clap::value_parser!(u16).range(1..),
    )]
    pub health_port: u16,
}

/// Default probe port, shared by [`WorkerArgs`] and [`ProbeCli`].
pub const DEFAULT_HEALTH_PORT: u16 = 8080;

#[derive(Args, Clone, Debug)]
pub struct WorkerArgs {
    // --- Endpoints ---------------------------------------------------------
    #[arg(
        long = "notifications-event-consumer-endpoint",
        env = "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
        value_parser = thumbnail_common::config::parse_plaintext_endpoint,
    )]
    pub notifications_event_consumer_endpoint: String,

    /// Resource-prefix filters for WorkerEventQueue, required — see
    /// [`WorkerArgs::resolve_queue_provisioning`]. Also the input to
    /// WorkerEventQueue's derived `queue_id`, which is why this is not
    /// optional: per the platform's observed behavior, an empty filter list
    /// does not mean "every `omni.thumbnail.generate` event" — it binds a
    /// queue that receives nothing (see `build_generate_filter_groups`).
    #[arg(
        long = "notifications-queue-resource-prefixes",
        env = "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        value_delimiter = ',',
        value_parser = parse_non_empty,
    )]
    pub notifications_queue_resource_prefixes: Option<Vec<String>>,

    #[arg(
        long = "storage-grpc-endpoint",
        env = "STORAGE_GRPC_ENDPOINT",
        value_parser = thumbnail_common::config::parse_plaintext_endpoint,
    )]
    pub storage_grpc_endpoint: String,

    /// The thumbnail-service gRPC endpoint the worker calls `AssociateThumbnail`
    /// on. Association is the service's job, not the worker's.
    #[arg(
        long = "thumbnail-service-grpc-endpoint",
        env = "THUMBNAIL_SERVICE_GRPC_ENDPOINT",
        value_parser = thumbnail_common::config::parse_plaintext_endpoint,
    )]
    pub thumbnail_service_grpc_endpoint: String,

    // --- Concurrency and retry --------------------------------------------
    /// Conversions in flight at once, per replica. The number of workers is the
    /// Kubernetes replica count, not a setting here: one blast radius per pod for
    /// image decode, and the HPA does the scaling.
    ///
    /// Two rather than four because the pod's memory limit is
    /// `max_concurrent_conversions × max_transform_peak_bytes + idle`, and
    /// buying the fourth concurrent conversion costs 2 GiB of ceiling for
    /// throughput the HPA can supply by adding a replica instead.
    #[arg(
        long = "max-concurrent-conversions",
        env = "MAX_CONCURRENT_CONVERSIONS",
        default_value_t = 2,
        value_parser = parse_positive_usize,
    )]
    pub max_concurrent_conversions: usize,

    /// How long a retryably-failed delivery waits before its negative
    /// acknowledgement is sent, spacing successive attempts at the same work.
    ///
    /// The broker requeues a negatively acknowledged delivery immediately, so this
    /// is the only thing separating attempts. It kept its name and its 60-second
    /// default from the recycle timer it replaced, which supplied the same spacing
    /// as a side effect of reconnecting; comparable to the 60-second Celery retry
    /// countdown before that. Bounded below for the same reason as before: too low
    /// turns a persistent failure into a hot loop.
    #[arg(
        long = "redelivery-recycle-interval",
        env = "REDELIVERY_RECYCLE_INTERVAL",
        default_value = "60s",
        value_parser = parse_recycle_interval,
    )]
    pub redelivery_recycle_interval: Duration,

    /// Most deliveries whose negative acknowledgement may be held at once.
    ///
    /// Held deliveries stay in-flight, so an unbounded number of them refills the
    /// prefetch window and stops delivery — the 2026-08-24 failure. Past the bound
    /// a failure is negatively acknowledged immediately and the anti-spin gate
    /// spaces the redelivery instead.
    #[arg(
        long = "nack-deferral-concurrency-limit",
        env = "NACK_DEFERRAL_CONCURRENCY_LIMIT",
        default_value_t = 10,
        value_parser = parse_positive_usize,
    )]
    pub nack_deferral_concurrency_limit: usize,

    /// Consecutive early redeliveries of one delivery key before the stream is
    /// recycled to force spacing by not being connected.
    ///
    /// Reached only when the broker requeues without honouring the delay, which
    /// otherwise leaves the consumer trading negative acknowledgements with it.
    #[arg(
        long = "nack-early-redelivery-limit",
        env = "NACK_EARLY_REDELIVERY_LIMIT",
        default_value_t = 5,
        value_parser = parse_positive_u32,
    )]
    pub nack_early_redelivery_limit: u32,

    /// Bound on every outbound call, so a hung dependency releases its
    /// conversion slot instead of occupying it indefinitely.
    #[arg(
        long = "outbound-request-timeout",
        env = "OUTBOUND_REQUEST_TIMEOUT",
        default_value = "30s",
        value_parser = parse_duration_or_millis,
    )]
    pub outbound_request_timeout: Duration,

    /// Grace period for in-flight conversions after SIGTERM. Work that does not
    /// finish inside it stays unacknowledged and comes back.
    #[arg(
        long = "shutdown-drain-timeout",
        env = "SHUTDOWN_DRAIN_TIMEOUT",
        default_value = "30s",
        value_parser = parse_duration_or_millis,
    )]
    pub shutdown_drain_timeout: Duration,

    /// How long the worker waits for the consume stream to be accepted.
    ///
    /// A `Duration`, so the consume loop uses it as-is rather than rebuilding the
    /// unit at the call site. Parsed as whole seconds rather than through
    /// `parse_duration_or_millis`, because that reads a bare integer as
    /// milliseconds and the variable is named `_SECS`: an existing
    /// `STREAM_TIMEOUT_SECS=90` has to keep meaning ninety seconds.
    #[arg(
        long = "stream-timeout-secs",
        env = "STREAM_TIMEOUT_SECS",
        default_value = "90",
        value_parser = parse_seconds,
    )]
    pub stream_establish_timeout: Duration,

    #[arg(
        long = "reconnect-base-backoff-seconds",
        env = "RECONNECT_BASE_BACKOFF_SECONDS",
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub reconnect_base_backoff_seconds: u64,

    #[arg(
        long = "max-reconnect-backoff-seconds",
        env = "MAX_RECONNECT_BACKOFF_SECONDS",
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub max_reconnect_backoff_seconds: u64,

    // --- Decode bounds -----------------------------------------------------
    /// Largest source the worker will download and decode.
    #[arg(
        long = "max-source-bytes",
        env = "MAX_SOURCE_BYTES",
        default_value_t = DecodeLimits::DEFAULT_MAX_SOURCE_BYTES,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub max_source_bytes: u64,

    /// Largest decoded image the worker will allocate for. Sized together with
    /// `max_concurrent_conversions` and the pod's memory limit: the worst case is
    /// the product of the two, times eight bytes per pixel. That eight needs
    /// both a PNG source (16-bit gray+alpha decodes to 4 B/px and passes
    /// `max_alloc` at the full pixel bound) and an output encoding that cannot
    /// carry alpha (JPEG). Chart-default PNG output skips flatten and stays at
    /// 4 B/px. Either half alone is not the 8 B/px path.
    #[arg(
        long = "max-decoded-pixels",
        env = "MAX_DECODED_PIXELS",
        default_value_t = DecodeLimits::DEFAULT_MAX_DECODED_PIXELS,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub max_decoded_pixels: u64,

    /// Largest peak allocation a single resample may reach.
    ///
    /// Separate from `max_decoded_pixels` because the resample allocates more
    /// than the image it produces: its working buffer takes the *source's*
    /// width and the *target's* height at 16 B/px, so a source well inside the
    /// decode bound can still peak in gigabytes for a small thumbnail. This is
    /// the figure the pod's memory limit is sized against, times
    /// `max_concurrent_conversions`.
    #[arg(
        long = "max-transform-peak-bytes",
        env = "MAX_TRANSFORM_PEAK_BYTES",
        default_value_t = DecodeLimits::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub max_transform_peak_bytes: u64,

    // --- Probe surface -----------------------------------------------------
    #[arg(
        long = "health-port",
        env = "HEALTH_PORT",
        default_value_t = DEFAULT_HEALTH_PORT,
        value_parser = clap::value_parser!(u16).range(1..),
    )]
    pub health_port: u16,

    #[arg(
        long = "development-mode",
        env = "DEVELOPMENT_MODE",
        action = ArgAction::Set,
        value_parser = parse_boolish,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false,
    )]
    pub development_mode: bool,
}

impl WorkerArgs {
    /// Test configuration built directly, without going through clap.
    ///
    /// Deliberately not `try_parse_from`: clap treats the environment as a value
    /// source, so a runner that exports `ENABLE_CLIENT_CREDENTIALS` or
    /// `STORAGE_GRPC_ENDPOINT` — as this repository's `rust:test` job does —
    /// changes what a test parses. Building the struct makes a test's inputs
    /// exactly what it writes down, and lets integration tests run concurrently
    /// instead of serializing on process-global state.
    ///
    /// Tests that are specifically about argument parsing still use clap, with
    /// the ambient environment cleared.
    #[doc(hidden)]
    pub fn for_tests() -> Self {
        Self {
            notifications_event_consumer_endpoint: "notifications:50051".to_string(),
            notifications_queue_resource_prefixes: Some(vec!["https://test/".to_string()]),
            storage_grpc_endpoint: "storage:50001".to_string(),
            thumbnail_service_grpc_endpoint: "thumbnail-service:50051".to_string(),
            max_concurrent_conversions: 4,
            redelivery_recycle_interval: Duration::from_secs(60),
            nack_deferral_concurrency_limit: 10,
            nack_early_redelivery_limit: 5,
            outbound_request_timeout: Duration::from_secs(30),
            shutdown_drain_timeout: Duration::from_secs(30),
            stream_establish_timeout: Duration::from_secs(90),
            reconnect_base_backoff_seconds: 5,
            max_reconnect_backoff_seconds: 60,
            max_source_bytes: DecodeLimits::DEFAULT_MAX_SOURCE_BYTES,
            max_decoded_pixels: DecodeLimits::DEFAULT_MAX_DECODED_PIXELS,
            max_transform_peak_bytes: DecodeLimits::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
            health_port: DEFAULT_HEALTH_PORT,
            development_mode: false,
        }
    }

    pub fn decode_limits(&self) -> DecodeLimits {
        DecodeLimits {
            max_source_bytes: self.max_source_bytes,
            max_decoded_pixels: self.max_decoded_pixels,
            max_transform_peak_bytes: self.max_transform_peak_bytes,
        }
    }

    /// The subset of stream tuning the shared provisioning helper reads.
    ///
    /// Lives here rather than in the binary because both the startup
    /// provisioning call and the consume loop's `NOT_FOUND` re-provisioning need
    /// it, and two copies would be two places to forget a field.
    pub fn stream_runtime_args(&self) -> thumbnail_common::config::StreamRuntimeArgs {
        thumbnail_common::config::StreamRuntimeArgs {
            // `StreamRuntimeArgs` speaks whole seconds; converting here keeps that
            // at the boundary to a foreign config type instead of in the loop.
            stream_timeout_secs: self.stream_establish_timeout.as_secs(),
            reconnect_base_backoff_seconds: self.reconnect_base_backoff_seconds,
            max_reconnect_backoff_seconds: self.max_reconnect_backoff_seconds,
            ..Default::default()
        }
    }

    /// Validate and resolve WorkerEventQueue's durable-queue provisioning
    /// contract: required, canonicalized resource prefixes and the id
    /// derived from them ([`thumbnail_common::worker_event_queue_id`]).
    /// Structurally cannot collide with StorageEventQueue's derived id for
    /// the same prefix set — the two ids share that set's digest and differ
    /// in a fixed-position literal segment (`-worker-` vs. `-storage-`), so
    /// no digest value can make them equal.
    pub fn resolve_queue_provisioning(
        &self,
    ) -> Result<thumbnail_common::QueueProvisioning, thumbnail_common::ProvisioningError> {
        let resource_prefixes = thumbnail_common::required_resource_prefixes(
            self.notifications_queue_resource_prefixes.clone(),
        )?;
        Ok(thumbnail_common::QueueProvisioning {
            queue_id: thumbnail_common::worker_event_queue_id(&resource_prefixes),
            resource_prefixes,
        })
    }
}

/// Every environment variable `WorkerArgs` reads.
///
/// Mirrors `thumbnail_common::outbound_auth_env_keys` and exists for the same
/// reason: tests that assert on argument parsing have to clear the ambient
/// environment first, because clap treats these as value sources and a CI
/// runner's `variables:` block would otherwise silently satisfy an argument the
/// test is trying to prove is required.
pub fn worker_env_keys() -> &'static [&'static str] {
    &[
        "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
        "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        "STORAGE_GRPC_ENDPOINT",
        "THUMBNAIL_SERVICE_GRPC_ENDPOINT",
        "MAX_CONCURRENT_CONVERSIONS",
        "REDELIVERY_RECYCLE_INTERVAL",
        "OUTBOUND_REQUEST_TIMEOUT",
        "SHUTDOWN_DRAIN_TIMEOUT",
        "STREAM_TIMEOUT_SECS",
        "RECONNECT_BASE_BACKOFF_SECONDS",
        "MAX_RECONNECT_BACKOFF_SECONDS",
        "MAX_SOURCE_BYTES",
        "MAX_DECODED_PIXELS",
        "HEALTH_PORT",
        "DEVELOPMENT_MODE",
    ]
}

/// The floor exists because the recycle is a stream reconnect. Below a few
/// seconds the worker would spend its life re-establishing the session instead
/// of converting.
const MIN_RECYCLE_INTERVAL: Duration = Duration::from_secs(5);

fn parse_recycle_interval(value: &str) -> Result<Duration, String> {
    let duration = parse_duration_or_millis(value)?;
    if duration < MIN_RECYCLE_INTERVAL {
        return Err(format!(
            "redelivery recycle interval must be at least {}s; a shorter one reconnects the \
             consume stream faster than it can drain, which starves conversion",
            MIN_RECYCLE_INTERVAL.as_secs()
        ));
    }
    Ok(duration)
}

/// Whole seconds, for settings whose environment variable already names the unit.
fn parse_seconds(value: &str) -> Result<Duration, String> {
    let seconds: u64 = value
        .trim()
        .parse()
        .map_err(|_| format!("expected a whole number of seconds, got '{value}'"))?;
    if seconds == 0 {
        return Err("must be at least 1 second".to_string());
    }
    Ok(Duration::from_secs(seconds))
}

fn parse_duration_or_millis(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    if let Ok(duration) = humantime::parse_duration(trimmed) {
        return Ok(duration);
    }
    if let Ok(milliseconds) = trimmed.parse::<u64>() {
        return Ok(Duration::from_millis(milliseconds));
    }
    Err(format!(
        "invalid duration '{trimmed}': expected humantime (e.g. '250ms', '2s') or integer \
         milliseconds"
    ))
}

/// `clap::value_parser!(usize)` has no `range`, so the floor is spelled out.
fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed: usize = value
        .trim()
        .parse()
        .map_err(|_| format!("expected a positive integer, got '{value}'"))?;
    if parsed == 0 {
        return Err("must be at least 1".to_string());
    }
    Ok(parsed)
}

/// Zero would fire the circuit breaker on a delivery's first early redelivery,
/// recycling the stream instead of spacing the retry.
fn parse_positive_u32(value: &str) -> Result<u32, String> {
    let parsed: u32 = value
        .trim()
        .parse()
        .map_err(|_| format!("expected a positive integer, got '{value}'"))?;
    if parsed == 0 {
        return Err("must be at least 1".to_string());
    }
    Ok(parsed)
}

fn parse_non_empty(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn parse_boolish(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(format!("expected true/false/1/0, got '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears every variable clap would read, restoring them on drop.
    ///
    /// Without this the suite passes on a clean laptop and fails in CI, whose
    /// `rust:test` job exports `ENABLE_CLIENT_CREDENTIALS`, `STORAGE_GRPC_ENDPOINT`,
    /// and `NOTIFICATIONS_QUEUE_ID` among others. Those turn "this argument is
    /// required" into "this argument has a value", and the auth flags conflict
    /// outright. `crates/common`'s outbound tests hit the same wall and solved it
    /// the same way.
    struct EnvGuard {
        snapshot: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn cleared() -> Self {
            let keys = thumbnail_common::outbound_auth_env_keys()
                .iter()
                .chain(worker_env_keys().iter());

            let snapshot: Vec<(&'static str, Option<String>)> =
                keys.map(|key| (*key, std::env::var(key).ok())).collect();
            for (key, _) in &snapshot {
                std::env::remove_var(key);
            }
            EnvGuard { snapshot }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.snapshot {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn args_from(extra: &[&str]) -> Result<WorkerCli, clap::Error> {
        let _guard = EnvGuard::cleared();
        let mut argv = vec![
            "task-worker",
            "--enable-access-token-file",
            "--access-token-file-path=/etc/secrets/token",
            "--notifications-event-consumer-endpoint=notifications:50051",
            "--storage-grpc-endpoint=storage:50001",
            "--thumbnail-service-grpc-endpoint=thumbnail-service:50051",
        ];
        argv.extend_from_slice(extra);
        WorkerCli::try_parse_from(argv)
    }

    #[test]
    #[serial_test::serial]
    fn the_minimum_configuration_parses() {
        let cli = args_from(&[]).expect("the documented minimum should parse");

        assert_eq!(cli.worker.max_concurrent_conversions, 2);
        assert_eq!(
            cli.worker.redelivery_recycle_interval,
            Duration::from_secs(60)
        );
    }

    #[test]
    #[serial_test::serial]
    fn every_required_endpoint_is_actually_required() {
        let _guard = EnvGuard::cleared();

        for missing in [
            "--notifications-event-consumer-endpoint",
            "--storage-grpc-endpoint",
            "--thumbnail-service-grpc-endpoint",
        ] {
            let argv: Vec<&str> = vec![
                "task-worker",
                "--enable-access-token-file",
                "--access-token-file-path=/etc/secrets/token",
                "--notifications-event-consumer-endpoint=notifications:50051",
                "--storage-grpc-endpoint=storage:50001",
                "--thumbnail-service-grpc-endpoint=thumbnail-service:50051",
            ]
            .into_iter()
            .filter(|arg| !arg.starts_with(missing))
            .collect();

            assert!(
                WorkerCli::try_parse_from(argv).is_err(),
                "{missing} should be required"
            );
        }
    }

    /// A recycle interval below the floor would reconnect the consume stream
    /// faster than it can drain, so it is refused at startup rather than
    /// silently clamped.
    #[test]
    #[serial_test::serial]
    fn a_too_short_recycle_interval_is_refused_at_startup() {
        let error =
            args_from(&["--redelivery-recycle-interval=1s"]).expect_err("1s is below the floor");

        assert!(
            error.to_string().contains("at least 5s"),
            "the error should name the floor, got: {error}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn the_recycle_floor_itself_is_accepted() {
        let cli = args_from(&["--redelivery-recycle-interval=5s"]).expect("5s is the floor");
        assert_eq!(
            cli.worker.redelivery_recycle_interval,
            Duration::from_secs(5)
        );
    }

    #[test]
    #[serial_test::serial]
    fn zero_concurrency_is_refused() {
        assert!(
            args_from(&["--max-concurrent-conversions=0"]).is_err(),
            "a worker that can run no conversions is misconfigured, not idle"
        );
    }

    /// The probe and the server must resolve the port from one definition, or a
    /// container health check reports a healthy worker as unhealthy.
    #[test]
    #[serial_test::serial]
    fn the_probe_resolves_the_same_port_as_the_server() {
        // One guard for the whole test: `args_from` clears the environment on
        // every call, which would erase the variable being tested here.
        let _guard = EnvGuard::cleared();

        let required = [
            "task-worker",
            "--enable-access-token-file",
            "--access-token-file-path=/etc/secrets/token",
            "--notifications-event-consumer-endpoint=notifications:50051",
            "--storage-grpc-endpoint=storage:50001",
            "--thumbnail-service-grpc-endpoint=thumbnail-service:50051",
        ];
        let server_port = || {
            WorkerCli::try_parse_from(required)
                .expect("required arguments parse")
                .worker
                .health_port
        };

        assert_eq!(
            ProbeCli::parse_from(["task-worker"]).health_port,
            server_port(),
            "both default to the same port"
        );

        std::env::set_var("HEALTH_PORT", "9090");
        assert_eq!(
            ProbeCli::parse_from(["task-worker"]).health_port,
            9090,
            "the probe honors HEALTH_PORT"
        );
        assert_eq!(
            server_port(),
            9090,
            "and resolves it to the same value the server binds"
        );
        std::env::remove_var("HEALTH_PORT");

        assert_eq!(
            ProbeCli::parse_from(["task-worker", "--health-port", "7000"]).health_port,
            7000,
            "an explicit flag reaches the probe too, not just the environment"
        );
    }

    #[test]
    #[serial_test::serial]
    fn decode_bounds_come_from_configuration() {
        let cli = args_from(&["--max-source-bytes=1024", "--max-decoded-pixels=2048"])
            .expect("explicit bounds should parse");

        let limits = cli.worker.decode_limits();
        assert_eq!(limits.max_source_bytes, 1024);
        assert_eq!(limits.max_decoded_pixels, 2048);
    }

    #[test]
    #[serial_test::serial]
    fn resource_prefixes_are_required_and_canonicalized() {
        let cli = args_from(&[]).expect("no prefixes should still parse via clap");
        let err = cli.worker.resolve_queue_provisioning().expect_err(
            "an empty prefix list must fail provisioning, not self-provision a dead queue",
        );
        assert_eq!(
            err,
            thumbnail_common::ProvisioningError::MissingResourcePrefixes,
            "got: {err}"
        );

        let cli = args_from(&[
            "--notifications-queue-resource-prefixes=https://b/,https://a/,https://a/",
        ])
        .expect("prefixes should parse");
        let provisioning = cli
            .worker
            .resolve_queue_provisioning()
            .expect("non-empty prefixes should resolve");
        assert_eq!(
            provisioning.resource_prefixes,
            vec!["https://a/".to_string(), "https://b/".to_string()],
            "a prefix listed twice must not produce two binding keys, and the surviving \
             set must be lexicographically sorted"
        );
    }

    #[test]
    #[serial_test::serial]
    fn worker_queue_id_is_derived_from_prefixes_and_cannot_collide_with_storage() {
        let cli = args_from(&["--notifications-queue-resource-prefixes=https://a/"])
            .expect("prefixes should parse");
        let provisioning = cli
            .worker
            .resolve_queue_provisioning()
            .expect("non-empty prefixes should resolve");

        let storage_id = thumbnail_common::storage_event_queue_id(&["https://a/".to_string()]);
        assert_ne!(
            provisioning.queue_id, storage_id,
            "WorkerEventQueue and StorageEventQueue ids must differ for the same prefix set"
        );

        let other = args_from(&["--notifications-queue-resource-prefixes=https://b/"])
            .expect("prefixes should parse")
            .worker
            .resolve_queue_provisioning()
            .expect("non-empty prefixes should resolve");
        assert_ne!(
            provisioning.queue_id, other.queue_id,
            "a different prefix set must derive a different WorkerEventQueue id"
        );
    }

    /// The two ids differ only in their label, so an operator comparing a queue
    /// listing can read "same digest" as "both components agree on the prefix
    /// set" — the reason the kind discriminator was taken out of the payload.
    #[test]
    #[serial_test::serial]
    fn worker_and_storage_ids_share_one_digest_for_the_same_prefix_set() {
        let prefixes = ["https://a/".to_string(), "https://b/".to_string()];
        let worker_id = thumbnail_common::worker_event_queue_id(&prefixes);
        let storage_id = thumbnail_common::storage_event_queue_id(&prefixes);

        let worker_digest = worker_id
            .strip_prefix("thumbnail-service-worker-")
            .expect("worker id must carry its label");
        let storage_digest = storage_id
            .strip_prefix("thumbnail-service-storage-")
            .expect("storage id must carry its label");

        assert_eq!(
            worker_digest, storage_digest,
            "one prefix set must produce one digest for both queues"
        );
        assert_eq!(worker_digest.len(), 64, "digest must be full sha256 hex");
        assert_ne!(
            worker_id, storage_id,
            "sharing a digest must not make the two ids equal — the label separates them"
        );
    }

    /// A drifted prefix set between the two components has to be visible as
    /// differing digests, which is the diagnostic the shared-digest scheme buys.
    #[test]
    #[serial_test::serial]
    fn differing_prefix_sets_produce_differing_digests_across_the_two_queues() {
        let storage_id = thumbnail_common::storage_event_queue_id(&["https://a/".to_string()]);
        let worker_id = thumbnail_common::worker_event_queue_id(&[
            "https://a/".to_string(),
            "https://b/".to_string(),
        ]);

        let storage_digest = storage_id
            .strip_prefix("thumbnail-service-storage-")
            .expect("storage id must carry its label");
        let worker_digest = worker_id
            .strip_prefix("thumbnail-service-worker-")
            .expect("worker id must carry its label");

        assert_ne!(
            storage_digest, worker_digest,
            "components configured with different prefix sets must not appear to agree"
        );
    }

    #[test]
    #[serial_test::serial]
    fn durations_accept_humantime_and_bare_milliseconds() {
        let cli = args_from(&["--outbound-request-timeout=1500"]).expect("bare millis");
        assert_eq!(
            cli.worker.outbound_request_timeout,
            Duration::from_millis(1500)
        );

        let cli = args_from(&["--outbound-request-timeout=2s"]).expect("humantime");
        assert_eq!(cli.worker.outbound_request_timeout, Duration::from_secs(2));

        let cli =
            args_from(&["--outbound-request-timeout=1m30s"]).expect("composite humantime duration");
        assert_eq!(cli.worker.outbound_request_timeout, Duration::from_secs(90));

        assert!(
            parse_duration_or_millis(&format!("{}h", u64::MAX)).is_err(),
            "an overflowing duration must be rejected instead of wrapping"
        );
    }

    /// The worker must not read the publisher's specification settings: every
    /// processing field travels on the message, and a local default here would
    /// silently diverge from what the publisher resolved.
    #[test]
    #[serial_test::serial]
    fn no_thumbnail_specification_settings_are_read() {
        let rendered = format!("{:?}", args_from(&[]).expect("parse").worker);

        for leaked in [
            "thumbnail_sizes",
            "thumbnail_encoding_format",
            "thumbnail_transform_type",
            "thumbnail_crop_strategy",
            "thumbnail_quality",
            "thumbnail_path_template",
        ] {
            assert!(
                !rendered.contains(leaked),
                "the worker must not carry '{leaked}'; it comes from the generate message"
            );
        }
    }
}
