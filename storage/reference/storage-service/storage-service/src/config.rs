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

use std::{
    collections::HashMap,
    net::{AddrParseError, IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use serde::{de::Error, Deserialize};
use storage_core::{
    additional_headers, azure, egress_throttle, list_cache, manifest, s3, small_object_cache, stat_cache,
};
use storage_core_util::cache::RedisConfig as RedisCoreConfig;
use url::Url;

use crate::common::{endpoint::EndpointUrl, secret::Secret};
use crate::{
    backend::metadata, backend::permissions::PermissionsConfig, backend::url_normalizer::UrlNormalizerConfig,
    common::url_remapper::UrlRemapperConfig, pubsub::notification,
    pubsub::synthetic_dir_events::SyntheticDirEventsConfig,
};

pub(crate) fn default_true() -> bool {
    true
}

const REDIS_DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_millis(250);
const REDIS_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REDIS_DEFAULT_BROADCAST_CHANNEL_CAPACITY: usize = 1024;
const REDIS_DEFAULT_POOL_SIZE: usize = 8;
const REDIS_DEFAULT_RECONNECT_MAX_ATTEMPTS: u32 = 0;
const REDIS_DEFAULT_RECONNECT_MIN_DELAY: Duration = Duration::from_millis(100);
const REDIS_DEFAULT_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
const REDIS_DEFAULT_RECONNECT_BASE: u32 = 2;
const REDIS_DEFAULT_RECONNECT_JITTER: Duration = Duration::from_millis(50);

/// Service configuration.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub rest: RestConfig,
    pub grpc: GrpcConfig,
    pub grpc_probe_service: GrpcProbeServiceConfig,
    pub storage: StorageConfig,
    pub storage_events: Option<HashMap<String, StorageEventSubscriberConfig>>,
    pub metadata: Option<HashMap<String, MetadataBackendConfig>>,
    pub url_remapper: UrlRemapperConfig,
    pub url_normalizer: UrlNormalizerConfig,
    pub small_object_cache: small_object_cache::Config,
    pub additional_headers: additional_headers::Config,
    pub ingress_throttle: IngressThrottleConfig,
    pub ingress_load_shed: IngressLoadShedConfig,
    pub egress_throttle: egress_throttle::Config,
    pub stat_cache: stat_cache::Config,
    pub metadata_cache: metadata::cache::config::Config,
    pub notification_client: NotificationServiceClientConfig,
    pub permissions: PermissionsConfig,
    pub list_cache: list_cache::Config,
    pub synthetic_dir_events: SyntheticDirEventsConfig,
    pub manifest: manifest::Config,
    pub redis: Option<RedisServiceConfig>,
    pub metrics: MetricsConfig,
}

/// `[metrics]` configuration section.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct MetricsConfig {
    /// Optional override for the metric name prefix prepended to every metric
    /// the service emits. When `None`, the default `"storage"` prefix is used
    /// (matches the historical behaviour and the prefix the existing Grafana
    /// dashboards expect). Set to `Some("")` to emit bare names.
    pub prefix: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct IngressThrottleConfig {
    pub enabled: bool,
    pub concurrency_limit: usize,
}

impl Default for IngressThrottleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            concurrency_limit: 8192,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default)]
#[derive(Default)]
pub struct IngressLoadShedConfig {
    pub enabled: bool,
}

/// `[redis]` configuration section. Mirrors
/// [`storage_core_util::cache::RedisConfig`] but takes duration strings for
/// the timeouts (matching the rest of this codebase's config style). Only
/// `address` is required; the other fields are optional and inherit the
/// upstream defaults when omitted.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct RedisServiceConfig {
    pub address: String,
    pub namespace: Option<String>,
    #[serde(with = "humantime_serde::option")]
    pub command_timeout: Option<Duration>,
    #[serde(with = "humantime_serde::option")]
    pub connect_timeout: Option<Duration>,
    pub broadcast_channel_capacity: Option<usize>,
    pub pool_size: Option<usize>,
    pub reconnect_max_attempts: Option<u32>,
    #[serde(with = "humantime_serde::option")]
    pub reconnect_min_delay: Option<Duration>,
    #[serde(with = "humantime_serde::option")]
    pub reconnect_max_delay: Option<Duration>,
    pub reconnect_base: Option<u32>,
    #[serde(with = "humantime_serde::option")]
    pub reconnect_jitter: Option<Duration>,
}

