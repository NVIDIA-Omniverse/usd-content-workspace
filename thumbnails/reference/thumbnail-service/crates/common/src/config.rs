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

//! Shared runtime configuration keys, typed clap args, and env readers.

use crate::{CropStrategy, EncodingFormat, ThumbnailPathTemplate, ThumbnailSizes, TransformType};
use clap::{ArgAction, Args, ValueEnum};
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;

/// Parsed runtime configuration shared across binaries.
#[derive(Args, Clone, Debug)]
pub struct RuntimeConfigArgs {
    #[command(flatten)]
    pub service: ServiceRuntimeArgs,
    #[command(flatten)]
    pub endpoints: EndpointArgs,
    #[command(flatten)]
    pub event_consumer: EventConsumerArgs,
    #[command(flatten)]
    pub stream: StreamRuntimeArgs,
    #[command(flatten)]
    pub storage_channel: StorageChannelArgs,
    #[command(flatten)]
    pub storage_retry: StorageRetryArgs,
    #[command(flatten)]
    pub thumbnail_defaults: ThumbnailDefaultsArgs,
    #[command(flatten)]
    pub thumbnail_path: ThumbnailPathArgs,
}

/// Listener/runtime flags consumed during HTTP/gRPC service startup.
#[derive(Args, Clone, Debug)]
pub struct ServiceRuntimeArgs {
    #[arg(id = "port", long = "port", env = "PORT", default_value_t = 3000)]
    pub port: u16,

    /// Deadline applied to the service's own unary outbound gRPC calls.
    ///
    /// Deliberately per-call rather than set on the channel: `build_channel` also
    /// builds the durable consume channel, and an `Endpoint::timeout` there would
    /// tear the long-lived `ConsumeDurableEventsWithAcks` stream down on a fixed
    /// interval. Connect timeout and keepalive do not substitute — the first
    /// bounds establishing a connection, the second notices a peer that has died,
    /// and neither bounds a peer that accepted the request and stopped answering.
    #[arg(
        id = "outbound-request-timeout",
        long = "outbound-request-timeout",
        env = "OUTBOUND_REQUEST_TIMEOUT",
        default_value = "30s",
        value_parser = parse_duration_or_millis,
    )]
    pub outbound_request_timeout: Duration,

    #[arg(
        id = "grpc-port",
        long = "grpc-port",
        env = "GRPC_PORT",
        default_value = "0.0.0.0:50051",
        value_parser = parse_non_empty,
    )]
    pub grpc_port: String,

    #[arg(
        id = "enable-cors",
        long = "enable-cors",
        env = "ENABLE_CORS",
        action = ArgAction::Set,
        value_parser = parse_boolish,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub enable_cors: bool,

    /// Origins permitted when CORS is enabled, comma-separated.
    ///
    /// Empty means no origin is permitted, which makes enabling CORS without
    /// naming origins a visibly useless configuration rather than a silently
    /// permissive one. The layer previously allowed any origin and any header, so
    /// "enabled" meant "unrestricted" — a latent hazard rather than a live one,
    /// since CORS is off by default, but the wrong shape to hand an operator who
    /// turns it on.
    #[arg(
        id = "cors-allowed-origins",
        long = "cors-allowed-origins",
        env = "CORS_ALLOWED_ORIGINS",
        default_value = ""
    )]
    pub cors_allowed_origins: String,

    #[arg(
        id = "development-mode",
        long = "development-mode",
        env = "DEVELOPMENT_MODE",
        action = ArgAction::Set,
        value_parser = parse_boolish,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub development_mode: bool,
}

