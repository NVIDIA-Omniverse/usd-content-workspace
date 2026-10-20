// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Debug,
    ops::Deref,
    sync::Arc,
};

use azure_core::headers::{HeaderName, HeaderValue, Headers};
use azure_core::{Context, CustomHeaders, ExponentialRetryOptions, HttpClient, TransportOptions};
use azure_storage::shared_access_signature::service_sas::UserDeligationKey;
use azure_storage::{CloudLocation, StorageCredentials};
use azure_storage_blobs::prelude::{BlobServiceClient, ClientBuilder};
use bytesize::ByteSize;
use figment::Figment;
use moka::future::Cache as MokaCache;
use serde::{Deserialize, Serialize};
use storage_core_util::{info_limited, warn_limited};
use tokio::sync::RwLock;
use url::{Position, Url};

use storage_core_util::{
    additional_headers::{AdditionalHeaders, RequestType},
    expiration::{Config as ExpirationConfig, ExpirationNormalizer},
    retry::Config as RetryConfig,
    secrets::{normalize_url_keys, SecretsReloadable},
    timeouts::Config as TimeoutConfig,
};

use crate::azure::secrets::CredentialOverride;
use crate::utils::{ucc_proxy_tls_verification, UCC_PROXY_ENV};

use super::error::Error;
use super::resource::Address;
use super::secrets::{ClientCredentials, Secrets};

// Current version https://learn.microsoft.com/en-us/rest/api/storageservices/versioning-for-the-azure-storage-services
pub static X_MS_VERISION: &str = "2025-01-05";

/// Configuration for the Azure storage backend.
///
/// The configuration key is `azure` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [azure]
/// emulator = false
/// read_redirect_size = "256KB"
/// write_body_max_size = "2MB"
/// write_redirect_max_size = "64MB"
///
/// [azure.timeouts]
/// connect = "10s"
/// read = "10s"
/// operation = "10s"
/// operation_attempt = "10s"
/// keepalive = "10s"
///
/// [azure.expiration]
/// presigned_expiration = "15m"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// connect to the Azurite emulator running locally
    #[serde(default = "default_emulator")]
    pub emulator: bool,

    /// when getting an object, if no download preference is specified and the object
    /// is smaller than this threshold, the object will be sent over the gRPC channel
    /// as opposed to a returning a presigned url for the client to fetch via HTTP(S)
    #[serde(default = "default_read_redirect_size")]
    pub read_redirect_size: ByteSize,

    /// When writing an object, if no upload preference is specified, then for any object smaller than this size,
    /// the storage service will instruct the client to upload the data directly through the storage service
    #[serde(default = "default_write_body_max_size")]
    pub write_body_max_size: ByteSize,

    /// When writing an object, if no upload preference is specified, then for any object larger than write_body_max_size,
    /// but smaller than this size, the storage service will return a presigned url to the client to use to upload the data.
    /// Above this size, the service will instruct the client to use a multipart upload.
    #[serde(default = "default_write_redirect_max_size")]
    pub write_redirect_max_size: ByteSize,

    /// Retry policy for the Azure storage backend.
    ///
    /// See the [RetryConfig] struct for more details.
    #[serde(default)]
    pub retry: RetryConfig,

    /// Timeouts for the Azure storage backend.
    ///
    /// See the [TimeoutConfig] struct for more details.
    #[serde(default)]
    pub timeouts: TimeoutConfig,

    /// Presigned url expiration for the Azure storage backend.
    ///
    /// See the [ExpirationConfig] struct for more details.
    #[serde(default)]
    pub expiration: ExpirationConfig,

    /// Maximum number of idle HTTP connections kept in the pool per host.
    ///
    /// When unset (the default), reqwest keeps an unbounded number of idle
    /// connections, allowing aggressive connection reuse. This prevents TCP
    /// port exhaustion under high request concurrency.
    ///
    /// Set to `0` to disable connection reuse — every request opens a new TCP
    /// connection. This was the previous hardcoded default and works around a
    /// hang in `hyper` (<https://github.com/hyperium/hyper/issues/2312>). Use
    /// it only if you observe client hangs in production.
    ///
    /// Set to a positive integer to cap the pool size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_max_idle_per_host: Option<usize>,

    /// When `false`, requests for Azure storage accounts that are **not** explicitly
    /// listed in the secrets configuration will be rejected with a `NotFound` error
    /// instead of falling through to anonymous access.
    ///
    /// Default: `true` (backwards-compatible — existing deployments keep the current
    /// fall-through behaviour).  New deployments should set this to `false` to prevent
    /// accidental access to unconfigured storage accounts.
    #[serde(default = "default_allow_unconfigured_endpoints")]
    pub allow_unconfigured_endpoints: bool,

    /// Operator-specified backend-wide default for account versioning state; a
    /// per-endpoint `VERSIONING_ENABLED` entry in the secrets configuration
    /// overrides it for that endpoint. When the resolved value is `false`,
    /// `EnumerateVersions` for the endpoint is short-circuited with
    /// `FAILED_PRECONDITION` before reaching the backend and
    /// `IsVersioned` reports `false`. When it is `true` or nothing is
    /// configured (the default), versioning is treated as enabled:
    /// `EnumerateVersions` goes to the backend unchanged and
    /// `IsVersioned` reports `true`. There is no runtime detection —
    /// the state is whatever the operator configures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versioning_enabled: Option<bool>,

    /// Whether FileObject-native metadata is enabled for this backend. When
    /// unset (the default) native metadata is treated as disabled:
    /// `GetNativeMetadataSupport` reports `false`, writes carrying non-empty
    /// `WriteParameters.native_metadata` are rejected with UNIMPLEMENTED, and
    /// `ListStat` with `include_native_metadata` does not fetch it. When
    /// enabled, native metadata is surfaced on the free stat/read paths (no extra
    /// backend call) and on list (Azure returns it inline). Writes carrying
    /// non-empty native metadata are still rejected with UNIMPLEMENTED on every
    /// upload path (direct-body, redirect and multipart) — write persistence is a
    /// follow-up. Because that write support is incomplete,
    /// `GetNativeMetadataSupport` reports `false` in this pre-release state
    /// regardless of this flag; it will report the flag once write persistence
    /// lands. There is no runtime detection — the state is whatever the operator
    /// configures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_metadata_enabled: Option<bool>,
    /// When `true`, addresses served by this backend are reported as read-only
    /// by `IsReadOnly`. When `false` or unset (the default), addresses are
    /// reported as writable. This is a declarative operator setting; there is no
    /// runtime detection and it does not itself enforce write rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

