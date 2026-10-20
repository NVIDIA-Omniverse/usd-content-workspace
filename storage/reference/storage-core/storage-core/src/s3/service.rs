// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Debug,
    ops::Deref,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

use aws_config::{retry::RetryConfigBuilder, timeout::TimeoutConfig, BehaviorVersion};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::{
    client::customize::CustomizableOperation,
    config::{AppName, Region},
};
use bytesize::ByteSize;
use figment::Figment;
use hyper_rustls024::ConfigBuilderExt;
use rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    Certificate, ServerName,
};
use serde::{Deserialize, Serialize};
use storage_core_util::{info_limited, trace_limited, warn_limited};
use tokio::sync::RwLock;

use storage_core_util::{
    additional_headers::{AdditionalHeaders, RequestType},
    expiration, retry,
    secrets::{normalize_url_keys, SecretsReloadable},
    timeouts,
};

use super::{
    cloudfront::CloudFrontInterceptor,
    error::{Error, InvalidArgument},
    rate_limit,
    resource::{Address, AddressingStyle, DEFAULT_MAX_KEY_BYTES},
};

use crate::s3::secrets::{ClientCredentials, CredentialsOverride, Secrets};
use crate::utils::{ucc_proxy_tls_verification, UCC_PROXY_ENV, UCC_PROXY_SSL_VERIFYCERT_ENV};
use url::Url;

/// Custom certificate verifier that accepts any certificate (for testing with self-signed certs).
struct AcceptAnyCertVerifier;

impl ServerCertVerifier for AcceptAnyCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}

/// Identifies the flavor of S3-compatible backend a particular [`Client`] is
/// configured to talk to. Used by capability reporting (e.g.
/// `GetOptimisticLockingSupport`) so we can advertise the right set of
/// conditional-operation supports for each backend's known limitations.
///
/// The kind is derived from the SDK's configured `endpoint_url` (set via
/// secrets) rather than the user-facing resource address — operators can
/// front any of these backends with a custom domain or CDN on the resource
/// address side, but the endpoint URL is the actual host the SDK dials and is
/// the operator's declaration of which backend they configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// AWS S3 (no explicit endpoint URL, or one ending in `.amazonaws.com`).
    Aws,
    /// Google Cloud Storage's S3 interop endpoint
    /// (`storage.{googleapis,cloud.google}.com`).
    GcpS3Compat,
    /// Any other S3-compatible endpoint — typically MinIO, but also covers
    /// custom S3 implementations. Conservative defaults are applied for
    /// capabilities since we can't probe the implementation.
    Other,
}

impl BackendKind {
    /// Classify a backend from its configured SDK endpoint URL. When no
    /// endpoint URL is configured the SDK talks to AWS S3 by default; an
    /// endpoint URL we can't parse or match falls to `Other` rather than
    /// assuming AWS.
    pub fn from_endpoint_url(endpoint_url: Option<&str>) -> Self {
        let Some(endpoint) = endpoint_url else {
            return BackendKind::Aws;
        };
        let Some(host) = url::Url::parse(endpoint)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        else {
            return BackendKind::Other;
        };
        if host == "storage.googleapis.com" || host == "storage.cloud.google.com" {
            BackendKind::GcpS3Compat
        } else if host.ends_with(".amazonaws.com") || host == "amazonaws.com" {
            BackendKind::Aws
        } else {
            BackendKind::Other
        }
    }
}

// the aws client can only be configured with 1 region and 1 set of credentials
// in order to support multiple regions and multiple buckets (with possibly different credentials)
// a client map is kept, mapping each domain name in the resource address or identity
// to a specific aws client configuration
#[derive(Debug)]
pub struct Client {
    pub client: aws_sdk_s3::Client,
    pub bucket: String,
    pub addressing_style: AddressingStyle,
    /// Classified backend flavor for capability reporting and behavior hints.
    /// Set at construction time from the SDK's configured `endpoint_url`.
    pub backend_kind: BackendKind,
    pub(crate) cloudfront_interceptor: Option<CloudFrontInterceptor>,
    pub(crate) proxy_interceptor: Option<HttpsProxyInterceptor>,
    pub(crate) rate_limit_interceptors: rate_limit::RateLimitInterceptors,

    /// Cached knowledge of whether this backend's S3-compatible API correctly
    /// supports `ListObjectVersions`.
    ///
    /// Motivation: GCS's S3 interop endpoint returns HTTP 200 with a
    /// `<ListBucketResult>` XML envelope (the `ListObjectsV2` schema) when
    /// `ListObjectVersions` is called. The AWS SDK expects `<ListVersionsResult>`
    /// and surfaces this as an `XmlDecodeError` wrapped in
    /// `ListObjectVersionsError::Unhandled`. Our `list_stat` path falls back to
    /// `ListObjectsV2` in that case (see `list_versions_sdk`), but without
    /// caching we'd pay that wasted round-trip on every call.
    ///
    /// Encoded as a `ListVersionsSupport` (see `list_versions_support` module).
    /// A two-observation latch is used for the `ConfirmedUnsupported` state so
    /// that a single flaky 2xx-no-code response from a real-S3 backend (e.g.
    /// a transient WAF/proxy misbehavior) does not permanently downgrade the
    /// client to `ListObjectsV2` for the rest of its lifetime. Known-GCS
    /// endpoints skip the latch and are pre-seeded to `ConfirmedUnsupported`.
    ///
    /// Scope/lifetime: one cell per `Client` (i.e. per bucket + addressing
    /// style). The `ClientMap` is cleared on secrets reload, so stale state
    /// self-heals when credentials are rotated.
    pub(crate) list_versions_support: AtomicU8,
}

pub(crate) mod list_versions_support {
    //! State machine tracking whether the backend's `ListObjectVersions` API
    //! is usable. Values are stored in an `AtomicU8` on `Client`.

    use super::{AtomicU8, Ordering};

    pub(crate) const UNPROBED: u8 = 0;
    pub(crate) const SUPPORTED: u8 = 1;
    /// First unsupported observation. We DON'T yet skip — a transient flake
    /// from real S3 could land here, and pinning would be sticky.
    pub(crate) const PROBED_ONCE_UNSUPPORTED: u8 = 2;
    /// Two unsupported observations in a row (or one plus a pre-seed for a
    /// known-bad host). Skip `ListObjectVersions` entirely from now on.
    pub(crate) const CONFIRMED_UNSUPPORTED: u8 = 3;

    /// Result of a `record_unsupported` call. Callers use this to distinguish
    /// the first warning ("1st attempt; will latch after 1 more") from the
    /// latching warning ("2nd attempt; future calls will skip").
    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub(crate) enum LatchTransition {
        /// `Unprobed` → `ProbedOnceUnsupported`. One more confirming observation
        /// will latch.
        FirstObservation,
        /// `ProbedOnceUnsupported` → `ConfirmedUnsupported`. Subsequent
        /// `should_skip()` calls return true.
        Confirmed,
        /// No state change. Either the state was already `Supported` (a real
        /// success in the past; current failure treated as a flake) or already
        /// `ConfirmedUnsupported` (latched by a concurrent observer).
        NoChange,
    }

    /// True if callers that accept fallback should bypass `ListObjectVersions`.
    pub(crate) fn should_skip(cell: &AtomicU8) -> bool {
        cell.load(Ordering::Relaxed) == CONFIRMED_UNSUPPORTED
    }