/// External service endpoints and queue identifiers consumed by runtime/outbound modules.
#[derive(Args, Clone, Debug)]
pub struct EndpointArgs {
    #[arg(
        id = "storage-grpc-endpoint",
        long = "storage-grpc-endpoint",
        env = "STORAGE_GRPC_ENDPOINT",
        default_value = "localhost:50001",
        value_parser = parse_plaintext_endpoint,
    )]
    pub storage_grpc_endpoint: String,

    /// Optional: only required when this binary is assigned the event
    /// consumer role (`EVENT_CONSUMER_SERVER`) and consumption isn't
    /// disabled — see `runtime::event_consumer_env_or_skip`.
    #[arg(
        id = "notifications-event-consumer-endpoint",
        long = "notifications-event-consumer-endpoint",
        env = "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
        value_parser = parse_plaintext_endpoint,
    )]
    pub notifications_event_consumer_endpoint: Option<String>,

    /// Optional: `None` skips the event-aggregation (publishing) channel.
    #[arg(
        id = "notifications-event-aggregation-endpoint",
        long = "notifications-event-aggregation-endpoint",
        env = "NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT",
        value_parser = parse_plaintext_endpoint,
    )]
    pub notifications_event_aggregation_endpoint: Option<String>,

    /// Optional: a comma-separated list of storage-container URL prefixes the
    /// durable queue filters on. Clap splits on `','` (`value_delimiter`) and
    /// each entry is trimmed/validated by `parse_non_empty` (blank entries and
    /// a wholly blank value fail at parse time). Unset resolves to `None`.
    /// De-duplication and lexicographic sort happen in
    /// [`EndpointArgs::resolve_queue_provisioning`]. The sorted unique list is
    /// also the input to StorageEventQueue's derived id.
    #[arg(
        id = "notifications-queue-resource-prefixes",
        long = "notifications-queue-resource-prefixes",
        env = "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
        value_delimiter = ',',
        value_parser = parse_non_empty,
    )]
    pub notifications_queue_resource_prefixes: Option<Vec<String>>,
}

/// Resolved durable-queue provisioning parameters for the assigned event
/// consumer. Produced by [`EndpointArgs::resolve_queue_provisioning`].
///
/// Durable-queue provisioning is always performed for the assigned consumer
/// (there is no opt-out toggle): the service ensures its queue exists on
/// startup rather than assuming an out-of-band process created it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueProvisioning {
    pub queue_id: String,
    pub resource_prefixes: Vec<String>,
}

impl EndpointArgs {
    /// Validate and resolve the durable-queue provisioning contract.
    ///
    /// Callers must only invoke this once they have established that this binary
    /// is the assigned event consumer (`runtime::resolve_consumer_assignment`)
    /// and consumption is not disabled (`disable_event_consumption`); those gates
    /// live in the runtime supervisor, not here.
    ///
    /// `notifications_queue_resource_prefixes` MUST be present — see
    /// [`required_resource_prefixes`]. StorageEventQueue's `queue_id` is
    /// derived from that canonical prefix set ([`storage_event_queue_id`]).
    pub fn resolve_queue_provisioning(&self) -> Result<QueueProvisioning, ProvisioningError> {
        let resource_prefixes =
            required_resource_prefixes(self.notifications_queue_resource_prefixes.clone())?;
        Ok(QueueProvisioning {
            queue_id: storage_event_queue_id(&resource_prefixes),
            resource_prefixes,
        })
    }
}

/// Errors resolving durable-queue provisioning from configuration, shared by
/// StorageEventQueue's and WorkerEventQueue's `resolve_queue_provisioning`.
/// A typed variant rather than a formatted string so callers (and their
/// tests) can match on what went wrong instead of parsing message text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProvisioningError {
    #[error("Missing required environment variable: NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES")]
    MissingResourcePrefixes,
}

/// De-duplicate prefixes (first-occurrence survivors) then sort lexicographically.
pub fn canonicalize_resource_prefixes(mut prefixes: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    prefixes.retain(|prefix| seen.insert(prefix.clone()));
    prefixes.sort();
    prefixes
}

/// Canonicalize `raw` and require it to be non-empty.
///
/// Shared by both StorageEventQueue and WorkerEventQueue: an empty filter
/// list does not act as an event-type-only catch-all against the real
/// Notification service — it binds a queue that receives nothing (see
/// `build_generate_filter_groups`) — so both consumers fail fast the same
/// way on a missing `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` rather than one
/// erroring and the other silently self-provisioning a dead queue.
pub fn required_resource_prefixes(
    raw: Option<Vec<String>>,
) -> Result<Vec<String>, ProvisioningError> {
    let resource_prefixes = canonicalize_resource_prefixes(raw.unwrap_or_default());
    if resource_prefixes.is_empty() {
        return Err(ProvisioningError::MissingResourcePrefixes);
    }
    Ok(resource_prefixes)
}

/// Human-recognizable prefix on every derived durable-queue id. Listing
/// durable queues on the Notification side otherwise surfaces bare sha256
/// hexdigests with nothing identifying which service they belong to
/// or which is which. A hardcoded literal, not configurable: every
/// deployment of this service — Helm, bare binary, CI, local dev — uses the
/// same prefix, so the id stays recognizable without needing a second
/// configuration value threaded through both binaries.
///
/// This prefix and the `-storage-`/`-worker-` label that follows it are what
/// keep the two queues' ids distinct, since neither digest carries a kind
/// discriminator any more. Dropping or unifying the label would make the two
/// ids equal for the same prefix set, which would put both consumers on one
/// queue — so the label is a correctness constraint, not just presentation.
pub const QUEUE_NAME_PREFIX: &str = "thumbnail-service";