impl From<RedisServiceConfig> for RedisCoreConfig {
    fn from(value: RedisServiceConfig) -> Self {
        Self {
            address: value.address,
            namespace: value.namespace,
            command_timeout_ms: value
                .command_timeout
                .unwrap_or(REDIS_DEFAULT_COMMAND_TIMEOUT)
                .as_millis() as u64,
            connect_timeout_ms: value
                .connect_timeout
                .unwrap_or(REDIS_DEFAULT_CONNECT_TIMEOUT)
                .as_millis() as u64,
            pool_size: value.pool_size.unwrap_or(REDIS_DEFAULT_POOL_SIZE),
            broadcast_channel_capacity: value
                .broadcast_channel_capacity
                .unwrap_or(REDIS_DEFAULT_BROADCAST_CHANNEL_CAPACITY),
            reconnect_max_attempts: value
                .reconnect_max_attempts
                .unwrap_or(REDIS_DEFAULT_RECONNECT_MAX_ATTEMPTS),
            reconnect_min_delay_ms: value
                .reconnect_min_delay
                .unwrap_or(REDIS_DEFAULT_RECONNECT_MIN_DELAY)
                .as_millis() as u32,
            reconnect_max_delay_ms: value
                .reconnect_max_delay
                .unwrap_or(REDIS_DEFAULT_RECONNECT_MAX_DELAY)
                .as_millis() as u32,
            reconnect_base: value.reconnect_base.unwrap_or(REDIS_DEFAULT_RECONNECT_BASE),
            reconnect_jitter_ms: value
                .reconnect_jitter
                .unwrap_or(REDIS_DEFAULT_RECONNECT_JITTER)
                .as_millis() as u32,
        }
    }
}

#[derive(Deserialize, Debug, Hash, Eq, PartialEq, Clone)]
pub struct AzureClientSecretCredentialsConfig {
    pub client_id: String,
    pub client_secret: Secret<String>,
    pub tenant_id: String,
    #[serde(default)]
    pub authority_host: Option<Url>,
}

#[derive(Deserialize, Debug, Hash, Eq, PartialEq, Clone)]
pub struct AzureFederatedCredentialsConfig {
    pub client_id: String,
    pub tenant_id: String,
    pub federated_token_file: PathBuf,
    #[serde(default)]
    pub authority_host: Option<Url>,
}