fn default_emulator() -> bool {
    false
}

fn default_read_redirect_size() -> ByteSize {
    ByteSize::kb(256)
}

fn default_write_body_max_size() -> ByteSize {
    ByteSize::mb(2)
}

fn default_write_redirect_max_size() -> ByteSize {
    ByteSize::mb(64)
}

fn default_allow_unconfigured_endpoints() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            emulator: default_emulator(),
            read_redirect_size: default_read_redirect_size(),
            write_body_max_size: default_write_body_max_size(),
            write_redirect_max_size: default_write_redirect_max_size(),
            retry: RetryConfig::default(),
            timeouts: TimeoutConfig::default(),
            expiration: ExpirationConfig::default(),
            pool_max_idle_per_host: None,
            allow_unconfigured_endpoints: default_allow_unconfigured_endpoints(),
            versioning_enabled: None,
            native_metadata_enabled: None,
            read_only: None,
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "azure";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

#[derive(Debug)]
pub struct AzureClient {
    pub client: BlobServiceClient,
    pub anonymous: bool,
    /// The Front Door client for presigned URLs.
    pub front_door_client: Option<BlobServiceClient>,
    /// Whether to use User Delegation SAS (service principal auth) instead of account key SAS.
    pub use_user_delegation: bool,
}

impl AzureClient {
    /// Creates an AzureClient configured for the local Azurite emulator.
    fn emulator() -> Self {
        Self {
            client: ClientBuilder::emulator().blob_service_client(),
            anonymous: false,
            front_door_client: None,
            use_user_delegation: false,
        }
    }

    /// Creates an AzureClient from resolved configuration.
    fn from_config(
        config: ResolvedClientConfig,
        transport_options: TransportOptions,
        retry_options: azure_core::RetryOptions,
    ) -> Self {
        let front_door_client = config.front_door_url.map(|url| {
            BlobServiceClient::builder(url.to_string(), config.credentials.clone())
                .cloud_location(CloudLocation::Custom {
                    account: config.account.clone(),
                    uri: url.to_string(),
                })
                .blob_service_client()
        });

        Self {
            client: ClientBuilder::new(config.account, config.credentials)
                .transport(transport_options)
                .retry(retry_options)
                .blob_service_client(),
            anonymous: config.anonymous,
            front_door_client,
            use_user_delegation: config.use_user_delegation,
        }
    }

    /// Creates an AzureClient with an optional Front Door URL for testing.
    /// Uses the emulator as the base client and anonymous credentials.
    #[cfg(test)]
    fn with_front_door_url(front_door_url: Option<Url>) -> Self {
        let front_door_client = front_door_url.map(|url| {
            BlobServiceClient::builder(url.to_string(), StorageCredentials::anonymous())
                .cloud_location(CloudLocation::Custom {
                    account: "teststorage".to_string(),
                    uri: url.to_string(),
                })
                .blob_service_client()
        });

        Self {
            client: ClientBuilder::emulator().blob_service_client(),
            anonymous: false,
            front_door_client,
            use_user_delegation: false,
        }
    }
}

impl Deref for AzureClient {
    type Target = BlobServiceClient;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

// The key is the hostname of the requested URL, the value is the client that can connect to it
pub type ClientMap = RwLock<HashMap<String, Arc<AzureClient>, ahash::RandomState>>;

pub struct Storage {
    pub clients: ClientMap,
    pub http_client: Arc<dyn HttpClient>,
    pub config: Config,
    pub secrets: RwLock<Option<Secrets>>,
    pub expiration_normalizer: ExpirationNormalizer,
    /// Cached User Delegation Keys for service principal authentication, keyed by
    /// `(account, sas_expiry_unix_timestamp)`.
    ///
    /// Delegation keys are scoped to the storage account that issued them, so each account
    /// needs its own cached key. The SAS expiry in the cache key ensures that when the
    /// `ExpirationNormalizer` epoch rolls over (producing a new SAS expiry), we fetch a fresh
    /// delegation key. Within the same epoch, all replicas produce the same SAS expiry, so
    /// they'll all cache-hit on the same delegation key and generate identical presigned URLs.
    ///
    /// Uses Moka's `try_get_with` for automatic request coalescing — if multiple tasks need
    /// the same delegation key concurrently, only one will fetch it from Azure while the
    /// others await the result. Stale entries are evicted via TTL.
    ///
    /// This is separate from the OAuth2 access token cache inside `ClientSecretCredential`,
    /// which the `azure_identity` SDK manages automatically. See `get_or_fetch_delegation_key`
    /// for details on the two-layer credential model.
    delegation_key_cache: MokaCache<(String, i64), UserDeligationKey>,
    pub(crate) rate_limit_counters: super::rate_limit::RateLimitCounters,
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
        let expiration_normalizer = ExpirationNormalizer::new(&config.expiration);

        info_limited!(?config);

        if config.emulator && !config.allow_unconfigured_endpoints {
            return Err(Error::invalid_argument(
                "emulator = true and allow_unconfigured_endpoints = false are incompatible; \
                 the emulator accepts any hostname and cannot enforce an endpoint allowlist",
            )
            .into());
        }

        if let Some(ref mut s) = secrets {
            storage_core_util::secrets::normalize_url_keys(s);
        }

        // build a http client manually so we specify various timeouts for http operations
        let mut http_builder = reqwest::ClientBuilder::new()
            .tcp_nodelay(true)
            .connect_timeout(config.timeouts.connect)
            .read_timeout(config.timeouts.read)
            .timeout(config.timeouts.operation)
            .tcp_keepalive(config.timeouts.keepalive);
        if let Some(max_idle) = config.pool_max_idle_per_host {
            http_builder = http_builder.pool_max_idle_per_host(max_idle);
        }
        // setup any http/https proxy if UCC_PROXY is set
        let ucc_proxy_enabled = if let Ok(proxy_address) = std::env::var(UCC_PROXY_ENV) {
            // TLS verification is off only for UCC_PROXY_SSL_VERIFYCERT=insecure (local test proxies)
            let verify_certs = ucc_proxy_tls_verification(&proxy_address);

            http_builder = http_builder
                .danger_accept_invalid_certs(!verify_certs)
                .danger_accept_invalid_hostnames(!verify_certs)
                .proxy(
                    reqwest::Proxy::all(proxy_address)
                        .map_err(|e| Error::internal(format!("failed to create http proxy: {e}")))?,
                );

            true
        } else {
            false
        };