/// Lowercase hex sha256 of the canonical comma-joined prefixes, shared by
/// [`storage_event_queue_id`] and [`worker_event_queue_id`].
///
/// The payload carries no queue-kind discriminator, so both queues' ids hash
/// to the same digest for the same configuration and differ only in the
/// `-storage-`/`-worker-` label their callers prepend. That is deliberate:
/// a Notification queue listing is the one place an operator sees both ids and
/// has no logs, and a shared digest is what makes "these two components agree
/// on `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`" readable there. Distinctness
/// does not depend on this payload — see the label argument on the two callers.
fn hashed_queue_id(prefixes: &[String]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let canonical = canonicalize_resource_prefixes(prefixes.to_vec());
    let payload = canonical.join(",");
    let digest = Sha256::digest(payload.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hex
}

/// StorageEventQueue id: `thumbnail-service-storage-<sha256 hex of the
/// canonical comma-joined prefixes>`.
///
/// Cannot equal [`worker_event_queue_id`] for any prefix set: the two differ in
/// a fixed-position literal segment (`-storage-` vs. `-worker-`) ahead of a
/// fixed-length digest, so the guarantee is structural and does not rest on
/// sha256 collision resistance.
pub fn storage_event_queue_id(prefixes: &[String]) -> String {
    format!("{QUEUE_NAME_PREFIX}-storage-{}", hashed_queue_id(prefixes))
}

/// WorkerEventQueue id: `thumbnail-service-worker-<sha256 hex of the canonical
/// comma-joined prefixes>`.
///
/// Shares its digest with [`storage_event_queue_id`] for the same prefix set
/// and is distinguished from it by the label alone.
pub fn worker_event_queue_id(prefixes: &[String]) -> String {
    format!("{QUEUE_NAME_PREFIX}-worker-{}", hashed_queue_id(prefixes))
}

/// Event-consumer routing and enable/disable controls used by runtime supervision logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum EventConsumerServer {
    Http,
    Grpc,
    None,
}

impl EventConsumerServer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Grpc => "GRPC",
            Self::None => "NONE",
        }
    }
}

/// Event-consumer routing and enable/disable controls used by runtime supervision logic.
#[derive(Args, Clone, Debug)]
pub struct EventConsumerArgs {
    /// Optional principal identity recorded on storage events produced by the
    /// writer of generated thumbnails, which is `task-worker`. When configured,
    /// events from this principal are generated outputs and must not recursively
    /// enqueue more thumbnails.
    ///
    /// Deliberately its own configuration value rather than a derived one: this
    /// must equal whatever storage eventually stamps on the writer's CREATE
    /// events, and no existing identity is guaranteed to match that. The check
    /// is inert until storage propagates a writer principal at all, which is why
    /// template-output shape matching is the guard actually doing the work — see
    /// `matches_rendered_output` below and ADR 0001.
    #[arg(
        id = "generated-thumbnail-principal-identity",
        long = "generated-thumbnail-principal-identity",
        env = "GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY",
        value_parser = parse_non_empty,
    )]
    pub generated_thumbnail_principal_identity: Option<String>,

    #[arg(
        id = "event-consumer-server",
        long = "event-consumer-server",
        env = "EVENT_CONSUMER_SERVER",
        value_enum,
        ignore_case = true,
        default_value_t = EventConsumerServer::Http,
    )]
    pub event_consumer_server: EventConsumerServer,

    #[arg(
        id = "disable-event-consumption",
        long = "disable-event-consumption",
        env = "DISABLE_EVENT_CONSUMPTION",
        action = ArgAction::Set,
        value_parser = parse_boolish,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub disable_event_consumption: bool,
}