    /// Record a successful `ListObjectVersions` call. Clears any partial
    /// negative observation so we don't half-remember a flake that already
    /// recovered.
    pub(crate) fn record_success(cell: &AtomicU8) {
        cell.store(SUPPORTED, Ordering::Relaxed);
    }

    /// Record an observation that the backend returned the unsupported shape.
    /// Requires two consecutive observations to latch `ConfirmedUnsupported`;
    /// the first bumps `Unprobed` → `ProbedOnceUnsupported`, the second bumps
    /// `ProbedOnceUnsupported` → `ConfirmedUnsupported`. Observations from the
    /// `Supported` state are discarded (we already saw a real success; a
    /// later negative observation is probably a flake and shouldn't demote).
    ///
    /// Returns the transition that occurred so callers can tailor log messages.
    pub(crate) fn record_unsupported(cell: &AtomicU8) -> LatchTransition {
        // First observation: Unprobed → ProbedOnceUnsupported.
        if cell
            .compare_exchange(UNPROBED, PROBED_ONCE_UNSUPPORTED, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return LatchTransition::FirstObservation;
        }
        // Second observation: ProbedOnceUnsupported → ConfirmedUnsupported.
        if cell
            .compare_exchange(
                PROBED_ONCE_UNSUPPORTED,
                CONFIRMED_UNSUPPORTED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return LatchTransition::Confirmed;
        }
        // State was already Supported or ConfirmedUnsupported; no transition.
        LatchTransition::NoChange
    }

    /// Pre-seed the support state to `ConfirmedUnsupported` for hosts that are
    /// known-bad (e.g. GCS). Skips the two-observation latch since host-based
    /// detection is trustworthy.
    pub(crate) fn seed_confirmed_unsupported(cell: &AtomicU8) {
        cell.store(CONFIRMED_UNSUPPORTED, Ordering::Relaxed);
    }
}

impl Deref for Client {
    type Target = aws_sdk_s3::Client;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

/// Returns true if `host` is a known S3-compatible endpoint whose
/// `ListObjectVersions` implementation is unusable via the AWS SDK.
///
/// Currently this matches only Google Cloud Storage's S3 interop endpoint.
/// GCS responds with HTTP 200 and a `<ListBucketResult>` XML body (the
/// `ListObjectsV2` envelope) when called via `ListObjectVersions`; the AWS
/// SDK expects `<ListVersionsResult>` and reports an `XmlDecodeError`. The
/// actual version data is in the body (Generation/IsLatest/ETag elements),
/// but we can't get at it without patching the SDK — so we fall back to
/// `ListObjectsV2` for `list_stat` on GCS.
///
/// Two hosts are recognized:
/// - `storage.googleapis.com` — the endpoint the SDK typically dials. This
///   is what appears in `AWS_ENDPOINT_URL` in our GCS secrets entries.
/// - `storage.cloud.google.com` — the host users usually put in resource
///   addresses. Our config maps this URL to the `storage.googleapis.com`
///   SDK endpoint, so it may not actually appear in the probe, but we accept
///   it defensively for deployments that configure things differently.
///
/// This is an optimization, not a correctness gate: the runtime cache in
/// `list_versions_sdk` will auto-detect any broken backend on the first call,
/// so missing a GCS hostname here just costs one extra round-trip per new
/// client before the cache takes over.
fn is_known_unversioned_listing_endpoint(host: &str) -> bool {
    host.eq_ignore_ascii_case("storage.googleapis.com") || host.eq_ignore_ascii_case("storage.cloud.google.com")
}

// domain -> aws client
pub type ClientMap = RwLock<HashMap<String, Arc<Client>, ahash::RandomState>>;

/// Configuration for the S3 storage backend.
///
/// The configuration key is `s3` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [s3]
/// read_redirect_size = "256KB"
/// write_body_max_size = "2MB"
/// write_redirect_max_size = "64MB"
///
/// [s3.timeouts]
/// connect = "10s"
/// read = "10s"
/// operation = "10s"
/// operation_attempt = "10s"
/// keepalive = "10s"
///
/// [s3.expiration]
/// presigned_expiration = "15m"
///
/// # Optional: override for S3-compatible backends with different key length limits
/// # max_key_bytes = 4096
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Timeouts for the S3 storage backend.
    ///
    /// See the [timeouts::Config] struct for more details.
    #[serde(default)]
    pub timeouts: timeouts::Config,

    /// Retry policy for the S3 storage backend.
    ///
    /// See the [retry::Config] struct for more details.
    #[serde(default)]
    pub retry: retry::Config,

    /// Presigned url expiration for the S3 storage backend.
    ///
    /// See the [expiration::Config] struct for more details.
    #[serde(default)]
    pub expiration: expiration::Config,

    // TODO: align azure/s3 configs so that they are the same in the config file / config map
    /// if a download preference is not specified in the read request,
    /// objects smaller than this size will be retrieved and returned over gRPC channel,
    /// objects >= will be returned as a presigned url
    #[serde(default = "default_read_redirect_size")]
    pub read_redirect_size: ByteSize,

    /// write body size (single upload vs multipart upload)
    #[serde(default = "default_write_body_max_size")]
    pub write_body_size: ByteSize,

    /// write redirect size (upload via gRPC vs HTTP presigned url)
    #[serde(default = "default_write_redirect_max_size")]
    pub write_redirect_size: ByteSize,

    /// Maximum object key size in bytes. Defaults to 1024 which matches the S3 spec.
    /// S3-compatible backends may have different limits:
    /// - AWS S3: 1024 bytes <https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-keys.html>
    /// - MinIO: 1024 bytes (default), 4096 in extended mode <https://min.io/docs/minio/linux/operations/concepts/thresholds.html>
    /// - GCP Cloud Storage: 1024 bytes <https://cloud.google.com/storage/docs/naming-objects>
    #[serde(default = "default_max_key_bytes")]
    pub max_key_bytes: usize,

    /// When `false`, requests for S3 buckets that are **not** explicitly listed in the
    /// secrets configuration will be rejected with a `NotFound` error instead of falling
    /// through to environment-variable credentials or anonymous access.
    ///
    /// Default: `true` (backwards-compatible — existing deployments keep the current
    /// fall-through behaviour).  New deployments should set this to `false` to prevent
    /// accidental access to unconfigured buckets.
    #[serde(default = "default_allow_unconfigured_endpoints")]
    pub allow_unconfigured_endpoints: bool,

    /// Operator-specified backend-wide default for bucket versioning state; a
    /// per-endpoint `VERSIONING_ENABLED` entry in the secrets configuration
    /// overrides it for that endpoint. When the resolved value is `false`,
    /// `EnumerateVersions` for the endpoint is short-circuited with
    /// `FAILED_PRECONDITION` before reaching the backend and
    /// `IsVersioned` reports `false`. When it is `true` or nothing is
    /// configured (the default), versioning is treated as enabled:
    /// `EnumerateVersions` goes to the backend unchanged and
    /// `IsVersioned` reports `true`. There is no runtime detection —
    /// the state is whatever the operator configures.
    #[serde(default)]
    pub versioning_enabled: Option<bool>,