        let http_client = http_builder
            .build()
            .map_err(|e| Error::internal(format!("failed to create http client: {e}")))?;

        // if proxy is enabled,wrap the http client in our own that will replace the scheme with http
        let http_client: Arc<dyn HttpClient> = if ucc_proxy_enabled {
            Arc::new(AzureHttpClient(http_client))
        } else {
            Arc::new(http_client)
        };

        let secrets_count = secrets.as_ref().map_or(0, |s| s.len());
        let expiration_duration = std::time::Duration::from_secs(2 * config.expiration.presigned_expiration.as_secs());

        info_limited!("Azure backend running with config: {:?}", config);
        Ok(Self {
            clients: ClientMap::default(),
            http_client,
            config,
            secrets: RwLock::new(secrets),
            expiration_normalizer,
            delegation_key_cache: MokaCache::builder()
                // At most one entry per (account, epoch). Allow 3× the number of
                // configured accounts to accommodate brief overlap during epoch
                // transitions. Minimum of 3 so the cache is functional even with
                // no secrets configured (anonymous/runtime-discovered accounts).
                .max_capacity((secrets_count as u64 * 3).max(3))
                .time_to_live(expiration_duration)
                .build(),
            rate_limit_counters: super::rate_limit::RateLimitCounters::new("azure"),
        })
    }