/// Streaming behavior knobs consumed by notification/event-consumer processing.
#[derive(Args, Clone, Debug)]
pub struct StreamRuntimeArgs {
    #[arg(
        id = "event-processing-max-concurrent",
        long = "event-processing-max-concurrent",
        env = "EVENT_PROCESSING_MAX_CONCURRENT",
        default_value_t = 100
    )]
    pub event_processing_max_concurrent: usize,

    #[arg(
        id = "max-reconnect-backoff-seconds",
        long = "max-reconnect-backoff-seconds",
        env = "MAX_RECONNECT_BACKOFF_SECONDS",
        default_value_t = 60
    )]
    pub max_reconnect_backoff_seconds: u64,

    #[arg(
        id = "reconnect-base-backoff-seconds",
        long = "reconnect-base-backoff-seconds",
        env = "RECONNECT_BASE_BACKOFF_SECONDS",
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub reconnect_base_backoff_seconds: u64,

    #[arg(
        id = "task-cleanup-timeout-secs",
        long = "task-cleanup-timeout-secs",
        env = "TASK_CLEANUP_TIMEOUT_SECS",
        default_value_t = 30,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub task_cleanup_timeout_secs: u64,

    #[arg(
        id = "reconnect-backoff-exponent-cap",
        long = "reconnect-backoff-exponent-cap",
        env = "RECONNECT_BACKOFF_EXPONENT_CAP",
        default_value_t = 4,
        value_parser = clap::value_parser!(u32).range(0..=16),
    )]
    pub reconnect_backoff_exponent_cap: u32,

    /// Maximum number of transient (`Unavailable`/`DeadlineExceeded`/connection-style)
    /// retry attempts `ensure_durable_queue` makes before failing fast. Decoupled from
    /// the backoff-growth exponent cap so flattening backoff growth never disables
    /// provisioning retries.
    #[arg(
        id = "notifications-provisioning-max-retries",
        long = "notifications-provisioning-max-retries",
        env = "NOTIFICATIONS_PROVISIONING_MAX_RETRIES",
        default_value_t = 4,
        value_parser = clap::value_parser!(u32).range(0..),
    )]
    pub notifications_provisioning_max_retries: u32,

    #[arg(
        id = "health-warning-interval",
        long = "health-warning-interval",
        env = "HEALTH_WARNING_INTERVAL",
        default_value_t = 10,
        value_parser = clap::value_parser!(u32).range(1..),
    )]
    pub health_warning_interval: u32,

    #[arg(
        id = "stream-timeout-secs",
        long = "stream-timeout-secs",
        env = "STREAM_TIMEOUT_SECS",
        default_value_t = 90
    )]
    pub stream_timeout_secs: u64,

    #[arg(
        id = "event-task-submission-max-retry-age",
        long = "event-task-submission-max-retry-age",
        env = "EVENT_TASK_SUBMISSION_MAX_RETRY_AGE",
        default_value = "1h",
        value_parser = parse_positive_duration_or_millis,
    )]
    pub event_task_submission_max_retry_age: Duration,

    /// How long to space successive redelivery attempts of the same delivery key.
    ///
    /// Applies to both StorageEventQueue and WorkerEventQueue. Because the
    /// notifications service requeues a NACK'd delivery immediately rather than
    /// carrying a requeue delay on the wire, this delay is applied consumer-side:
    /// the NACK is deferred by this duration before being sent so the broker never
    /// holds an in-flight delivery longer than this far inside its own
    /// acknowledgement timeout (1800s observed).
    ///
    /// Defaults to 60s, matching the worker's existing recycle interval so no
    /// environment needs to change to get spacing.
    #[arg(
        id = "nack-redelivery-delay-secs",
        long = "nack-redelivery-delay-secs",
        env = "NACK_REDELIVERY_DELAY_SECS",
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub nack_redelivery_delay_secs: u64,

    /// Maximum number of deliveries that may be concurrently waiting their
    /// redelivery delay (sleeping before their NACK is sent). Once this bound is
    /// reached, further retryable failures are NACKed immediately rather than
    /// deferred, so the anti-spin gate absorbs the early redelivery, and no more
    /// deliveries await the delay at once than this many.
    ///
    /// The cap prevents the deferred set from exhausting the consumer's prefetch
    /// window the way an unbounded hold did on 2026-08-24.
    /// Zero would defer nothing, making every retryable failure an immediate
    /// NACK against a broker that requeues immediately — the hot loop the
    /// deferral exists to prevent. Rejected here as it is in `WorkerArgs`.
    #[arg(
        id = "nack-deferral-concurrency-limit",
        long = "nack-deferral-concurrency-limit",
        env = "NACK_DEFERRAL_CONCURRENCY_LIMIT",
        default_value_t = 10,
        value_parser = parse_positive_usize,
    )]
    pub nack_deferral_concurrency_limit: usize,

    /// Maximum number of consecutive early redeliveries of one delivery key
    /// before the consumer recycles the session (circuit-breaker role). An early
    /// redelivery is one that arrives before the delivery key's next-attempt time,
    /// indicating the broker is not honouring the intended spacing. Recycling
    /// achieves spacing by not being connected rather than by not consuming.
    #[arg(
        id = "nack-early-redelivery-limit",
        long = "nack-early-redelivery-limit",
        env = "NACK_EARLY_REDELIVERY_LIMIT",
        default_value_t = 5,
        value_parser = clap::value_parser!(u32).range(1..),
    )]
    pub nack_early_redelivery_limit: u32,
}