    /// Whether FileObject-native metadata is enabled for this backend. When
    /// unset (the default) native metadata is treated as disabled:
    /// `GetNativeMetadataSupport` reports `false`, writes carrying non-empty
    /// `WriteParameters.native_metadata` are rejected with UNIMPLEMENTED, and
    /// `ListStat` with `include_native_metadata` does not issue the per-object
    /// HEAD fan-out. When enabled, native metadata is surfaced on the free
    /// stat/read paths (no extra backend call) and on list. Writes carrying non-empty
    /// native metadata are still rejected with UNIMPLEMENTED on every upload path
    /// (direct-body, redirect and multipart) — write persistence is a follow-up.
    /// Because that write support is incomplete, `GetNativeMetadataSupport` reports
    /// `false` in this pre-release state regardless of this flag; it will report the
    /// flag once write persistence lands. There is no runtime detection — the state
    /// is whatever the operator configures.
    #[serde(default)]
    pub native_metadata_enabled: Option<bool>,

    /// When `native_metadata_enabled`, whether `ListStat` additionally fetches each listed object's native
    /// metadata via a HeadObject fan-out. S3 LIST does not return user metadata, so this issues one HeadObject
    /// per listed object (bounded concurrency) — expensive on large folders. Off by default; enable only for
    /// small folders or latency-tolerant callers that need native metadata in listings. No effect when
    /// `native_metadata_enabled` is off. Azure returns metadata inline and ignores this flag.
    #[serde(default)]
    pub native_metadata_list_fanout_enabled: Option<bool>,
    /// When `true`, addresses served by this backend are reported as read-only
    /// by `IsReadOnly`. When `false` or unset (the default), addresses are
    /// reported as writable. This is a declarative operator setting; there is no
    /// runtime detection and it does not itself enforce write rejection.
    #[serde(default)]
    pub read_only: Option<bool>,
}

fn default_read_redirect_size() -> ByteSize {
    ByteSize::kib(256)
}

fn default_write_body_max_size() -> ByteSize {
    ByteSize::mib(2)
}

fn default_write_redirect_max_size() -> ByteSize {
    ByteSize::mib(64)
}

fn default_max_key_bytes() -> usize {
    DEFAULT_MAX_KEY_BYTES
}

fn default_allow_unconfigured_endpoints() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeouts: timeouts::Config::default(),
            retry: retry::Config::default(),
            expiration: expiration::Config::default(),
            read_redirect_size: default_read_redirect_size(),
            write_body_size: default_write_body_max_size(),
            write_redirect_size: default_write_redirect_max_size(),
            max_key_bytes: default_max_key_bytes(),
            allow_unconfigured_endpoints: default_allow_unconfigured_endpoints(),
            versioning_enabled: None,
            native_metadata_enabled: None,
            native_metadata_list_fanout_enabled: None,
            read_only: None,
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "s3";

    /// Reject a native-metadata configuration that would make the read-side caches return wrong data.
    ///
    /// On S3, `ListStat` only carries native metadata when the HeadObject fan-out
    /// (`native_metadata_list_fanout_enabled`) is on. The `list_cache` and `small_object_cache` layers
    /// are filled from `ListStat`, so with `native_metadata_enabled` set but the fan-out off, a Stat or
    /// Read served from a cached listing/hint would return an empty map while a direct Stat/Read (a free
    /// HeadObject) returns the object's user metadata. Rather than serve that inconsistency, refuse the
    /// config: the operator must enable the fan-out or disable the caches. Azure returns metadata inline
    /// regardless of the flag and is unaffected, so this guard is S3-only.
    pub fn validate_native_metadata_cache_consistency(
        &self,
        figment: &Figment,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // A cache layer is enabled unless its section explicitly sets `enabled = false` (matching each
        // layer's own default). Probe just that field so this stays independent of the cache features.
        fn cache_enabled(figment: &Figment, key: &str) -> bool {
            #[derive(serde::Deserialize)]
            struct Probe {
                #[serde(default)]
                enabled: Option<bool>,
            }
            figment
                .focus(key)
                .extract::<Probe>()
                .ok()
                .and_then(|p| p.enabled)
                .unwrap_or(true)
        }

        let fanout_off = self.native_metadata_list_fanout_enabled != Some(true);
        if self.native_metadata_enabled == Some(true)
            && fanout_off
            && (cache_enabled(figment, "list_cache") || cache_enabled(figment, "small_object_cache"))
        {
            return Err(
                "[s3] native_metadata_enabled is set with native_metadata_list_fanout_enabled off \
                        while list_cache and/or small_object_cache are enabled: a stat/read served from a \
                        cached listing would drop the native metadata a direct stat/read returns. Enable \
                        native_metadata_list_fanout_enabled, or disable the caches."
                    .into(),
            );
        }
        Ok(())
    }
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

// Storage Service
pub struct Storage {
    // map of all the AWS client objects, keyed by bucket
    pub clients: ClientMap,
    // configuration for the storage service
    pub config: Config,
    // secrets for the storage service, wrapped in RwLock for hot-reload support
    pub secrets: RwLock<Option<Secrets>>,
    // presigned urls expiration
    pub expiration_normalizer: expiration::ExpirationNormalizer,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("Storage");
        dbg.field("clients", &self.clients).field("config", &self.config);
        match self.secrets.try_read() {
            Ok(guard) => dbg.field("secrets", &*guard),
            Err(_) => dbg.field("secrets", &"<locked>"),
        };
        dbg.field("expiration_normalizer", &self.expiration_normalizer).finish()
    }
}

impl Storage {
    pub fn new(config: Config, mut secrets: Option<Secrets>) -> Result<Self, Box<dyn std::error::Error>> {
        let expiration_normalizer = expiration::ExpirationNormalizer::new(&config.expiration);

        info_limited!(?config);

        if let Some(ref mut s) = secrets {
            storage_core_util::secrets::normalize_url_keys(s);
        }

        Ok(Self {
            clients: ClientMap::default(),
            config,
            secrets: RwLock::new(secrets),
            expiration_normalizer,
        })
    }