    /// Creates a new Azure storage service from a figment config and secrets.
    ///
    /// The config will be extracted from the Figment struct, looking for the `azure` key.
    /// The secrets will be extracted from the Figment struct, looking for the `storage-service-azure` key.
    pub fn from_figment(config: &Figment, secrets: &Option<Figment>) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::try_from(config)?;
        let secrets = secrets.as_ref().map(Secrets::try_from).transpose()?;
        Self::new(config, secrets)
    }

    /// Gets a client for a resource address.
    ///
    /// If the client does not exist, it will be created.
    pub async fn client(&self, address: &Address) -> Result<Arc<AzureClient>, Error> {
        self.get_or_create_client(&address.resource_address).await
    }

    /// Gets a client for a URL.
    ///
    /// If the client does not exist, it will be created.
    ///
    /// This is used to get a client for a URL that does not contain a container name, which would
    /// not be a valid resource address.
    pub async fn client_from_url(&self, url: &Url) -> Result<Arc<AzureClient>, Error> {
        self.get_or_create_client(url).await
    }

    /// Checks whether the endpoint identified by `url` is listed in the secrets
    /// configuration when `allow_unconfigured_endpoints = false`.
    ///
    /// Returns `Ok(())` immediately when:
    /// - `allow_unconfigured_endpoints` is `true` (default), or
    /// - the emulator is enabled (the emulator accepts any hostname).
    ///
    /// Otherwise returns `Err(NotFound)` when no matching entry is found.
    pub async fn check_endpoint_configured(&self, url: &Url) -> Result<(), Error> {
        if self.config.allow_unconfigured_endpoints {
            return Ok(());
        }
        if self.config.emulator {
            return Ok(());
        }
        let lookup_key = &url[..Position::BeforePath];
        let secrets_guard = self.secrets.read().await;
        let hostname = url.host_str().unwrap_or("unknown");
        match &*secrets_guard {
            None => Err(Error::not_found(format!("endpoint not configured: {hostname}"))),
            Some(secrets) => {
                if secrets.contains_key(lookup_key) {
                    Ok(())
                } else {
                    Err(Error::not_found(format!("endpoint not configured: {hostname}")))
                }
            },
        }
    }

    async fn get_or_create_client(&self, url: &Url) -> Result<Arc<AzureClient>, Error> {
        let hostname = Address::parse_hostname(url)?;

        // this is optimized for the read/hit case
        // the clients map is not expected to be modified very often
        if let Some(client) = self.clients.read().await.get(hostname) {
            return Ok(client.clone());
        }

        match self.clients.write().await.entry(hostname.into()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let client = if self.config.emulator {
                    AzureClient::emulator()
                } else {
                    // Scope the lock guard so it's dropped before any .await points
                    let config = {
                        let secrets_guard = self.secrets.read().await;
                        Self::resolve_client_config(
                            url,
                            &secrets_guard,
                            &self.http_client,
                            self.config.allow_unconfigured_endpoints,
                        )?
                    };
                    let transport_options = TransportOptions::new(self.http_client.clone());
                    let retry_options = azure_core::RetryOptions::exponential(
                        ExponentialRetryOptions::default()
                            .max_retries(self.config.retry.attempts)
                            .initial_delay(self.config.retry.initial_backoff)
                            .max_delay(self.config.retry.max_backoff)
                            .max_total_elapsed(self.config.retry.max_total_elapsed),
                    );
                    AzureClient::from_config(config, transport_options, retry_options)
                };
                Ok(entry.insert(Arc::new(client)).clone())
            },
        }
    }

    /// Gets or fetches a User Delegation Key for generating User Delegation SAS tokens.
    ///
    /// # User Delegation Keys vs OAuth2 Tokens
    ///
    /// When using service principal (SP) authentication, there are two layers of credentials:
    ///
    /// 1. **OAuth2 access token** — obtained by `ClientSecretCredential` from Azure AD using the
    ///    client credentials flow (tenant_id + client_id + client_secret). This token authenticates
    ///    the service principal to Azure Storage for direct API calls (stat, read, write, etc.).
    ///    The `azure_identity` SDK caches this token internally via `TokenCache` and automatically
    ///    refreshes it when expired (with a 20-second buffer). We do not manage this ourselves.
    ///
    /// 2. **User Delegation Key** — a signing key obtained from the Azure Storage service via
    ///    `POST ?restype=service&comp=userdelegationkey`. This key is used to generate User
    ///    Delegation SAS tokens for presigned URLs. Unlike the OAuth2 token, the SDK does not
    ///    cache this — it requires a full HTTP round-trip to Azure Storage each time. We cache
    ///    it here to avoid making that call on every presigned URL generation.
    ///
    /// # Caching strategy
    ///
    /// The key is cached by `(account, sas_expiry)`. The `sas_expiry` comes from the
    /// `ExpirationNormalizer`, which aligns expiry times to deterministic epoch boundaries.
    /// This means:
    /// - Within the same epoch, all requests hit the same cache entry → one delegation key fetch.
    /// - All service replicas compute the same epoch → identical SAS tokens → cacheable by CDN.
    /// - When the epoch rolls over, `sas_expiry` changes → cache miss → new delegation key.
    ///
    /// The delegation key is requested with `signed_expiry = sas_expiry` so it's guaranteed to
    /// be valid for the lifetime of any SAS token signed with it.
    pub async fn get_or_fetch_delegation_key(
        &self,
        service_client: &BlobServiceClient,
        sas_expiry: time::OffsetDateTime,
    ) -> Result<UserDeligationKey, Error> {
        let account = service_client.account().to_string();
        let cache_key = (account.clone(), sas_expiry.unix_timestamp());

        // Moka's `try_get_with` handles both caching and request coalescing: on a cache
        // miss, only one task executes the init future while concurrent callers with the
        // same key await its result.
        self.delegation_key_cache
            .try_get_with(cache_key, async {
                // Subtract 15 minutes from `now` to account for clock skew between this
                // machine and Azure's servers. If our clock is ahead of Azure's, a
                // `signed_start` of exactly `now` would be in Azure's future, causing
                // "Signature not valid in the specified key time frame" errors.
                // Azure recommends setting start time at least 15 minutes in the past:
                // https://learn.microsoft.com/en-us/azure/storage/common/storage-sas-overview#best-practices-when-using-sas
                let signed_start = time::OffsetDateTime::now_utc() - time::Duration::minutes(15);
                let response = service_client
                    .get_user_deligation_key(signed_start, sas_expiry)
                    .into_future()
                    .await
                    .map_err(Error::from)?;

                info_limited!(account, "Fetched new User Delegation Key (valid until: {})", sas_expiry);
                Ok(response.user_deligation_key) as Result<UserDeligationKey, Error>
            })
            .await
            .map_err(|e| e.as_ref().clone())
    }

    /// Resolves the client configuration for a given URL from secrets.
    ///
    /// This determines the storage account, credentials, and optional front door URL
    /// based on the provided secrets configuration. If no secrets are provided or
    /// the hostname is not found, anonymous access is used.
    fn resolve_client_config(
        url: &Url,
        secrets: &Option<Secrets>,
        http_client: &Arc<dyn HttpClient>,
        allow_unconfigured_endpoints: bool,
    ) -> Result<ResolvedClientConfig, Error> {
        // Secrets keys are normalized to include a scheme (see Storage::new), so use
        // the scheme://authority portion of the request URL as the lookup key.
        let lookup_key = &url[..Position::BeforePath];

        let Some(secrets) = secrets else {
            if !allow_unconfigured_endpoints {
                let hostname = url.host_str().unwrap_or("unknown");
                return Err(Error::not_found(format!("endpoint not configured: {hostname}")));
            }
            warn_limited!(lookup_key, "No config provided. Running in anonymous mode");
            return ResolvedClientConfig::anonymous(url);
        };

        let Some(val) = secrets.get(lookup_key) else {
            if !allow_unconfigured_endpoints {
                let hostname = url.host_str().unwrap_or("unknown");
                return Err(Error::not_found(format!("endpoint not configured: {hostname}")));
            }
            warn_limited!(lookup_key, "Not found in config. Running in anonymous mode");
            return ResolvedClientConfig::anonymous(url);
        };

        let account = val
            .storage_account
            .as_ref()
            .map(|a| a.to_string())
            .map_or_else(|| Address::account_from_url(url).map(|a| a.to_string()), Ok)?;

        let front_door_url = Self::parse_front_door_url(val.front_door_url.as_deref())?;

        if let Some(credentials_override) = val.credential_override.as_ref() {
            let (credentials, anonymous, use_user_delegation) = match credentials_override {
                CredentialOverride::Anonymous => {
                    info_limited!(lookup_key, "using anonymous credential override");
                    (StorageCredentials::anonymous(), true, false)
                },
                CredentialOverride::TokenCredential(token) => {
                    info_limited!(lookup_key, "using token credential override");
                    (StorageCredentials::token_credential(token.clone()), false, true)
                },
            };

            return Ok(ResolvedClientConfig {
                credentials,
                anonymous,
                use_user_delegation,
                account,
                front_door_url,
            });
        }

        // Check for service principal credentials.
        // All three fields (tenant_id, client_id, client_secret) must be present to use SP auth.
        let sp_fields = [
            ("AZURE_TENANT_ID", &val.tenant_id),
            ("AZURE_CLIENT_ID", &val.client_id),
            ("AZURE_CLIENT_SECRET", &val.client_secret),
        ];
        let sp_present: Vec<&str> = sp_fields.iter().filter(|(_, v)| v.is_some()).map(|(k, _)| *k).collect();
        let sp_missing: Vec<&str> = sp_fields.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| *k).collect();

        if let (Some(tenant_id), Some(client_id), Some(client_secret)) =
            (&val.tenant_id, &val.client_id, &val.client_secret)
        {
            // All SP fields present — use service principal authentication.
            //
            // This creates a `ClientSecretCredential` which authenticates via the OAuth2 client
            // credentials flow (POST to Azure AD token endpoint). The `azure_identity` SDK
            // internally caches the resulting OAuth2 access token and automatically refreshes it
            // when expired — we do not need to manage token lifecycle ourselves.
            //
            // With SP auth, presigned URLs use User Delegation SAS instead of account-key SAS.
            // The delegation key is cached separately in `Storage::delegation_key_cache`.
            if val.storage_key.is_some() {
                warn_limited!(
                    lookup_key,
                    "Both service principal credentials and AZURE_STORAGE_KEY are configured. \
                     Service principal will be used; AZURE_STORAGE_KEY will be ignored."
                );
            }

            info_limited!(lookup_key, "Using service principal credentials from config");
            info_limited!(
                configuration_key = lookup_key,
                hash = val.credential_hash(),
                "credentials"
            );

            let authority_host = val
                .authority_host
                .as_deref()
                .unwrap_or(ClientCredentials::DEFAULT_AUTHORITY_HOST);
            let authority_url = Url::parse(authority_host).map_err(|e| {
                Error::invalid_argument(format!("Invalid authority host URL '{}': {}", authority_host, e))
            })?;

            let credential = azure_identity::ClientSecretCredential::new(
                http_client.clone(),
                authority_url,
                tenant_id.clone(),
                client_id.clone(),
                client_secret.clone(),
            );
            let credentials = StorageCredentials::token_credential(
                std::sync::Arc::new(credential) as std::sync::Arc<dyn azure_core::auth::TokenCredential>
            );

            return Ok(ResolvedClientConfig {
                credentials,
                account,
                anonymous: false,
                front_door_url,
                use_user_delegation: true,
            });
        } else if !sp_present.is_empty() {
            // Some but not all SP fields are set — this is likely a configuration mistake.
            warn_limited!(
                lookup_key,
                present = ?sp_present,
                missing = ?sp_missing,
                "Incomplete service principal configuration. All three of AZURE_TENANT_ID, \
                 AZURE_CLIENT_ID, and AZURE_CLIENT_SECRET must be provided. \
                 Falling back to access key or anonymous."
            );
        }

        match &val.storage_key {
            Some(key) => {
                info_limited!(lookup_key, "Using access key from config");
                info_limited!(
                    configuration_key = lookup_key,
                    hash = val.credential_hash(),
                    "credentials"
                );

                Ok(ResolvedClientConfig {
                    credentials: StorageCredentials::access_key(&account, key.to_string()),
                    account,
                    anonymous: false,
                    front_door_url,
                    use_user_delegation: false,
                })
            },
            None => {
                warn_limited!(
                    lookup_key,
                    "No credentials provided in config. Running in anonymous mode"
                );
                Ok(ResolvedClientConfig {
                    account,
                    credentials: StorageCredentials::anonymous(),
                    anonymous: true,
                    front_door_url,
                    use_user_delegation: false,
                })
            },
        }
    }

    /// Parses the front door URL configuration, which can be either:
    /// - A hostname only: `frontdoor.azurefd.net` (will be converted to `https://frontdoor.azurefd.net`)
    /// - A full URL: `https://frontdoor.azurefd.net/path/prefix`
    fn parse_front_door_url(url_str: Option<&str>) -> Result<Option<Url>, Error> {
        match url_str {
            Some(s) if !s.is_empty() => {
                // Check if it looks like a URL (has scheme) or just a hostname
                let url = if s.contains("://") {
                    Url::parse(s).map_err(|e| Error::internal(format!("Invalid front door URL '{}': {}", s, e)))?
                } else {
                    // Treat as hostname only, default to https
                    Url::parse(&format!("https://{}", s))
                        .map_err(|e| Error::internal(format!("Invalid front door hostname '{}': {}", s, e)))?
                };
                info_limited!(front_door_url = %url, "Configured front door URL");
                Ok(Some(url))
            },
            _ => Ok(None),
        }
    }

    pub fn create_custom_context(
        additional_headers: AdditionalHeaders,
        request_type: RequestType,
    ) -> Result<Context, Error> {
        let mut context = Context::new();
        let custom_headers: CustomHeaders = {
            let mut custom_headers = std::collections::HashMap::<HeaderName, HeaderValue>::new();
            additional_headers.request_headers(request_type).for_each(|h| {
                custom_headers.insert(h.0.into(), h.1.into());
            });
            let hs: Headers = custom_headers.into();
            hs.into()
        };
        context.insert(custom_headers);
        Ok(context)
    }
}