/// The same values clap's `default_value` attributes above declare.
///
/// For callers that need this struct without a command line — `task-worker`
/// passes it to `ensure_durable_queue` while keeping its own flat settings, and
/// the server's full `RuntimeConfigArgs` carries a great deal the worker has no
/// business reading. Parsing an empty argv to synthesize it would work, but clap
/// treats the environment as a value source, so it would silently pick up any
/// `STREAM_*` or `EVENT_PROCESSING_*` the pod happened to export.
///
/// `stream_runtime_defaults_match_the_clap_attributes` keeps the two in step.
impl Default for StreamRuntimeArgs {
    fn default() -> Self {
        Self {
            event_processing_max_concurrent: 100,
            max_reconnect_backoff_seconds: 60,
            reconnect_base_backoff_seconds: 5,
            task_cleanup_timeout_secs: 30,
            reconnect_backoff_exponent_cap: 4,
            notifications_provisioning_max_retries: 4,
            health_warning_interval: 10,
            stream_timeout_secs: 90,
            event_task_submission_max_retry_age: Duration::from_secs(3600),
            nack_redelivery_delay_secs: 60,
            nack_deferral_concurrency_limit: 10,
            nack_early_redelivery_limit: 5,
        }
    }
}

/// Storage gRPC channel transport tuning consumed by storage connection builders.
#[derive(Args, Clone, Debug)]
pub struct StorageChannelArgs {
    #[arg(
        id = "storage-grpc-connect-timeout",
        long = "storage-grpc-connect-timeout",
        env = "STORAGE_GRPC_CONNECT_TIMEOUT",
        default_value = "10s",
        value_parser = parse_duration_or_millis,
    )]
    pub storage_grpc_connect_timeout: Duration,

    #[arg(
        id = "storage-grpc-request-timeout",
        long = "storage-grpc-request-timeout",
        env = "STORAGE_GRPC_REQUEST_TIMEOUT",
        default_value = "30s",
        value_parser = parse_duration_or_millis,
    )]
    pub storage_grpc_request_timeout: Duration,

    #[arg(
        id = "storage-grpc-http2-keepalive-interval",
        long = "storage-grpc-http2-keepalive-interval",
        env = "STORAGE_GRPC_HTTP2_KEEPALIVE_INTERVAL",
        default_value = "30s",
        value_parser = parse_duration_or_millis,
    )]
    pub storage_grpc_http2_keepalive_interval: Duration,

    #[arg(
        id = "storage-grpc-keepalive-timeout",
        long = "storage-grpc-keepalive-timeout",
        env = "STORAGE_GRPC_KEEPALIVE_TIMEOUT",
        default_value = "10s",
        value_parser = parse_duration_or_millis,
    )]
    pub storage_grpc_keepalive_timeout: Duration,

    #[arg(
        id = "storage-grpc-tcp-keepalive",
        long = "storage-grpc-tcp-keepalive",
        env = "STORAGE_GRPC_TCP_KEEPALIVE",
        default_value = "60s",
        value_parser = parse_duration_or_millis,
    )]
    pub storage_grpc_tcp_keepalive: Duration,
}

/// Storage metadata retry policy values consumed by metadata helpers.
#[derive(Args, Clone, Debug)]
pub struct StorageRetryArgs {
    #[arg(
        id = "storage-metadata-max-retries",
        long = "storage-metadata-max-retries",
        env = "STORAGE_METADATA_MAX_RETRIES",
        default_value_t = 3,
        value_parser = clap::value_parser!(u32).range(1..),
    )]
    pub storage_metadata_max_retries: u32,

    #[arg(
        id = "storage-metadata-initial-delay",
        long = "storage-metadata-initial-delay",
        env = "STORAGE_METADATA_INITIAL_DELAY",
        default_value = "200ms",
        value_parser = parse_positive_duration_or_millis,
    )]
    pub storage_metadata_initial_delay: Duration,

    #[arg(
        id = "storage-metadata-max-delay",
        long = "storage-metadata-max-delay",
        env = "STORAGE_METADATA_MAX_DELAY",
        default_value = "2s",
        value_parser = parse_positive_duration_or_millis,
    )]
    pub storage_metadata_max_delay: Duration,
}