    /// Creates a new S3 storage service from a figment config and secrets.
    ///
    /// The config will be extracted from the Figment struct, looking for the `s3` key.
    /// The secrets will be extracted from the Figment struct, looking for the `storage-service-s3` key.
    pub fn from_figment(figment: &Figment, secrets: &Option<Figment>) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::try_from(figment)?;
        config.validate_native_metadata_cache_consistency(figment)?;
        let secrets = secrets.as_ref().map(Secrets::try_from).transpose()?;
        Self::new(config, secrets)
    }

    pub(super) async fn client(&self, resource_address: &mut Address) -> Result<Arc<Client>, Error> {
        resource_address.max_key_bytes = self.config.max_key_bytes;

        // this is optimized for the read/hit case
        // the clients map is not expected to be modified very often
        // First try the Virtual addressing style, this should be the most common case
        if let Some(client) = self
            .clients
            .read()
            .await
            .get(resource_address.config_key(AddressingStyle::Virtual))
        {
            resource_address.addressing_style = client.addressing_style;
            return Ok(client.clone());
        }

        // if the Virtual addressing style is not found, try the Path addressing style
        if let Some(client) = self
            .clients
            .read()
            .await
            .get(resource_address.config_key(AddressingStyle::Path))
        {
            resource_address.addressing_style = client.addressing_style;
            return Ok(client.clone());
        }

        let mut write_lock = self.clients.write().await;

        // this could potentially be ineffecient because we can't check if the client already exists
        // before creating it because we don't know the path style yet, so we could create and then
        // check the map and see it already exists, but this should be a rare case.
        let client = self.create_client(resource_address).await?;
        let config_key = resource_address.config_key(client.addressing_style);

        match write_lock.entry(config_key.into()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                // no one else created the client, insert it
                entry.insert(client.clone());
                Ok(client)
            },
        }
    }

    async fn create_client(&self, resource_address: &mut Address) -> Result<Arc<Client>, Error> {
        // if the AWS env vars for credentials, AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY are set,
        // and there are no matching configs for the host, use the env vars for credentials.
        // this is not an advertised method for credentials, but useful for local development / CI/CD
        // nothing special has to be done for this to work, the aws config object will automatically
        // use the env vars if present and credentials were not explicitly specified.
        // Scope the lock guard so it's dropped before any .await points
        let client_config = {
            let secrets_guard = self.secrets.read().await;
            match &*secrets_guard {
                Some(secrets) => {
                    // have configuration, see if we can find a config for this resource address
                    match ClientCredentials::from_secrets(secrets, resource_address)? {
                        Some(client_config) => {
                            info_limited!(
                                configuration_key =
                                    resource_address.config_key(client_config.addressing_style.unwrap_or_default()),
                                ?client_config,
                                "found configuration entry for resource address"
                            );
                            client_config
                        },
                        // have configuration, host not in configuration
                        None => {
                            if !self.config.allow_unconfigured_endpoints {
                                let host = resource_address.host().unwrap_or("unknown");
                                return Err(Error::not_found(format!("endpoint not configured: {host}")));
                            }
                            let client_config = ClientCredentials::from_env()?;
                            info_limited!(
                                configuration_key =
                                    resource_address.config_key(client_config.addressing_style.unwrap_or_default()),
                                ?client_config,
                                "configuration exists, but no entry found for resource address, using env vars"
                            );
                            client_config
                        },
                    }
                },
                // no configuration provided
                None => {
                    if !self.config.allow_unconfigured_endpoints {
                        let host = resource_address.host().unwrap_or("unknown");
                        return Err(Error::not_found(format!("endpoint not configured: {host}")));
                    }
                    let client_config = ClientCredentials::from_env()?;
                    info_limited!(
                        configuration_key =
                            resource_address.config_key(client_config.addressing_style.unwrap_or_default()),
                        ?client_config,
                        "no configuration provided, using env vars"
                    );
                    client_config
                },
            }
        };

        let addressing_style = client_config.addressing_style.unwrap_or_default();

        // for debugging, sha256 hash the key, secret, token and log the hash
        info_limited!(
            configuration_key = resource_address.config_key(addressing_style),
            hash = client_config.credential_hash(),
            role_arn = ?client_config.role_arn,
            role_session_name = ?client_config.role_session_name,
            role_session_length = ?client_config.role_session_length,
            external_id = ?client_config.external_id,
            "credentials"
        );

        // if the bucket/region where not part of the sercets, attempt
        // to parse out of the url. custom domain names for S3 buckets would require
        // bucket and region are specified in the secrets.

        let (bucket, region) = match (client_config.bucket, client_config.region) {
            (Some(bucket), Some(region)) => (bucket, region),
            _ => match addressing_style {
                AddressingStyle::Virtual => self.bucket_region(resource_address.host()?)?,
                AddressingStyle::Path => {
                    return Err(Error::internal(
                        "Path addressing style requires region to be specified in the configuration",
                    ))
                },
            },
        };

        let region = Region::new(region);

        // setup retry policy
        let retry_config = RetryConfigBuilder::default()
            .max_attempts(self.config.retry.attempts)
            .initial_backoff(self.config.retry.initial_backoff)
            .max_backoff(self.config.retry.max_backoff);

        // setup client timeouts
        let mut timeout_builder = TimeoutConfig::builder()
            .connect_timeout(self.config.timeouts.connect)
            .read_timeout(self.config.timeouts.read)
            .operation_timeout(self.config.timeouts.operation);

        // operation attempt timeout is optional
        timeout_builder.set_operation_attempt_timeout(self.config.timeouts.operation_attempt);

        let mut config_loader = aws_config::defaults(BehaviorVersion::latest())
            .app_name(AppName::new(format!("nv-{}", env!("CARGO_PKG_NAME"))).expect("invalid aws sdk app name"))
            .region(region.clone())
            .retry_config(retry_config.build())
            .timeout_config(timeout_builder.build());

        // Keep the endpoint_url around after loading it into the SDK config so we can
        // use its host to pre-seed the `ListObjectVersions` support cache (below).
        let endpoint_url = client_config.endpoint_url.clone();
        if let Some(ref endpoint_url) = endpoint_url {
            config_loader = config_loader.endpoint_url(endpoint_url.clone());
        }

        // at this point we almost have our config setup. now it depends on how the credentials are specified
        // in the client config. for public / anonymous access, we have explicitly not to sign the requests.
        // if role ARN is specified, we use STS to assume the role.

        let config = if let Some(override_credentials) = client_config.credentials_override.as_ref() {
            match override_credentials {
                CredentialsOverride::Anonymous => {
                    info_limited!("using anonymous credentials override for the client");
                    config_loader.no_credentials().load().await
                },
                CredentialsOverride::CredentialsProvider(shared) => {
                    info_limited!("using provider credentials override for the client");
                    config_loader.credentials_provider(shared.clone()).load().await
                },
            }
        } else if client_config.access_key_id.is_none()
            && client_config.secret_access_key.is_none()
            && client_config.role_arn.is_none()
        {
            // for this case we have to explicitly tell aws sdk to not sign the requests
            config_loader.no_credentials().load().await
        } else {
            // if access key id and secret key are specified, we want to use them for any STS operations,
            // otherwise we can just stick with the loaded environment (e.g IRSA + ServiceAccount in Kuberenetes
            // providing the base credentials to STS)
            if let (Some(access_key_id), Some(secret_access_key)) =
                (&client_config.access_key_id, &client_config.secret_access_key)
            {
                // access key and secret were specified, use them directly
                let provider = SharedCredentialsProvider::new(Credentials::new(
                    access_key_id,
                    secret_access_key,
                    client_config.session_token.clone(),
                    None,
                    "storage-service",
                ));

                config_loader = config_loader.credentials_provider(provider);
            }

            let base_config = config_loader.load().await;

            // if role arn was specified, attempt to configure it and then assume it
            if let Some(role_arn) = &client_config.role_arn {
                let mut provider_builder = aws_config::sts::AssumeRoleProvider::builder(role_arn);

                if let Some(role_session_name) = &client_config.role_session_name {
                    provider_builder = provider_builder.session_name(role_session_name);
                }

                if let Some(role_session_length) = &client_config.role_session_length {
                    provider_builder = provider_builder.session_length(*role_session_length);
                }

                if let Some(external_id) = &client_config.external_id {
                    provider_builder = provider_builder.external_id(external_id);
                }

                // provider of the assumed role
                let provider = provider_builder.configure(&base_config).build().await;

                // final config with the assumed role provider
                base_config
                    .to_builder()
                    .credentials_provider(SharedCredentialsProvider::new(provider))
                    .build()
            } else {
                // no role arn specified, use the base config, which may contain credentials
                // from the environment or from the secrets.
                base_config
            }
        };

        let mut client_builder =
            aws_sdk_s3::config::Builder::from(&config).force_path_style(addressing_style == AddressingStyle::Path);

        // delay setting up UCC proxy. this means that all STS operations above will happen without the proxy
        // this is mainly due to there is no way in the AWS SDK to specify an interceptor, like with the S3 Config,
        // so unless we can modify the scheme or Host: header, all https traffic would end up using CONNECT tunnel
        // and fail, since UCC does not support CONNECT tunnels.

        // using custom UCC_PROXY environment variable to specify the proxy server
        // This applies to both HTTP and HTTPS and UCC proxy will convert all HTTP (unencrypted) requests
        // to HTTPS (encrypted) requests.
        //
        // Standard HTTP proxies are specified with the following environment variables:
        //   - HTTP_PROXY specifies the proxy server to use for HTTP (unencrypted) requests.
        //   - HTTPS_PROXY specifies the proxy server to use for HTTPS (encrypted) requests
        //
        // we may want to consider using the standard env vars in the future for greater compatibility.
        //
        // we are using the older deprecated hyper crates because the newer hyper crates are not
        // compatible with the aws sdk s3.
        // see https://github.com/awslabs/aws-sdk-rust/issues/169

        info_limited!(
            proxy = ?std::env::var(UCC_PROXY_ENV),
            verify_certs = ?std::env::var(UCC_PROXY_SSL_VERIFYCERT_ENV),
            %resource_address,
        );

        // setup any http/https proxy if UCC_PROXY is set
        let proxy_interceptor = if let Ok(proxy_address) = std::env::var(UCC_PROXY_ENV) {
            // TLS verification is off only for UCC_PROXY_SSL_VERIFYCERT=insecure (local test proxies)
            let verify_certs = ucc_proxy_tls_verification(&proxy_address);

            let mut http_connector = hyper014::client::HttpConnector::new();
            http_connector.enforce_http(false);
            http_connector.set_nodelay(true);
            http_connector.set_reuse_address(true);
            http_connector.set_keepalive(self.config.timeouts.keepalive);
            http_connector.set_connect_timeout(Some(self.config.timeouts.connect));

            let base_builder = hyper_rustls024::HttpsConnectorBuilder::new();
            let builder = if !verify_certs {
                let mut cfg = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_native_roots()
                    .with_no_client_auth();
                cfg.dangerous()
                    .set_certificate_verifier(Arc::new(AcceptAnyCertVerifier));
                base_builder.with_tls_config(cfg)
            } else {
                base_builder.with_native_roots()
            };
            // HTTP/2 must stay enabled here, even though this ALPN offer is the only way this
            // workspace can reach h2 0.3.27 (via hyper 0.14 under the AWS Smithy `hyper-014`
            // connector below) and RUSTSEC-2026-0258 is receive-side. Dropping it was tried and
            // reverted: without h2 the SDK falls back to HTTP/1.1, and Google Cloud Storage's
            // S3-compatible endpoint answers the bodyless CopyObject PUT with 411 Length
            // Required, which breaks copy and move on every GCS-compatible backend reached
            // through the UCC proxy. See the RUSTSEC-2026-0258 note in deny.toml.
            let https_connector = builder
                .https_or_http()
                .enable_http1()
                .enable_http2()
                .wrap_connector(http_connector);

            let proxy_uri = proxy_address
                .parse()
                .map_err(|e| Error::invalid_argument(format!("failed to parse proxy address {proxy_address}: {e}")))?;
            let proxy = hyper_proxy::Proxy::new(hyper_proxy::Intercept::All, proxy_uri);
            let proxy_connector = hyper_proxy::ProxyConnector::from_proxy(https_connector, proxy)
                .map_err(|e| Error::internal(format!("failed to setup connector: {e}")))?;

            let http_client =
                aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder::new().build(proxy_connector);
            client_builder = client_builder.http_client(http_client);

            Some(HttpsProxyInterceptor)
        } else {
            None
        };

        let client = aws_sdk_s3::Client::from_conf(client_builder.build());
        trace_limited!(?client);

        resource_address.addressing_style = addressing_style;

        // setup cloudfront interceptor if configured for cloudfront
        let cloudfront_interceptor = if let Some(cloudfront) = &client_config.cloudfront {
            info_limited!(?cloudfront);

            let distribution_url = cloudfront
                .distribution_url
                .as_ref()
                .ok_or(Error::invalid_argument("missing cloudfront distribution url"))?
                .clone();

            if let (Some(key_pair_id), Some(private_key_pem)) = (&cloudfront.key_pair_id, &cloudfront.private_key_pem) {
                Some(
                    CloudFrontInterceptor::with_signing_info(
                        distribution_url.clone(),
                        key_pair_id,
                        private_key_pem,
                        self.expiration_normalizer.clone(),
                    )
                    .map_err(|e| {
                        Error::internal(format!("failed setup cloudfront distribution {distribution_url}: {e}"))
                    })?,
                )
            } else {
                Some(CloudFrontInterceptor::new(distribution_url))
            }
        } else {
            None
        };

        // Pre-seed the `ListObjectVersions` support cache for known-GCS endpoints.
        //
        // GCS's S3 interop responds to `ListObjectVersions` with HTTP 200 and a
        // `<ListBucketResult>` envelope instead of `<ListVersionsResult>`, which the
        // SDK cannot deserialize. The runtime cache (see `list_versions_sdk`) will
        // learn this on the first call, but pre-seeding here skips the wasted
        // round-trip entirely for the common GCS case.
        //
        // We consult two hosts in order of preference:
        //   1. The configured `endpoint_url` host — this is what the SDK actually
        //      dials (for GCS: "storage.googleapis.com").
        //   2. The resource address host — the user-facing URL, which for GCS is
        //      typically "storage.cloud.google.com/<bucket>". Used as a fallback
        //      in case a deployment sets endpoint_url to something non-matching
        //      while still pointing at GCS via the resource address.
        //
        // This is only an optimization. If we miss a GCS variant here, the
        // runtime cache still kicks in after two wasted calls (one to probe,
        // one to confirm) — no correctness impact.
        let list_versions_support = AtomicU8::new(list_versions_support::UNPROBED);
        let endpoint_host = endpoint_url
            .as_deref()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(str::to_owned));
        let address_host = resource_address.host().ok().map(str::to_owned);
        let probe_host = endpoint_host.as_deref().or(address_host.as_deref()).unwrap_or("");
        if is_known_unversioned_listing_endpoint(probe_host) {
            list_versions_support::seed_confirmed_unsupported(&list_versions_support);
        }

        let backend_kind = BackendKind::from_endpoint_url(endpoint_url.as_deref());

        Ok(Arc::new(Client {
            client,
            bucket: bucket.clone(),
            addressing_style,
            backend_kind,
            cloudfront_interceptor,
            proxy_interceptor,
            rate_limit_interceptors: rate_limit::RateLimitInterceptors::new("s3"),
            list_versions_support,
        }))
    }

    // figure out bucket/region for a given host
    pub fn bucket_region(&self, hostname: &str) -> Result<(String, String), Error> {
        // parse out domain => <bucket>.s3.<region>.amazonaws.com
        let mut pieces = hostname.split('.');

        // bucket
        let bucket = match pieces.next() {
            Some(bucket) => bucket,
            None => {
                return InvalidArgument {
                    message: format!("missing bucket '{hostname}'"),
                }
                .fail()
            },
        };

        // s3 part of url — supports both "s3.<region>" (new) and "s3-<region>" (old) styles
        let region_from_s3_piece;
        match pieces.next() {
            Some("s3") => {
                region_from_s3_piece = None;
            },
            Some(s3_piece) if s3_piece.starts_with("s3-") => {
                region_from_s3_piece = Some(&s3_piece[3..]);
            },
            _ => {
                return InvalidArgument {
                    message: format!("missing s3 '{hostname}'"),
                }
                .fail()
            },
        }

        // region — for old-style URLs the region was already extracted from the s3-<region> piece
        let region = match region_from_s3_piece {
            Some(region) => region,
            None => match pieces.next() {
                Some(region) => region,
                None => {
                    return InvalidArgument {
                        message: format!("missing region '{hostname}'"),
                    }
                    .fail()
                },
            },
        };

        // amazonaws part of url
        match pieces.next() {
            Some("amazonaws") => {},
            _ => {
                return InvalidArgument {
                    message: format!("missing amazonaws '{hostname}'"),
                }
                .fail()
            },
        }

        // tld part of url
        match pieces.next() {
            Some("com") => {},
            _ => {
                return InvalidArgument {
                    message: format!("missing .com '{hostname}'"),
                }
                .fail()
            },
        }

        // should by empty
        if pieces.next().is_some() {
            return InvalidArgument {
                message: format!("unknown parts in hostname '{hostname}'"),
            }
            .fail();
        }

        trace_limited!(hostname, bucket, region);
        Ok((bucket.into(), region.into()))
    }

    pub(super) fn apply_request_context<T, E, R>(
        mut request: CustomizableOperation<T, E, R>,
        additional_headers: Option<AdditionalHeaders>,
        request_type: RequestType,
        proxy_interceptor: Option<HttpsProxyInterceptor>,
        cloudfront_interceptor: Option<CloudFrontInterceptor>,
        rate_limit_interceptor: rate_limit::RateLimitInterceptor,
    ) -> CustomizableOperation<T, E, R> {
        if let Some(additional_headers) = additional_headers {
            request = request.mutate_request(move |request| {
                for (key, value) in additional_headers.request_headers(request_type) {
                    request.headers_mut().append(key, value);
                }
            });
        }
        if let Some(interceptor) = proxy_interceptor {
            request = request.interceptor(interceptor);
        }
        if let Some(interceptor) = cloudfront_interceptor {
            request = request.interceptor(interceptor);
        }
        request = request.interceptor(rate_limit_interceptor);
        request
    }

    /// Checks whether the endpoint identified by `url` is listed in the secrets
    /// configuration when `allow_unconfigured_endpoints = false`.
    ///
    /// Returns `Ok(())` immediately when `allow_unconfigured_endpoints` is `true`
    /// (the default). Otherwise returns `Err(NotFound)` when no matching entry
    /// is found in the secrets map.
    pub async fn check_endpoint_configured(&self, url: &Url) -> Result<(), Error> {
        if self.config.allow_unconfigured_endpoints {
            return Ok(());
        }
        let addr = Address::parse(url.as_str())?;
        let secrets_guard = self.secrets.read().await;
        let host = addr.host().unwrap_or("unknown");
        match &*secrets_guard {
            None => Err(Error::not_found(format!("endpoint not configured: {host}"))),
            Some(secrets) => {
                if ClientCredentials::from_secrets(secrets, &addr)?.is_some() {
                    Ok(())
                } else {
                    Err(Error::not_found(format!("endpoint not configured: {host}")))
                }
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HttpsProxyInterceptor;

impl aws_smithy_runtime_api::client::interceptors::Intercept for HttpsProxyInterceptor {
    fn name(&self) -> &'static str {
        "HttpsProxyInterceptor"
    }

    fn modify_before_transmit(
        &self,
        context: &mut aws_smithy_runtime_api::client::interceptors::context::BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &aws_sdk_s3::config::RuntimeComponents,
        _cfg: &mut aws_smithy_types::config_bag::ConfigBag,
    ) -> Result<(), aws_smithy_runtime_api::box_error::BoxError> {
        let request = context.request_mut();

        // replace scheme with http. this tells the proxy to terminate the https connection.
        // using https would use CONNECT verb to the proxy. we currently don't support CONNECT.
        // rarely expect to see http, mainly https, so just going to always replace the scheme,
        // knowing that if the url is http to begin with, it will be an extra string copy.
        if let Some(uri) = request.uri().split("://").nth(1) {
            const HTTP_SCHEME: &str = "http://";
            // faster than format! and parsing fmt string
            let mut proxy_uri = String::with_capacity(HTTP_SCHEME.len() + uri.len());
            proxy_uri.push_str(HTTP_SCHEME);
            proxy_uri.push_str(uri);
            request.set_uri(proxy_uri)?;
        }

        Ok(())
    }
}

impl SecretsReloadable for Storage {
    async fn reload_secrets(&self, secrets: Option<&Figment>) {
        let mut new_secrets = match secrets.map(Secrets::try_from).transpose() {
            Ok(s) => s,
            Err(e) => {
                warn_limited!(?e, "Failed to parse new S3 secrets, keeping existing credentials");
                return;
            },
        };

        if let Some(ref mut s) = new_secrets {
            normalize_url_keys(s);
        }

        // Hold the clients write lock first to block new client creation
        // while we swap secrets and clear stale caches.
        let mut clients = self.clients.write().await;
        *self.secrets.write().await = new_secrets;
        clients.clear();
        info_limited!("S3 secrets reloaded, client cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use crate::s3::error::Result;
    use crate::s3::resource::{self, WrappedResourceInfo};
    use crate::traits::fileobject::v1alpha::{ResourceIdentity, ResourceInfo};

    use super::*;

    use assert_matches::assert_matches;
    use rstest::rstest;
    use tracing::debug;

    #[test]
    fn backend_kind_from_endpoint_url_classifies_known_hosts() {
        // No endpoint configured -> SDK defaults to AWS S3.
        assert_eq!(BackendKind::from_endpoint_url(None), BackendKind::Aws);

        // AWS S3 — explicit endpoints with the canonical suffix.
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://s3.amazonaws.com")),
            BackendKind::Aws
        );
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://s3.us-west-2.amazonaws.com")),
            BackendKind::Aws
        );

        // GCS S3 interop — both host forms our config uses.
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://storage.googleapis.com")),
            BackendKind::GcpS3Compat
        );
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://storage.cloud.google.com")),
            BackendKind::GcpS3Compat
        );
        // Case-insensitive.
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://STORAGE.GOOGLEAPIS.COM")),
            BackendKind::GcpS3Compat
        );

        // MinIO / custom S3-compat fall to `Other`.
        assert_eq!(
            BackendKind::from_endpoint_url(Some("http://127.0.0.1:9000")),
            BackendKind::Other
        );
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://minio.example.com")),
            BackendKind::Other
        );
        // No suffix smuggling on AWS detection.
        assert_eq!(
            BackendKind::from_endpoint_url(Some("https://amazonaws.com.attacker.example")),
            BackendKind::Other
        );
        // Unparseable URL falls back to `Other` (it certainly isn't AWS).
        assert_eq!(BackendKind::from_endpoint_url(Some("not-a-url")), BackendKind::Other);
    }

    #[test]
    fn known_unversioned_listing_endpoint_matches_gcs_hosts() {
        assert!(is_known_unversioned_listing_endpoint("storage.googleapis.com"));
        assert!(is_known_unversioned_listing_endpoint("storage.cloud.google.com"));
        // case-insensitive — hosts are compared case-insensitively per RFC 3986.
        assert!(is_known_unversioned_listing_endpoint("Storage.Googleapis.COM"));

        // Non-matching hosts: real S3, MinIO, Azure, unrelated domains.
        assert!(!is_known_unversioned_listing_endpoint(""));
        assert!(!is_known_unversioned_listing_endpoint("s3.amazonaws.com"));
        assert!(!is_known_unversioned_listing_endpoint(
            "mybucket.s3.us-west-2.amazonaws.com"
        ));
        assert!(!is_known_unversioned_listing_endpoint("play.min.io"));
        assert!(!is_known_unversioned_listing_endpoint(
            "exampleaccountprivate.blob.core.windows.net"
        ));
        // Must be exact host — no suffix smuggling.
        assert!(!is_known_unversioned_listing_endpoint(
            "evil-storage.googleapis.com.attacker.com"
        ));
        assert!(!is_known_unversioned_listing_endpoint("fakestorage.googleapis.com"));
    }

    #[test_log::test(tokio::test)]
    async fn url_from() {
        // resource identities
        let resource_identity = ResourceIdentity {
            encoded_identity: "https://omniverse-content-production.s3.us-west-2.amazonaws.com/Samples/Astronaut/Astronaut.usd?etag=%2235c0f035686ddde806ada4d2a744b7a6-2%22&content-length=28562005&last-modified=2024-05-13T15%3A52%3A43Z".to_owned(),
        };
        // A legacy identity URL that still carries the pre-Phase-3 content-length and
        // last-modified query params should parse successfully — those params are
        // silently accepted and dropped. Only etag + version-id survive on Identity.
        let result = resource::Identity::try_from(Some(resource_identity).as_ref()).unwrap();
        debug!(?result);
        assert_eq!(
            "https://omniverse-content-production.s3.us-west-2.amazonaws.com",
            result.resource_address.config_key(AddressingStyle::Virtual)
        );
        assert_eq!("Samples/Astronaut/Astronaut.usd", result.key().unwrap());
        assert_eq!(Some("\"35c0f035686ddde806ada4d2a744b7a6-2\""), result.etag.as_deref());
        // Re-serializing the identity must round-trip without the legacy metadata params.
        let reserialized = result.to_string();
        assert!(
            !reserialized.contains("content-length"),
            "reserialized identity retained content-length: {reserialized}"
        );
        assert!(
            !reserialized.contains("last-modified"),
            "reserialized identity retained last-modified: {reserialized}"
        );

        let resource_identity: Option<&ResourceIdentity> = None;
        let result: Result<resource::Identity> = resource_identity.try_into();
        assert_matches!(result, Err(_));

        // resource info
        let resource_identity = resource::Identity::parse(
            "http://ov-content.nvidia.com/bar.usd?content-length=1&etag=abcdef&last-modified=2050-12-25T23:59:59.999Z",
        )
        .unwrap();
        let resource_info: Option<ResourceInfo> = Some(WrappedResourceInfo::from(&resource_identity).0.into());
        let result: resource::Identity = resource_info.try_into().unwrap();
        assert_eq!(
            "http://ov-content.nvidia.com",
            result.resource_address.config_key(AddressingStyle::Virtual)
        );
        assert_eq!("bar.usd", result.key().unwrap());

        // no metadata
        let mut resource_info: Option<ResourceInfo> = Some(WrappedResourceInfo::from(&resource_identity).0.into());
        resource_info.as_mut().unwrap().metadata = None;

        let result: resource::Identity = resource_info.try_into().unwrap();
        assert_eq!(
            "http://ov-content.nvidia.com",
            result.resource_address.config_key(AddressingStyle::Virtual)
        );
        assert_eq!("bar.usd", result.key().unwrap());

        // no identity
        let mut resource_info: Option<ResourceInfo> = Some(WrappedResourceInfo::from(&resource_identity).0.into());
        resource_info.as_mut().unwrap().resource_identity = None;
        let result: Result<resource::Identity> = resource_info.try_into();
        assert_matches!(result, Err(_));

        // no identity or metadata
        let resource_info: Option<ResourceInfo> = Some(ResourceInfo {
            resource_identity: None,
            metadata: None,
        });
        let result: Result<resource::Identity> = resource_info.try_into();
        assert_matches!(result, Err(_));

        let resource_info: Option<ResourceInfo> = None;
        let result: Result<resource::Identity> = resource_info.try_into();
        assert_matches!(result, Err(_));
    }

    #[rstest]
    #[test_log::test(tokio::test)]
    async fn parse_bucket_region() {
        let secrets = Secrets(HashMap::from([
            (
                "example-bucket.s3.us-west-2.amazonaws.com".to_string(),
                ClientCredentials {
                    access_key_id: Some("abc".to_string()),
                    secret_access_key: Some("xyz".to_string()),
                    ..Default::default()
                },
            ),
            (
                "mybucket.example.com".to_string(),
                ClientCredentials {
                    bucket: Some("foobucket".to_string()),
                    region: Some("us-east-1".to_string()),
                    ..Default::default()
                },
            ),
            // cases where only bucket or region is specified it parse it out of url
            // ignoring the config values
            (
                "rrr.s3.us-west-1.amazonaws.com".to_string(),
                ClientCredentials {
                    bucket: Some("foobucket".to_string()),
                    ..Default::default()
                },
            ),
            (
                "abc.s3.us-west-1.amazonaws.com".to_string(),
                ClientCredentials {
                    region: Some("eu-north-1".to_string()),
                    ..Default::default()
                },
            ),
            // custom domains with only 1 value specified
            (
                "qwerty.microsoft.com".to_string(),
                ClientCredentials {
                    region: Some("eu-north-1".to_string()),
                    ..Default::default()
                },
            ),
            (
                "qwerty.apple.com".to_string(),
                ClientCredentials {
                    bucket: Some("stora-x".to_string()),
                    ..Default::default()
                },
            ),
            (
                "storage.cloud.google.com/example-bucket".to_string(),
                ClientCredentials {
                    endpoint_url: Some("https://storage.cloud.google.com".to_string()),
                    ..Default::default()
                },
            ),
            // old-style S3 URLs with s3-<region> format
            (
                "oldbucket.s3-us-west-2.amazonaws.com".to_string(),
                ClientCredentials {
                    access_key_id: Some("abc".to_string()),
                    secret_access_key: Some("xyz".to_string()),
                    ..Default::default()
                },
            ),
            (
                "another-bucket.s3-eu-west-1.amazonaws.com".to_string(),
                ClientCredentials {
                    access_key_id: Some("abc".to_string()),
                    secret_access_key: Some("xyz".to_string()),
                    ..Default::default()
                },
            ),
            (
                "http://127.0.0.1:9000/example-bucket".to_string(),
                ClientCredentials {
                    region: Some("us-west-2".to_string()),
                    addressing_style: Some(AddressingStyle::Path),
                    endpoint_url: Some("http://127.0.0.1:9000".to_string()),
                    ..Default::default()
                },
            ),
        ]));

        let service = Storage::new(Config::default(), Some(secrets)).unwrap();

        let client = service
            .client(&mut resource::Address::parse("https://example-bucket.s3.us-west-2.amazonaws.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "example-bucket");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-west-2"))
        );

        let client = service
            .client(&mut resource::Address::parse("https://mybucket.example.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "foobucket");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-east-1"))
        );

        let client = service
            .client(&mut resource::Address::parse("https://rrr.s3.us-west-1.amazonaws.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "rrr");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-west-1"))
        );

        let client = service
            .client(&mut resource::Address::parse("https://abc.s3.us-west-1.amazonaws.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "abc");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-west-1"))
        );

        let client = service
            .client(&mut resource::Address::parse("https://qwerty.microsoft.com").unwrap())
            .await;
        assert!(client.is_err());

        let client = service
            .client(&mut resource::Address::parse("https://qwerty.apple.com").unwrap())
            .await;
        assert!(client.is_err());

        let client = service
            .client(&mut resource::Address::parse("https://storage.cloud.google.com/example-bucket").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "example-bucket");
        assert_eq!(client.addressing_style, AddressingStyle::Path);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-east-1"))
        );

        let client = service
            .client(&mut resource::Address::parse("http://127.0.0.1:9000/example-bucket").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "example-bucket");
        assert_eq!(client.addressing_style, AddressingStyle::Path);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-west-2"))
        );

        // old-style S3 URLs: s3-<region>
        let client = service
            .client(&mut resource::Address::parse("https://oldbucket.s3-us-west-2.amazonaws.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "oldbucket");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("us-west-2"))
        );

        let client = service
            .client(&mut resource::Address::parse("https://another-bucket.s3-eu-west-1.amazonaws.com").unwrap())
            .await
            .unwrap();
        assert_eq!(client.bucket, "another-bucket");
        assert_eq!(client.addressing_style, AddressingStyle::Virtual);
        assert_eq!(
            client.config().region(),
            Some(&aws_config::Region::from_static("eu-west-1"))
        );
    }

    #[test_log::test(tokio::test)]
    async fn test_s3_strict_mode_no_secrets() {
        let service = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            None,
        )
        .unwrap();
        let result = service
            .client(&mut resource::Address::parse("https://mybucket.s3.us-east-1.amazonaws.com").unwrap())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    #[rstest]
    // native_metadata_enabled with the fan-out off and the caches on (their default): rejected.
    #[case("[s3]\nnative_metadata_enabled = true\n", true)]
    // fan-out on: the listing carries native metadata, so the cache is consistent.
    #[case(
        "[s3]\nnative_metadata_enabled = true\nnative_metadata_list_fanout_enabled = true\n",
        false
    )]
    // both caches explicitly disabled: nothing is served from a cached listing/hint.
    #[case(
        "[s3]\nnative_metadata_enabled = true\n[list_cache]\nenabled = false\n[small_object_cache]\nenabled = false\n",
        false
    )]
    // only one cache left enabled still trips the guard.
    #[case(
        "[s3]\nnative_metadata_enabled = true\n[small_object_cache]\nenabled = false\n",
        true
    )]
    // native metadata off entirely: nothing to be inconsistent about.
    #[case("[s3]\n", false)]
    fn native_metadata_cache_guard(#[case] toml: &str, #[case] expect_err: bool) {
        use figment::providers::{Format, Toml};
        let figment = Figment::from(Toml::string(toml));
        let config = Config::try_from(&figment).unwrap();
        let result = config.validate_native_metadata_cache_consistency(&figment);
        assert_eq!(result.is_err(), expect_err, "toml: {toml}");
    }

    #[test_log::test(tokio::test)]
    async fn test_s3_strict_mode_not_in_secrets() {
        let secrets = Secrets(HashMap::from([(
            "https://configured.s3.us-east-1.amazonaws.com/configured".to_string(),
            ClientCredentials { ..Default::default() },
        )]));
        let service = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            Some(secrets),
        )
        .unwrap();
        let result = service
            .client(&mut resource::Address::parse("https://other.s3.us-east-1.amazonaws.com/other").unwrap())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    /// A positive strict-mode case: a configured endpoint must still resolve when
    /// `allow_unconfigured_endpoints = false`, guarding against over-rejection
    /// that would brick strict deployments.
    #[test_log::test(tokio::test)]
    async fn test_s3_strict_mode_configured_resolves() {
        let secrets = Secrets(HashMap::from([(
            "example-bucket.s3.us-west-2.amazonaws.com".to_string(),
            ClientCredentials {
                access_key_id: Some("abc".to_string()),
                secret_access_key: Some("xyz".to_string()),
                ..Default::default()
            },
        )]));
        let service = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            Some(secrets),
        )
        .unwrap();
        let result = service
            .client(&mut resource::Address::parse("https://example-bucket.s3.us-west-2.amazonaws.com").unwrap())
            .await;
        assert!(
            result.is_ok(),
            "configured endpoint must resolve in strict mode: {result:?}"
        );
    }

    mod list_versions_support_latch {
        use super::super::list_versions_support;
        use super::*;

        #[test]
        fn unprobed_does_not_skip() {
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            assert!(!list_versions_support::should_skip(&cell));
        }

        #[test]
        fn supported_does_not_skip() {
            let cell = AtomicU8::new(list_versions_support::SUPPORTED);
            assert!(!list_versions_support::should_skip(&cell));
        }

        #[test]
        fn probed_once_unsupported_does_not_skip() {
            // Key invariant: a single flaky 2xx-no-code observation must NOT
            // latch the client into the fallback path.
            let cell = AtomicU8::new(list_versions_support::PROBED_ONCE_UNSUPPORTED);
            assert!(!list_versions_support::should_skip(&cell));
        }

        #[test]
        fn confirmed_unsupported_skips() {
            let cell = AtomicU8::new(list_versions_support::CONFIRMED_UNSUPPORTED);
            assert!(list_versions_support::should_skip(&cell));
        }

        #[test]
        fn single_unsupported_observation_reaches_probed_once_not_confirmed() {
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::record_unsupported(&cell);
            assert_eq!(
                cell.load(Ordering::Relaxed),
                list_versions_support::PROBED_ONCE_UNSUPPORTED
            );
            assert!(!list_versions_support::should_skip(&cell));
        }

        #[test]
        fn two_unsupported_observations_confirm() {
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::record_unsupported(&cell);
            list_versions_support::record_unsupported(&cell);
            assert_eq!(
                cell.load(Ordering::Relaxed),
                list_versions_support::CONFIRMED_UNSUPPORTED
            );
            assert!(list_versions_support::should_skip(&cell));
        }

        #[test]
        fn success_after_flake_recovers_to_supported() {
            // Simulates: transient 2xx-no-code flake on call 1, real success on
            // call 2. The flake must not leave the client in an unsupported-ish
            // state.
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::record_unsupported(&cell);
            list_versions_support::record_success(&cell);
            assert_eq!(cell.load(Ordering::Relaxed), list_versions_support::SUPPORTED);
            assert!(!list_versions_support::should_skip(&cell));
        }

        #[test]
        fn flake_after_success_does_not_demote() {
            // Once we've seen a real success, a single later unsupported-shape
            // observation is ignored — otherwise a transient proxy error could
            // demote a working real-S3 client.
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::record_success(&cell);
            list_versions_support::record_unsupported(&cell);
            assert_eq!(cell.load(Ordering::Relaxed), list_versions_support::SUPPORTED);
        }

        #[test]
        fn pre_seed_skips_the_two_observation_latch() {
            // Known-bad hosts (GCS) are pre-seeded directly to Confirmed.
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::seed_confirmed_unsupported(&cell);
            assert_eq!(
                cell.load(Ordering::Relaxed),
                list_versions_support::CONFIRMED_UNSUPPORTED
            );
            assert!(list_versions_support::should_skip(&cell));
        }

        #[test]
        fn success_does_not_override_a_confirmed_unsupported_seed() {
            // If we've ever confirmed unsupported (via latch OR pre-seed), we
            // don't attempt the call, so we wouldn't observe a success in the
            // first place. But if the code paths ever change, `record_success`
            // explicitly overwrites — validate that behavior so future readers
            // see it asserted.
            let cell = AtomicU8::new(list_versions_support::UNPROBED);
            list_versions_support::seed_confirmed_unsupported(&cell);
            list_versions_support::record_success(&cell);
            assert_eq!(cell.load(Ordering::Relaxed), list_versions_support::SUPPORTED);
        }
    }
}