#[derive(Deserialize, Debug, Hash, Eq, PartialEq, Clone)]
pub struct AzureTokenFileCredentialsConfig {
    pub path: PathBuf,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AzureStorageKeyCredentialsConfig {
    pub storage_key: Secret<String>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct AzureMaybeAnonStorageCredentialsConfig {
    pub anonymous: bool,
    pub client_secret: Option<AzureClientSecretCredentialsConfig>,
    pub storage_key: Option<AzureStorageKeyCredentialsConfig>,
    pub federated: Option<AzureFederatedCredentialsConfig>,
    pub access_token_file: Option<AzureTokenFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct AzureStorageCredentialsConfig {
    pub client_secret: Option<AzureClientSecretCredentialsConfig>,
    pub storage_key: Option<AzureStorageKeyCredentialsConfig>,
    pub federated: Option<AzureFederatedCredentialsConfig>,
    pub access_token_file: Option<AzureTokenFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AzureCosmosDBPrimaryKeyCredentialsConfig {
    pub key: Secret<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct AzureCosmosDBCredentialsConfig {
    pub client_secret: Option<AzureClientSecretCredentialsConfig>,
    pub primary_key: Option<AzureCosmosDBPrimaryKeyCredentialsConfig>,
    pub federated: Option<AzureFederatedCredentialsConfig>,
    pub access_token_file: Option<AzureTokenFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AzureSASPolicyCredentialsConfig {
    pub policy_name: String,
    pub policy_key: Secret<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct AzureServiceBusCredentialsConfig {
    pub client_secret: Option<AzureClientSecretCredentialsConfig>,
    pub shared_access_policy_key: Option<AzureSASPolicyCredentialsConfig>,
    pub federated: Option<AzureFederatedCredentialsConfig>,
    pub access_token_file: Option<AzureTokenFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Eq, PartialEq, Hash, Clone)]
pub struct AWSAccessKeyCredentialsConfig {
    pub access_key_id: String,
    pub secret_access_key: Secret<String>,
    #[serde(default)]
    pub session_token: Option<String>,
}

#[derive(Deserialize, Debug, Eq, PartialEq, Hash, Clone)]
pub struct AWSWebIdentityTokenCredentialsConfig {
    pub web_identity_token_file: PathBuf,
    pub role_arn: String,
    #[serde(default)]
    pub role_session_name: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AWSAssumeRoleCredentialsConfig {
    pub role_arn: String,
    pub external_id: Option<String>,
    pub session_name: Option<String>,
    #[serde(default, with = "humantime_serde")]
    pub session_length: Option<Duration>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AWSCredentialsFileCredentialsConfig {
    pub path: PathBuf,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct AWSMaybeAnonCredentialsConfig {
    pub anonymous: bool,
    pub access_key: Option<AWSAccessKeyCredentialsConfig>,
    pub assume_role: Option<AWSAssumeRoleCredentialsConfig>,
    pub web_identity_token: Option<AWSWebIdentityTokenCredentialsConfig>,
    pub credentials_file: Option<AWSCredentialsFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct AWSCredentialsConfig {
    pub access_key: Option<AWSAccessKeyCredentialsConfig>,
    pub assume_role: Option<AWSAssumeRoleCredentialsConfig>,
    pub web_identity_token: Option<AWSWebIdentityTokenCredentialsConfig>,
    pub credentials_file: Option<AWSCredentialsFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct S3Config {
    #[serde(flatten)]
    pub client_config: s3::Config,
    pub endpoints: HashMap<S3EndpointUrl, S3EndpointConfig>,
}

/// Storage backend configuration.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct StorageConfig {
    pub azure_blob: AzureBlobConfig,
    pub s3: S3Config,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SqsSubscriberConfig {
    #[serde(flatten)]
    pub client_config: crate::pubsub::sqs::Config,
    pub credentials: AWSCredentialsConfig,
}

/// Storage event configuration.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct StorageEventSubscriberConfig {
    pub azure_service_bus: Option<AzureServiceBusSubscriberConfig>,
    pub sqs: Option<SqsSubscriberConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AzureServiceBusSubscriberConfig {
    #[serde(flatten)]
    pub client_config: crate::pubsub::azure_service_bus::AzureServiceBusSubscriberConfig,
    pub credentials: AzureServiceBusCredentialsConfig,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct MetadataBackendConfig {
    #[cfg(feature = "cosmosdb")]
    pub cosmosdb: Option<CosmosDBMetadataBackendConfig>,
    #[cfg(feature = "dynamodb")]
    pub dynamodb: Option<DynamoDBMetadataBackendConfig>,
    pub postgres: Option<PostgresMetadataBackendConfig>,
}

#[cfg(feature = "cosmosdb")]
#[derive(Deserialize, Debug, Clone)]
pub struct CosmosDBMetadataBackendConfig {
    #[serde(flatten)]
    pub client_config: metadata::azure_cosmosdb::config::Config,
    pub credentials: AzureCosmosDBCredentialsConfig,
}

#[cfg(feature = "dynamodb")]
#[derive(Deserialize, Debug, Clone)]
pub struct DynamoDBMetadataBackendConfig {
    #[serde(flatten)]
    pub client_config: metadata::dynamodb::Config,
    pub credentials: AWSCredentialsConfig,
}

/// Azure Blob backend configuration.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct AzureBlobConfig {
    #[serde(flatten)]
    pub client_config: azure::Config,
    pub endpoints: HashMap<AzureBlobEndpointUrl, AzureBlobEndpointConfig>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct PostgresCredentialsConfig {
    pub password: Option<PostgresPasswordCredentialsConfig>,
    pub access_token_file: Option<PostgresAccessTokenFileCredentialsConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PostgresPasswordCredentialsConfig {
    pub user: String,
    pub password: Secret<String>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub struct PostgresAccessTokenFileCredentialsConfig {
    pub user: String,
    pub path: PathBuf,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PostgresMetadataBackendConfig {
    #[serde(flatten)]
    pub client_config: metadata::postgres::config::Config,
    pub credentials: PostgresCredentialsConfig,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct NotificationServiceClientConfig {
    #[serde(flatten)]
    pub client_config: notification::ClientConfig,
    pub credentials: Option<notification::ClientCredentialsConfig>,
}

/// An Azure Blob account endpoint URL.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct AzureBlobEndpointUrl(pub EndpointUrl);

impl<'de> Deserialize<'de> for AzureBlobEndpointUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let endpoint_url_string = String::deserialize(deserializer)?;
        EndpointUrl::parse_azure_blob_endpoint_url(&endpoint_url_string)
            .map_err(|source| {
                Error::custom(format!(
                    "invalid azure endpoint url {:?}: {}",
                    endpoint_url_string, source
                ))
            })
            .map(Self)
    }
}

impl AsRef<EndpointUrl> for AzureBlobEndpointUrl {
    fn as_ref(&self) -> &EndpointUrl {
        &self.0
    }
}

/// Azure Blob endpoint-specific configuration.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AzureBlobEndpointConfig {
    pub storage_account: Option<String>,
    pub versioning_enabled: Option<bool>,
    pub read_only: Option<bool>,
    pub credentials: AzureMaybeAnonStorageCredentialsConfig,
    pub front_door: Option<AzureFrontDoorConfig>,
    pub metadata_backend: Option<String>,
    pub permissions_enabled: bool,
    /// Whether this backend is listed in `ListTopLevelAddresses`. Defaults to true.
    pub list_in_top_level_addresses: bool,
    /// Whether this backend is listed in `ListRoutes` (and, being a superset, in
    /// `ListTopLevelAddresses`). Defaults to true. A hidden backend stays fully
    /// configured but is reachable only through a url-remapper alias.
    pub list_in_routes: bool,
}

impl Default for AzureBlobEndpointConfig {
    fn default() -> Self {
        Self {
            storage_account: None,
            versioning_enabled: None,
            read_only: None,
            credentials: Default::default(),
            front_door: None,
            metadata_backend: None,
            permissions_enabled: true,
            list_in_top_level_addresses: true,
            list_in_routes: true,
        }
    }
}

/// Azure Blob FrontDoor CDN configuration.
#[derive(Deserialize, Debug, Clone)]
pub struct AzureFrontDoorConfig {
    pub url: String,
}

/// An S3 bucket endpoint URL;
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct S3EndpointUrl(pub EndpointUrl);

impl<'de> Deserialize<'de> for S3EndpointUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let endpoint_url_string = String::deserialize(deserializer)?;
        EndpointUrl::parse_s3_endpoint_url(&endpoint_url_string)
            .map_err(|source| Error::custom(format!("invalid s3 endpoint url {:?}: {}", endpoint_url_string, source)))
            .map(Self)
    }
}

impl AsRef<EndpointUrl> for S3EndpointUrl {
    fn as_ref(&self) -> &EndpointUrl {
        &self.0
    }
}

/// S3 endpoint-specific configuration.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct S3EndpointConfig {
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub endpoint_url: Option<String>,
    pub addressing_style: Option<s3::resource::AddressingStyle>,
    pub versioning_enabled: Option<bool>,
    pub read_only: Option<bool>,
    pub credentials: AWSMaybeAnonCredentialsConfig,
    pub cloudfront: Option<CloudfrontConfig>,
    pub metadata_backend: Option<String>,
    pub permissions_enabled: bool,
    /// Whether this backend is listed in `ListTopLevelAddresses`. Defaults to true.
    pub list_in_top_level_addresses: bool,
    /// Whether this backend is listed in `ListRoutes` (and, being a superset, in
    /// `ListTopLevelAddresses`). Defaults to true. A hidden backend stays fully
    /// configured but is reachable only through a url-remapper alias.
    pub list_in_routes: bool,
}

impl Default for S3EndpointConfig {
    fn default() -> Self {
        Self {
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            versioning_enabled: None,
            read_only: None,
            credentials: Default::default(),
            cloudfront: None,
            metadata_backend: None,
            permissions_enabled: true,
            list_in_top_level_addresses: true,
            list_in_routes: true,
        }
    }
}

/// S3 CloudFront CDN configuration.
#[derive(Deserialize, Debug, Clone)]
pub struct CloudfrontConfig {
    pub distribution_url: Url,
    pub key_pair_id: Option<String>,
    pub private_key_pem: Option<Secret<String>>,
}

/// REST service configuration.
#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct RestConfig {
    pub enabled: bool,
    pub addr: String,
    pub port: u16,
    /// Serve the OpenAPI documents. The Swagger UI renders from them and goes with them.
    pub serve_openapi: bool,
}

impl RestConfig {
    pub fn listen_addr(&self) -> Result<Option<SocketAddr>, AddrParseError> {
        if self.enabled {
            get_listen_addr(&self.addr, self.port).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            addr: "0.0.0.0".into(),
            port: 8011,
            serve_openapi: true,
        }
    }
}

/// GRPC service configuration.
#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct GrpcConfig {
    pub enabled: bool,
    pub addr: String,
    pub port: u16,
}

impl GrpcConfig {
    pub fn listen_addr(&self) -> Result<Option<SocketAddr>, AddrParseError> {
        if self.enabled {
            get_listen_addr(&self.addr, self.port).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            addr: "0.0.0.0".into(),
            port: 8012,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct GrpcProbeServiceConfig {
    pub enabled: bool,
    pub addr: String,
    pub port: u16,
}

impl GrpcProbeServiceConfig {
    pub fn listen_addr(&self) -> Result<Option<SocketAddr>, AddrParseError> {
        if self.enabled {
            get_listen_addr(&self.addr, self.port).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Default for GrpcProbeServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            addr: "0.0.0.0".into(),
            port: 8100,
        }
    }
}

/// Create a socket address from the host and port components.
fn get_listen_addr(addr: &str, port: u16) -> Result<SocketAddr, AddrParseError> {
    Ok(SocketAddr::new(IpAddr::V4(FromStr::from_str(addr)?), port))
}