/// Default thumbnail specification values consumed by conversion and startup initialization.
#[derive(Args, Clone, Debug)]
pub struct ThumbnailDefaultsArgs {
    #[arg(
        id = "thumbnail-sizes",
        long = "thumbnail-sizes",
        env = "THUMBNAIL_SIZES",
        value_parser = parse_thumbnail_sizes,
    )]
    pub thumbnail_sizes: ThumbnailSizes,

    #[arg(
        id = "thumbnail-encoding-format",
        long = "thumbnail-encoding-format",
        env = "THUMBNAIL_ENCODING_FORMAT",
        value_parser = parse_uppercase_enum::<EncodingFormat>,
    )]
    pub thumbnail_encoding_format: EncodingFormat,

    #[arg(
        id = "thumbnail-transform-type",
        long = "thumbnail-transform-type",
        env = "THUMBNAIL_TRANSFORM_TYPE",
        value_parser = parse_uppercase_enum::<TransformType>,
    )]
    pub thumbnail_transform_type: TransformType,

    #[arg(
        id = "thumbnail-crop-strategy",
        long = "thumbnail-crop-strategy",
        env = "THUMBNAIL_CROP_STRATEGY",
        value_parser = parse_uppercase_enum::<CropStrategy>,
    )]
    pub thumbnail_crop_strategy: CropStrategy,

    #[arg(
        id = "thumbnail-quality",
        long = "thumbnail-quality",
        env = "THUMBNAIL_QUALITY",
        value_parser = clap::value_parser!(i32).range(1..=100),
    )]
    pub thumbnail_quality: i32,

    #[arg(
        id = "thumbnail-preserve-aspect-ratio",
        long = "thumbnail-preserve-aspect-ratio",
        env = "THUMBNAIL_PRESERVE_ASPECT_RATIO",
        action = ArgAction::Set,
        value_parser = parse_boolish,
    )]
    pub thumbnail_preserve_aspect_ratio: bool,
}

/// Operator-owned policy for deriving concrete thumbnail destinations.
#[derive(Args, Clone, Debug)]
pub struct ThumbnailPathArgs {
    #[arg(
        id = "thumbnail-path-template",
        long = "thumbnail-path-template",
        env = "THUMBNAIL_PATH_TEMPLATE"
    )]
    pub thumbnail_path_template: ThumbnailPathTemplate,
}

static RUNTIME_CONFIG: OnceLock<RuntimeConfigArgs> = OnceLock::new();

/// Install the process-wide parsed runtime config.
///
/// Fails if called more than once in the same process: each binary's `main`
/// installs config exactly once during startup, and test helpers guard
/// against repeat calls themselves (see `test_fixtures::init_test_runtime_config`,
/// which wraps this in a `std::sync::Once`).
pub fn install_runtime_config(config: RuntimeConfigArgs) -> Result<(), String> {
    RUNTIME_CONFIG
        .set(config)
        .map_err(|_| "runtime configuration already initialized".to_string())
}

/// Returns the active runtime configuration graph.
///
/// Callers must install config during startup via [`install_runtime_config`]
/// before reading values from this accessor.
pub fn current_runtime_config() -> &'static RuntimeConfigArgs {
    RUNTIME_CONFIG.get().expect(
        "runtime configuration is not installed; call install_runtime_config() during startup before reading config",
    )
}

/// Install runtime config and return the process-global snapshot.
///
/// Use this at binary startup to avoid mixing parsed `Args.runtime` values
/// with the global accessor in downstream wiring.
pub fn install_and_get_runtime_config(
    config: RuntimeConfigArgs,
) -> Result<&'static RuntimeConfigArgs, String> {
    install_runtime_config(config)?;
    Ok(current_runtime_config())
}

/// Reads a raw env value (without trimming/validation), if present.
///
/// Gated with `test_fixtures`, its only caller. The comment that used to sit here
/// explained why it could not be `#[cfg(test)]`-gated — true, and the reason the
/// `test-utils` feature exists: the fixtures call this from outside any inner
/// `#[cfg(test)]` block, so the gate has to be the same one they use.
#[cfg(any(test, feature = "test-utils"))]
pub(crate) fn raw_env_value(var_name: &str) -> Option<String> {
    std::env::var(var_name).ok()
}