#[derive(Debug)]
struct AzureHttpClient(reqwest::Client);

#[async_trait::async_trait]
impl azure_core::HttpClient for AzureHttpClient {
    async fn execute_request(&self, request: &azure_core::Request) -> azure_core::Result<azure_core::Response> {
        // avoid http crate here because of the entanglements with hyper 0.14 and 1.x
        const HTTP_SCHEME: &str = "http";
        const HTTPS_SCHEME: &str = "https";

        // replace scheme with http. this tells the proxy to terminate the https connection.
        // using https would use CONNECT verb to the proxy. we currently don't support CONNECT.
        let mut req = request.clone();
        let url = req.url_mut();

        if url.scheme().eq_ignore_ascii_case(HTTPS_SCHEME) {
            url.set_scheme(HTTP_SCHEME).map_err(|_| {
                azure_core::Error::message(
                    azure_core::error::ErrorKind::Other,
                    format!("failed to set scheme to {HTTP_SCHEME}: {req:?}"),
                )
            })?;
        }

        self.0.execute_request(&req).await
    }
}

/// Resolved configuration for creating an Azure client for a specific hostname.
/// Contains all information needed to construct an AzureClient.
#[derive(Debug)]
struct ResolvedClientConfig {
    /// The storage account name
    account: String,
    /// The credentials to use for authentication
    credentials: StorageCredentials,
    /// Whether this is anonymous access (no credentials provided)
    anonymous: bool,
    /// Optional front door URL for presigned URLs
    front_door_url: Option<Url>,
    /// Whether to use User Delegation SAS for presigned URLs
    use_user_delegation: bool,
}

impl ResolvedClientConfig {
    /// Creates an anonymous configuration derived from the URL.
    fn anonymous(url: &Url) -> Result<Self, Error> {
        Ok(Self {
            account: Address::account_from_url(url)?.to_string(),
            credentials: StorageCredentials::anonymous(),
            anonymous: true,
            front_door_url: None,
            use_user_delegation: false,
        })
    }
}