/// Environment variable names read by `OutboundAuthArgs`.
pub fn outbound_auth_env_keys() -> &'static [&'static str] {
    &[
        "ENABLE_CLIENT_CREDENTIALS",
        "CLIENT_ID",
        "CLIENT_SECRET",
        "CLIENT_SECRET_FILE",
        "CLIENT_SCOPE",
        "CLIENT_CREDENTIALS_OPENID_CONFIGURATION",
        "CLIENT_CREDENTIALS_EXTRA_HEADERS",
        "CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS",
        "ENABLE_ACCESS_TOKEN_FILE",
        "ACCESS_TOKEN_FILE_PATH",
        "ACCESS_TOKEN_FILE_POLL_INTERVAL",
    ]
}

/// Parse a gRPC endpoint, refusing a TLS scheme this build cannot honor.
///
/// `tonic` is declared without a TLS feature, so nothing here can complete a TLS
/// handshake. What used to happen with `https://` was worse than a refusal: the
/// outbound builder preserved the scheme and failed to connect with an opaque
/// transport error, while `storage::build_storage_endpoint` prepended
/// unconditionally and produced `http://https://host:443` — a startup failure
/// whose message pointed nowhere near the cause.
///
/// In-cluster plaintext is the deployment model. The chart terminates TLS at the
/// ingress proxy and there is no service mesh, so every endpoint here is a
/// cluster-internal address. Refusing `https://` at parse time states that
/// decision where an operator meets it, instead of letting a reasonable-looking
/// setting fail obscurely at connect time.
///
/// Enabling a tonic `tls-*` feature and honoring the scheme is the other valid
/// answer; it is a deliberate change with a chart and a certificate story, not
/// something to infer from a URL.
pub fn parse_plaintext_endpoint(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("value must not be empty".to_string());
    }
    if trimmed.starts_with("https://") {
        return Err(format!(
            "TLS endpoints are not supported: {trimmed}. This build links tonic without a TLS \
             feature and reaches storage and notifications over plaintext HTTP/2 inside the \
             cluster; TLS is terminated at the ingress proxy. Supply a scheme-less or http:// \
             address, or enable TLS support deliberately."
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_non_empty(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

/// `clap::value_parser!(usize)` has no `.range()` — the ranged parsers are
/// integer-width specific — so a positive `usize` needs its own parser. Matches
/// `WorkerArgs::parse_positive_usize` so the same variable is valid in both.
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

fn parse_boolish(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(format!("expected true/false/1/0, got '{other}'")),
    }
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
        "invalid duration '{trimmed}': expected humantime (e.g. '250ms', '2s') or integer milliseconds"
    ))
}

fn parse_positive_duration_or_millis(value: &str) -> Result<Duration, String> {
    let duration = parse_duration_or_millis(value)?;
    if duration.is_zero() {
        return Err("duration must be greater than zero".to_string());
    }
    Ok(duration)
}

fn parse_thumbnail_sizes(value: &str) -> Result<ThumbnailSizes, String> {
    value
        .trim()
        .parse::<ThumbnailSizes>()
        .map_err(|e| e.to_string())
}

fn parse_uppercase_enum<T>(value: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    value
        .trim()
        .to_ascii_uppercase()
        .parse::<T>()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod plaintext_endpoint_tests {
    use super::parse_plaintext_endpoint;

    /// A TLS endpoint is refused at parse time with a message that names the
    /// reason.
    ///
    /// Before this, `https://` failed at connect time instead: the outbound
    /// builder preserved the scheme and reported an opaque transport error, and
    /// `build_storage_endpoint` prepended unconditionally and produced
    /// `http://https://host:443`. Both are startup failures whose message points
    /// nowhere near the setting that caused them.
    #[test]
    fn a_tls_endpoint_is_refused_with_an_explanation() {
        let error = parse_plaintext_endpoint("https://storage.example:443")
            .expect_err("this build cannot complete a TLS handshake");

        assert!(
            error.contains("TLS endpoints are not supported"),
            "the message must say what is wrong: {error}"
        );
        assert!(
            error.contains("ingress proxy"),
            "and where TLS actually terminates: {error}"
        );
    }

    #[test]
    fn scheme_less_and_plaintext_endpoints_are_accepted() {
        assert_eq!(
            parse_plaintext_endpoint("storage-service:8011").as_deref(),
            Ok("storage-service:8011"),
            "the deployed form, which the chart supplies"
        );
        assert_eq!(
            parse_plaintext_endpoint("http://localhost:50001").as_deref(),
            Ok("http://localhost:50001")
        );
        assert_eq!(
            parse_plaintext_endpoint("  storage-service:8011  ").as_deref(),
            Ok("storage-service:8011"),
            "trimming is preserved from parse_non_empty"
        );
    }

    #[test]
    fn an_empty_endpoint_is_still_refused() {
        assert!(parse_plaintext_endpoint("   ").is_err());
    }
}

/// The durable-queue id derivation, previously exercised nowhere.
///
/// Four functions here had zero test references while implementing the
/// `thumbnail-service-{storage,worker}-<sha256>` ids that `README.md` documents
/// as the operator's check that both binaries agree on
/// `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`. The failure they guard is quiet: a
/// mismatch binds a queue whose filters match nothing, and the consumer reports
/// itself healthy while receiving no work. Nothing about that surfaces in logs,
/// so a broken derivation is only visible as "thumbnails stopped happening".
#[cfg(test)]
mod queue_id_derivation_tests {
    use super::{
        canonicalize_resource_prefixes, required_resource_prefixes, storage_event_queue_id,
        worker_event_queue_id, ProvisioningError, QUEUE_NAME_PREFIX,
    };

    fn prefixes(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn canonicalization_keeps_the_first_of_each_duplicate_and_sorts() {
        assert_eq!(
            canonicalize_resource_prefixes(prefixes(&["https://b/", "https://a/", "https://b/"])),
            prefixes(&["https://a/", "https://b/"])
        );
    }

    /// The property the operator-facing check rests on: two binaries handed the
    /// same prefix set in different order, or with a duplicate, must derive the
    /// same id. Without canonicalization inside the derivation, one component's
    /// values-file ordering would silently put it on a different queue.
    #[test]
    fn queue_ids_ignore_order_and_duplicates() {
        let one = prefixes(&["https://a/", "https://b/"]);
        let other = prefixes(&["https://b/", "https://a/", "https://b/"]);

        assert_eq!(
            storage_event_queue_id(&one),
            storage_event_queue_id(&other),
            "storage id must not depend on how the prefixes were written"
        );
        assert_eq!(worker_event_queue_id(&one), worker_event_queue_id(&other));
    }

    /// Distinctness is structural — a fixed-position label ahead of a
    /// fixed-length digest — so it holds without relying on sha256 collision
    /// resistance. The shared digest is deliberate and is what makes a queue
    /// listing readable, so both halves are asserted together: if a refactor
    /// unified the labels, both consumers would land on one queue.
    #[test]
    fn the_two_ids_share_a_digest_but_never_collide() {
        for set in [
            prefixes(&[]),
            prefixes(&["https://a/"]),
            prefixes(&["https://a/", "https://b/"]),
        ] {
            let storage = storage_event_queue_id(&set);
            let worker = worker_event_queue_id(&set);

            assert_ne!(storage, worker, "the two queues must never share an id");
            assert_eq!(
                storage.rsplit('-').next(),
                worker.rsplit('-').next(),
                "but the digest is shared on purpose, for the queue listing"
            );
            assert_eq!(
                storage,
                format!("{QUEUE_NAME_PREFIX}-storage-{}", digest(&storage))
            );
            assert_eq!(
                worker,
                format!("{QUEUE_NAME_PREFIX}-worker-{}", digest(&worker))
            );
        }
    }

    fn digest(id: &str) -> &str {
        let tail = id.rsplit('-').next().expect("an id always has a tail");
        assert_eq!(tail.len(), 64, "sha256 hex is 64 characters: {id}");
        assert!(
            tail.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "lowercase hex only: {id}"
        );
        tail
    }

    /// An absent or empty prefix list is a startup failure for both consumers.
    /// It does not act as a catch-all: it binds a queue that receives nothing,
    /// which is the same silent-healthy failure the ids guard against.
    #[test]
    fn a_missing_or_empty_prefix_list_is_refused() {
        assert_eq!(
            required_resource_prefixes(None),
            Err(ProvisioningError::MissingResourcePrefixes)
        );
        assert_eq!(
            required_resource_prefixes(Some(Vec::new())),
            Err(ProvisioningError::MissingResourcePrefixes)
        );
        assert_eq!(
            required_resource_prefixes(Some(prefixes(&["https://b/", "https://a/"]))),
            Ok(prefixes(&["https://a/", "https://b/"])),
            "and a usable list comes back canonicalized"
        );
    }
}