impl SecretsReloadable for Storage {
    async fn reload_secrets(&self, secrets: Option<&Figment>) {
        let mut new_secrets = match secrets.map(Secrets::try_from).transpose() {
            Ok(s) => s,
            Err(e) => {
                warn_limited!(?e, "Failed to parse new Azure secrets, keeping existing credentials");
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
        self.delegation_key_cache.invalidate_all();
        info_limited!("Azure secrets reloaded, client and delegation key caches cleared");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::azure::secrets::ClientCredentials;
    use azure_storage::prelude::BlobSasPermissions;
    use azure_storage::StorageCredentials;
    use azure_storage::StorageCredentialsInner;
    use std::time::Duration;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;
    use storage_core_util::expiration;
    use time::OffsetDateTime;
    use url::Url;

    fn test_http_client() -> Arc<dyn HttpClient> {
        azure_core::new_http_client()
    }

    use super::*;

    #[test_log::test(tokio::test)]
    async fn test_azure_access_config() {
        let secrets = Secrets(HashMap::from([(
            "https://exampleaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("access_key".to_string()),
                ..Default::default()
            },
        )]));
        let address = Address::parse_blob(
            Url::parse("https://exampleaccount.blob.core.windows.net/example-public/hello.txt").unwrap(),
        )
        .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "exampleaccount");
        assert!(
            matches!(config.credentials.0.read().await.deref(), StorageCredentialsInner::Key(k, v) if k == "exampleaccount" && v.secret() == "access_key")
        );
        assert!(!config.anonymous);
        assert!(config.front_door_url.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_account_override() {
        let secrets = Secrets(HashMap::from([(
            "https://exampleaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_account: Some("custom_account".to_string()),
                storage_key: Some("access_key".to_string()),
                ..Default::default()
            },
        )]));
        let address = Address::parse_blob(
            Url::parse("https://exampleaccount.blob.core.windows.net/example-public/hello.txt").unwrap(),
        )
        .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "custom_account");
        assert!(
            matches!(config.credentials.0.read().await.deref(), StorageCredentialsInner::Key(k, v) if k == &config.account && v.secret() == "access_key")
        );
        assert!(!config.anonymous);
        assert!(config.front_door_url.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_none() {
        let url = Url::parse("https://exampleaccount.blob.core.windows.net/example-public/hello.txt").unwrap();

        let config = Storage::resolve_client_config(&url, &None, &test_http_client(), true).unwrap();
        assert_eq!(config.account, "exampleaccount");
        assert!(matches!(
            config.credentials.0.read().await.deref(),
            StorageCredentialsInner::Anonymous
        ));
        assert!(config.anonymous);
        assert!(config.front_door_url.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_invalid_url() {
        let url =
            Url::parse("https://example-bucket-public.s3.us-west-2.amazonaws.com/example-public/hello.txt").unwrap();

        let result = Storage::resolve_client_config(&url, &None, &test_http_client(), true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid Azure hostname format"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_with_front_door() {
        let secrets = Secrets(HashMap::from([(
            "https://teststorage.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("test_access_key".to_string()),
                front_door_url: Some("fd-teststorage-abc123.z01.azurefd.net".to_string()),
                ..Default::default()
            },
        )]));
        let address =
            Address::parse_blob(Url::parse("https://teststorage.blob.core.windows.net/container/blob.txt").unwrap())
                .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "teststorage");
        assert!(
            matches!(config.credentials.0.read().await.deref(), StorageCredentialsInner::Key(k, v) if k == "teststorage" && v.secret() == "test_access_key")
        );
        assert!(!config.anonymous);
        let front_door_url = config.front_door_url.expect("front_door_url should be set");
        assert_eq!(front_door_url.host_str(), Some("fd-teststorage-abc123.z01.azurefd.net"));
        assert_eq!(front_door_url.path(), "/");
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_front_door_with_account_override() {
        let secrets = Secrets(HashMap::from([(
            "https://customdomain.example.com".to_string(),
            ClientCredentials {
                storage_account: Some("overridden_account".to_string()),
                storage_key: Some("override_key".to_string()),
                front_door_url: Some("fd-custom-def456.b02.azurefd.net".to_string()),
                ..Default::default()
            },
        )]));

        let url = Url::parse("https://customdomain.example.com/container/blob").unwrap();
        let config = Storage::resolve_client_config(&url, &Some(secrets), &test_http_client(), true).unwrap();
        assert_eq!(config.account, "overridden_account");
        assert!(
            matches!(config.credentials.0.read().await.deref(), StorageCredentialsInner::Key(k, v) if k == "overridden_account" && v.secret() == "override_key")
        );
        assert!(!config.anonymous);
        let front_door_url = config.front_door_url.expect("front_door_url should be set");
        assert_eq!(front_door_url.host_str(), Some("fd-custom-def456.b02.azurefd.net"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_front_door_anonymous() {
        let secrets = Secrets(HashMap::from([(
            "https://anonaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                front_door_url: Some("fd-anon-ghi789.c03.azurefd.net".to_string()),
                ..Default::default()
            },
        )]));
        let address =
            Address::parse_blob(Url::parse("https://anonaccount.blob.core.windows.net/public/file.txt").unwrap())
                .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "anonaccount");
        assert!(matches!(
            config.credentials.0.read().await.deref(),
            StorageCredentialsInner::Anonymous
        ));
        assert!(config.anonymous);
        // Front door URL should still be returned even for anonymous access
        let front_door_url = config.front_door_url.expect("front_door_url should be set");
        assert_eq!(front_door_url.host_str(), Some("fd-anon-ghi789.c03.azurefd.net"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_service_principal() {
        let secrets = Secrets(HashMap::from([(
            "https://examplesasaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                tenant_id: Some("tenant-123".to_string()),
                client_id: Some("client-456".to_string()),
                client_secret: Some("secret-789".to_string()),
                ..Default::default()
            },
        )]));
        let address = Address::parse_blob(
            Url::parse("https://examplesasaccount.blob.core.windows.net/container/blob.txt").unwrap(),
        )
        .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "examplesasaccount");
        assert!(matches!(
            config.credentials.0.read().await.deref(),
            StorageCredentialsInner::TokenCredential(_)
        ));
        assert!(!config.anonymous);
        assert!(config.use_user_delegation);
        assert!(config.front_door_url.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_service_principal_with_key_fallback() {
        // When both SP and key are present, SP takes precedence
        let secrets = Secrets(HashMap::from([(
            "https://examplesasaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("should_not_be_used".to_string()),
                tenant_id: Some("tenant-123".to_string()),
                client_id: Some("client-456".to_string()),
                client_secret: Some("secret-789".to_string()),
                ..Default::default()
            },
        )]));

        let url = Url::parse("https://examplesasaccount.blob.core.windows.net/container/blob.txt").unwrap();
        let config = Storage::resolve_client_config(&url, &Some(secrets), &test_http_client(), true).unwrap();
        assert!(matches!(
            config.credentials.0.read().await.deref(),
            StorageCredentialsInner::TokenCredential(_)
        ));
        assert!(config.use_user_delegation);
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_partial_sp_falls_back_to_key() {
        // If only some SP fields are present, fall back to storage key
        let secrets = Secrets(HashMap::from([(
            "https://examplesasaccount.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("access_key".to_string()),
                tenant_id: Some("tenant-123".to_string()),
                // missing client_id and client_secret
                ..Default::default()
            },
        )]));

        let url = Url::parse("https://examplesasaccount.blob.core.windows.net/container/blob.txt").unwrap();
        let config = Storage::resolve_client_config(&url, &Some(secrets), &test_http_client(), true).unwrap();
        assert!(
            matches!(config.credentials.0.read().await.deref(), StorageCredentialsInner::Key(k, v) if k == "examplesasaccount" && v.secret() == "access_key")
        );
        assert!(!config.use_user_delegation);
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_access_front_door_with_path_prefix() {
        let secrets = Secrets(HashMap::from([(
            "https://teststorage.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("test_access_key".to_string()),
                front_door_url: Some("https://frontdoor.example.com/storage/prefix".to_string()),
                ..Default::default()
            },
        )]));
        let address =
            Address::parse_blob(Url::parse("https://teststorage.blob.core.windows.net/container/blob.txt").unwrap())
                .unwrap();

        let config =
            Storage::resolve_client_config(&address.resource_address, &Some(secrets), &test_http_client(), true)
                .unwrap();
        assert_eq!(config.account, "teststorage");
        assert!(!config.anonymous);
        let front_door_url = config.front_door_url.expect("front_door_url should be set");
        assert_eq!(front_door_url.host_str(), Some("frontdoor.example.com"));
        assert_eq!(front_door_url.path(), "/storage/prefix");
    }

    #[test]
    fn test_azure_client_emulator() {
        let client = AzureClient::emulator();
        assert!(!client.anonymous);
        assert!(client.front_door_client.is_none());
    }

    #[test]
    fn unit_front_door_client_url_hostname_only() {
        // Create an AzureClient with a Front Door URL (hostname only)
        let front_door_url = Url::parse("https://frontdoor.azurefd.net").unwrap();
        let azure_client = AzureClient::with_front_door_url(Some(front_door_url));

        // Get blob URL through the front door client
        let front_door_client = azure_client
            .front_door_client
            .as_ref()
            .expect("front_door_client should be set");
        let blob_client = front_door_client.container_client("container").blob_client("blob.txt");
        let url = blob_client.url().unwrap();

        assert_eq!(url.host_str(), Some("frontdoor.azurefd.net"));
        assert_eq!(url.path(), "/container/blob.txt");
    }

    #[test]
    fn test_front_door_client_url_with_path_prefix() {
        // Create an AzureClient with a Front Door URL that has a path prefix
        let front_door_url = Url::parse("https://frontdoor.example.com/storage/prefix").unwrap();
        let azure_client = AzureClient::with_front_door_url(Some(front_door_url));

        // Get blob URL through the front door client
        let front_door_client = azure_client
            .front_door_client
            .as_ref()
            .expect("front_door_client should be set");
        let blob_client = front_door_client.container_client("container").blob_client("blob.txt");
        let url = blob_client.url().unwrap();

        assert_eq!(url.host_str(), Some("frontdoor.example.com"));
        // The path prefix from the Front Door URL is included
        assert_eq!(url.path(), "/storage/prefix/container/blob.txt");
    }

    #[test]
    fn test_direct_client_url_no_front_door() {
        // Create an AzureClient without a Front Door URL
        let azure_client = AzureClient::with_front_door_url(None);

        // Verify no front door client is set
        assert!(azure_client.front_door_client.is_none());

        // Get blob URL through the direct client
        let blob_client = azure_client
            .client
            .container_client("container")
            .blob_client("blob.txt");
        let url = blob_client.url().unwrap();

        // Emulator uses localhost
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.path(), "/devstoreaccount1/container/blob.txt");
    }

    /// Tests that User Delegation SAS presigned URLs are cacheable — i.e., that all URLs
    /// generated within the same ExpirationNormalizer epoch are identical, and that they
    /// change when the epoch rolls over.
    ///
    /// This mirrors the `presigned_urls` test (which tests the access key SAS path) but
    /// uses a mock TokenCredential + UserDeligationKey to test the service principal path
    /// without requiring real Azure credentials.
    #[test_log::test(tokio::test)]
    async fn presigned_urls_user_delegation() {
        use azure_core::auth::{AccessToken, Secret, TokenCredential};
        use azure_storage::shared_access_signature::service_sas::UserDeligationKey;
        use uuid::Uuid;

        // A mock TokenCredential that satisfies the SDK's credential type check in
        // user_delegation_shared_access_signature. The token value doesn't matter since
        // we're only testing SAS signature determinism, not making real Azure calls.
        #[derive(Debug)]
        struct MockTokenCredential;

        #[async_trait::async_trait]
        impl TokenCredential for MockTokenCredential {
            async fn get_token(&self, _scopes: &[&str]) -> azure_core::Result<AccessToken> {
                Ok(AccessToken::new(
                    Secret::new("mock-token"),
                    OffsetDateTime::now_utc() + time::Duration::hours(1),
                ))
            }
            async fn clear_cache(&self) -> azure_core::Result<()> {
                Ok(())
            }
        }

        let credentials =
            StorageCredentials::token_credential(Arc::new(MockTokenCredential) as Arc<dyn TokenCredential>);
        let client = ClientBuilder::new("testaccount", credentials).blob_service_client();
        let blob_client = client.container_client("container").blob_client("blob.txt");

        let expiration_normalizer = ExpirationNormalizer::new(&expiration::Config {
            presigned_expiration: Duration::from_secs(15 * 60),
        });
        let epoch = expiration_normalizer.expiration_duration;

        // Create a single delegation key whose signed_expiry covers the first epoch's SAS expiry.
        // Within an epoch, get_or_fetch_delegation_key returns the same key (same signed_expiry),
        // so all SAS tokens will be signed identically.
        let base_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut current_presigned_url = None;

        // Test across ~1 day of simulated seconds
        for i in base_time..base_time + 100000 {
            let sas_expiry = expiration_normalizer.get_expiration_with_current_time(i).end_time();
            let sas_expiry_dt = OffsetDateTime::from_unix_timestamp(sas_expiry).unwrap();

            // Build a delegation key with signed_expiry matching the epoch-aligned SAS expiry.
            // This is what get_or_fetch_delegation_key does: it requests the key with
            // signed_expiry = sas_expiry, so within the same epoch the key is identical.
            let delegation_key = UserDeligationKey {
                signed_oid: Uuid::nil(),
                signed_tid: Uuid::nil(),
                signed_start: OffsetDateTime::UNIX_EPOCH,
                signed_expiry: sas_expiry_dt,
                signed_service: "b".to_string(),
                signed_version: "2022-11-02".to_string(),
                // base64 encoded "test-delegation-key-value"
                // NOT ACTUAL KEY VALUE
                value: Secret::new("dGVzdC1kZWxlZ2F0aW9uLWtleS12YWx1ZQ=="),
            };

            let sas = blob_client
                .user_delegation_shared_access_signature(
                    BlobSasPermissions {
                        read: true,
                        ..Default::default()
                    },
                    &delegation_key,
                )
                .await
                .unwrap();
            let presigned = blob_client.generate_signed_blob_url(&sas).unwrap();

            if current_presigned_url.is_none() {
                current_presigned_url = Some(presigned);
            } else if i % epoch == 0 {
                // Epoch boundary: URL must change (new SAS expiry → new signature)
                assert_ne!(
                    *current_presigned_url.as_ref().unwrap(),
                    presigned,
                    "URL should change at epoch boundary (i={})",
                    i
                );
                current_presigned_url = Some(presigned);
            } else {
                // Within same epoch: URL must be identical (same key + same expiry)
                assert_eq!(
                    *current_presigned_url.as_ref().unwrap(),
                    presigned,
                    "URL should be stable within epoch (i={})",
                    i
                );
            }
        }
    }

    #[test]
    fn unit_azure_strict_mode_emulator_rejected() {
        let result = Storage::new(
            Config {
                emulator: true,
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("incompatible"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_strict_mode_no_secrets() {
        let url = Url::parse("https://account.blob.core.windows.net/container/blob").unwrap();
        let result = Storage::resolve_client_config(&url, &None, &test_http_client(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_strict_mode_not_in_secrets() {
        let secrets = Secrets(HashMap::from([(
            "https://other.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("key".to_string()),
                ..Default::default()
            },
        )]));
        let url = Url::parse("https://unknown.blob.core.windows.net/container/blob").unwrap();
        let result = Storage::resolve_client_config(&url, &Some(secrets), &test_http_client(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    /// Drives the real data path (`client` → `get_or_create_client`) to prove the
    /// backend forwards `self.config.allow_unconfigured_endpoints` into
    /// `resolve_client_config`; the direct-resolver tests above wouldn't catch a
    /// regression that hardcodes the flag at the forwarding site.
    #[test_log::test(tokio::test)]
    async fn unit_azure_strict_mode_client_no_secrets() {
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            None,
        )
        .unwrap();
        let address =
            Address::parse_blob(Url::parse("https://account.blob.core.windows.net/container/blob").unwrap()).unwrap();

        let result = storage.client(&address).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    #[test_log::test(tokio::test)]
    async fn test_azure_strict_mode_client_not_in_secrets() {
        let secrets = Secrets(HashMap::from([(
            "https://other.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("key".to_string()),
                ..Default::default()
            },
        )]));
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            Some(secrets),
        )
        .unwrap();
        let address =
            Address::parse_blob(Url::parse("https://unknown.blob.core.windows.net/container/blob").unwrap()).unwrap();

        let result = storage.client(&address).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("endpoint not configured"));
    }

    /// A positive strict-mode case: a configured endpoint must still resolve when
    /// `allow_unconfigured_endpoints = false`, guarding against over-rejection
    /// that would brick strict deployments.
    #[test_log::test(tokio::test)]
    async fn test_azure_strict_mode_client_configured_resolves() {
        let secrets = Secrets(HashMap::from([(
            "https://account.blob.core.windows.net".to_string(),
            ClientCredentials {
                storage_key: Some("key".to_string()),
                ..Default::default()
            },
        )]));
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            Some(secrets),
        )
        .unwrap();
        let address =
            Address::parse_blob(Url::parse("https://account.blob.core.windows.net/container/blob").unwrap()).unwrap();

        let result = storage.client(&address).await;

        assert!(
            result.is_ok(),
            "configured endpoint must resolve in strict mode: {result:?}"
        );
    }

    #[test_log::test(tokio::test)]
    async fn presigned_urls() {
        let url = Url::parse("https://exampleaccountprivate.blob.core.windows.net/unittests/storage-service-unittests/unittests/readFile/jellyfish - hevc.mkv").unwrap();
        let address = Address::parse_blob(url).unwrap();
        let azure_storage_key = match std::env::var("OMNI_STORAGE_SERVICE_AZURE_STORAGE_KEY") {
            Ok(k) => k,
            Err(_) => {
                panic!("OMNI_STORAGE_SERVICE_AZURE_STORAGE_KEY not set. Cannot test presigned urls");
            },
        };

        let account = address.account().unwrap();
        let client = ClientBuilder::new(account, StorageCredentials::access_key(account, azure_storage_key));

        let blob_client = client
            .container_client(address.container().unwrap())
            .blob_client(address.blob());

        let expiration_normalizer = ExpirationNormalizer::new(&expiration::Config {
            presigned_expiration: Duration::from_secs(15 * 60),
        });

        let expiration = ExpirationNormalizer::new(&expiration::Config {
            presigned_expiration: Duration::from_secs(15 * 60),
        });
        let epoch = expiration.expiration_duration;

        let base_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut current_presigned_url = None;
        // ~1 day worth of seconds to test
        for i in base_time..base_time + 100000 {
            let sas = blob_client
                .shared_access_signature(
                    BlobSasPermissions {
                        read: true,
                        ..Default::default()
                    },
                    OffsetDateTime::from_unix_timestamp(
                        expiration_normalizer.get_expiration_with_current_time(i).end_time(),
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            let presigned = blob_client.generate_signed_blob_url(&sas).unwrap();

            if current_presigned_url.is_none() {
                current_presigned_url = Some(presigned);
            } else {
                // Start of a new epoch, reset the expected url
                if i % epoch == 0 {
                    current_presigned_url = Some(presigned);
                } else {
                    // Second in same epoch should match
                    assert_eq!(*current_presigned_url.as_ref().unwrap(), presigned);
                }
            }
        }
    }
}
