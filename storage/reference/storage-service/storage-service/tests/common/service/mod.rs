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

use std::fmt::Display;

use bytes::Bytes;
use serde_json::{json, Value};
use tonic::transport::{Channel, Endpoint};

use crate::common::{
    clients::{
        capabilities, discovery, filefolder, fileobject, metadata, utils::grpc::connect_with_retries, versioning,
    },
    env::get_env,
    metrics,
    service::handle::ServiceHandle,
    types::Interface,
    url::UrlBuilder,
};

pub mod handle;

#[derive(Debug)]
pub struct Builder {
    az_config: Value,
    s3_config: Value,
    url_remapper_config: Value,
    url_normalization_config: Value,
    notification_config: Value,
    sqs_config: Option<Value>,
    service_bus_config: Option<Value>,
    stat_cache_config: Value,
    list_cache_config: Value,
    metadata_cache_config: Value,
    permissions_config: Value,
    synthetic_dir_events_config: Value,
    metadata_config: Option<Value>,
    s3_base_url: Option<String>,
    az_base_url: Option<String>,
    remapper_mapped_base_url: Option<String>,
    remapper_original_base_url: Option<String>,
    redis_config: Value,
}

impl Builder {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            az_config: disabled(),
            s3_config: disabled(),
            url_remapper_config: disabled(),
            url_normalization_config: disabled(),
            notification_config: disabled(),
            sqs_config: None,
            service_bus_config: None,
            stat_cache_config: disabled(),
            list_cache_config: disabled(),
            metadata_cache_config: disabled(),
            metadata_config: None,
            permissions_config: disabled(),
            // Disabled by default — tests opt in via `enable_synthetic_dir_events(...)`. Without
            // this the service's own serde default would enable the feature with a 6s flush
            // interval, which adds 6+ seconds of wall-clock to every pubsub test.
            synthetic_dir_events_config: disabled(),
            az_base_url: None,
            s3_base_url: None,
            remapper_mapped_base_url: None,
            remapper_original_base_url: None,
            redis_config: json!(null),
        }
    }

    pub fn enable_az(self, enable_metadata: bool) -> Self {
        self.enable_az_with_permissions(enable_metadata, true)
    }

    /// Enable the Azure backend, choosing whether its endpoint enforces the
    /// permission layer (`permissions_enabled = false` opts the endpoint out).
    pub fn enable_az_with_permissions(self, enable_metadata: bool, permissions_enabled: bool) -> Self {
        let metadata_backend = if enable_metadata { json!("default") } else { json!(null) };

        let env = get_env().unwrap_azure();
        Self {
            az_config: json!({
                "endpoints": {
                    env.storage_account_url(): {
                        "credentials": {
                            "storage_key": {
                                "storage_key": &env.storage_key,
                            },
                        },
                        "metadata_backend": metadata_backend,
                        "permissions_enabled": permissions_enabled,
                    },
                },
            }),
            az_base_url: Some(env.storage_container_url()),
            ..self
        }
    }

    pub fn enable_s3(self, enable_metadata: bool) -> Self {
        let metadata_backend = if enable_metadata { json!("default") } else { json!(null) };

        let env = get_env().unwrap_s3();
        Self {
            s3_config: json!({
                "endpoints": {
                    format!("http://{}:{}/{}", env.host, env.port, env.bucket): {
                        "credentials": {
                            "access_key": {
                                "access_key_id": &env.access_key_id,
                                "secret_access_key": &env.secret_access_key,
                            },
                        },
                        "endpoint_url": env.endpoint_url(),
                        "addressing_style": "path",
                        "metadata_backend": metadata_backend,
                    },
                }
            }),
            s3_base_url: Some(env.bucket_url()),
            ..self
        }
    }

    pub fn enable_url_remapper(self, mapped_url: impl Into<String>, original_url: impl Into<String>) -> Self {
        let mapped_url = mapped_url.into();
        let original_url = original_url.into();
        Self {
            url_remapper_config: json!({
                "enabled": true,
                "rewrite_rules": {
                    &mapped_url: { "target": &original_url },
                },
            }),
            remapper_mapped_base_url: Some(mapped_url),
            remapper_original_base_url: Some(original_url),
            ..self
        }
    }

    pub fn with_url_remapper_config(self, url_remapper_config: Value) -> Self {
        Self {
            url_remapper_config,
            ..self
        }
    }

    pub fn enable_az_url_remapper(self, mapped_prefix: impl Display, original_prefix: impl Display) -> Self {
        let env = get_env().unwrap_azure();
        self.enable_url_remapper(
            format!("omniverse://myhost.mydomain/{}", mapped_prefix),
            env.resource_url(original_prefix),
        )
    }

    pub fn enable_s3_url_remapper(self, mapped_prefix: impl Display, original_prefix: impl Display) -> Self {
        let env = get_env().unwrap_s3();
        self.enable_url_remapper(
            format!("omniverse://myhost.mydomain/{}", mapped_prefix),
            env.resource_url(original_prefix),
        )
    }

    pub fn enable_notification_client(
        self,
        endpoint: impl Into<String>,
        credentials: impl IntoNotificationServiceClientCredentials,
    ) -> Self {
        Self {
            notification_config: json!({
                "enabled": true,
                "endpoint": endpoint.into(),
                "credentials": credentials.into_config_json(),
            }),
            ..self
        }
    }

    pub fn enable_sqs_subscriber(self) -> Self {
        let env = get_env().unwrap_elasticmq();
        Self {
            sqs_config: Some(json!({
                "credentials": {
                    "access_key": {
                        "access_key_id": &env.access_key_id,
                        "secret_access_key": &env.secret_access_key,
                    },
                },
                "queue_url": &env.queue_url,
                "region": &env.region_name,
                "endpoint_url": &env.endpoint_url(),
                "wait_time": "1s",
                "workers": 1,
            })),
            ..self
        }
    }

    pub fn enable_service_bus_subscriber(self) -> Self {
        let env = get_env().unwrap_azure_service_bus();
        Self {
            service_bus_config: Some(json!({
                "queue_namespace": &env.queue_namespace,
                "queue_name": &env.queue_name,
                "credentials": {
                    "shared_access_policy_key": {
                        "policy_name": &env.policy_name,
                        "policy_key": &env.policy_key,
                    },
                },
                "workers": 1,
                "receive_batch_size": 20,
                "receive_max_wait_time": "1s",
            })),
            ..self
        }
    }

    pub fn enable_stat_cache(self) -> Self {
        Self {
            stat_cache_config: json!({
                "enabled": true,
                "invalidate_on_update": true,
            }),
            ..self
        }
    }

    pub fn enable_list_cache(self) -> Self {
        Self {
            list_cache_config: json!({
                "enabled": true,
                "invalidate_on_update": true,
            }),
            ..self
        }
    }

    pub fn enable_metadata_cache(self, l2_enabled: bool) -> Self {
        Self {
            metadata_cache_config: json!({
                "enabled": true,
                "l2_enabled": l2_enabled,
            }),
            ..self
        }
    }

    pub fn enable_permissions(
        self,
        backend: impl Into<String>,
        endpoint: impl Into<String>,
        include_acl_in_all_keys: bool,
    ) -> Self {
        Self {
            permissions_config: json!({
                "enabled": true,
                "client": {
                    backend.into(): {
                        "endpoint": endpoint.into(),
                    },
                },
                "include_acl_in_all_keys": include_acl_in_all_keys,
            }),
            ..self
        }
    }

    /// Forward the named incoming request headers to the permission service.
    pub fn forward_headers(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let names = names.into_iter().map(Into::into).collect::<Vec<String>>();

        self.permissions_config["forward_headers"] = json!(names);
        self
    }

    pub fn enable_url_normalization(self) -> Self {
        Self {
            url_normalization_config: json!({
                "enabled": true,
            }),
            ..self
        }
    }

    /// Enable synthetic directory events with the given flush interval. Tests should pick a
    /// short interval (~100ms) so they don't sleep for the whole production default (6s).
    pub fn enable_synthetic_dir_events(self, flush_interval: impl Into<String>) -> Self {
        Self {
            synthetic_dir_events_config: json!({
                "enabled": true,
                "flush_interval": flush_interval.into(),
                "retry": {
                    "max_attempts": 3,
                    "initial_backoff": "1ms",
                    "max_backoff": "10ms",
                },
            }),
            ..self
        }
    }

    pub fn enable_dynamodb_metadata(self) -> Self {
        let dynamodb_env = get_env().unwrap_dynamodb();
        Self {
            metadata_config: Some(json!({
                "dynamodb": {
                    "endpoint_url": dynamodb_env.endpoint_url(),
                    "credentials": {
                        "access_key": {
                            "access_key_id": dynamodb_env.access_key_id,
                            "secret_access_key": dynamodb_env.secret_access_key,
                        },
                    },
                    "region": dynamodb_env.region_name,
                    "table": dynamodb_env.table,
                },
            })),
            ..self
        }
    }

    pub fn enable_cosmosdb_metadata(self) -> Self {
        let cosmosdb_env = get_env().unwrap_cosmosdb();
        Self {
            metadata_config: Some(json!({
                "cosmosdb": {
                    "account": cosmosdb_env.account,
                    "database_id": cosmosdb_env.database_id,
                    "container_id": cosmosdb_env.container_id,
                    "credentials": {
                        "primary_key": {
                            "key": cosmosdb_env.primary_key,
                        }
                    },
                    "query_batch_size": 5,
                },
            })),
            ..self
        }
    }

    pub fn enable_postgres_metadata(self) -> Self {
        let postgres_env = get_env().unwrap_postgres();
        Self {
            metadata_config: Some(json!({
                "postgres": {
                    "host": postgres_env.host,
                    "port": postgres_env.port,
                    "credentials": {
                        "password": {
                            "user": postgres_env.user,
                            "password": postgres_env.password,
                        },
                    },
                    "dbname": postgres_env.dbname,
                    "query_batch_size": 5,
                    "connection_pool_size": 1,
                }
            })),
            ..self
        }
    }

    pub fn enable_redis(self) -> Self {
        let redis_env = get_env().unwrap_redis();
        Self {
            redis_config: json!({
                "enabled": true,
                "address": redis_env.address(),
            }),
            ..self
        }
    }

    pub fn start(self) -> Service {
        let metadata_config = if let Some(config) = self.metadata_config {
            json!({
                "default": config,
            })
        } else {
            json!(null)
        };

        let storage_events_config = {
            let mut config = serde_json::Map::new();
            if let Some(service_bus_config) = self.service_bus_config {
                config.insert(
                    "azure-service-bus-0".into(),
                    json!({
                        "azure_service_bus": service_bus_config,
                    }),
                );
            }

            if let Some(sqs_config) = self.sqs_config {
                config.insert(
                    "sqs-0".into(),
                    json!({
                        "sqs": sqs_config,
                    }),
                );
            }

            config
        };

        let config_json = json!({
            "storage": {
                "azure_blob": self.az_config,
                "s3": self.s3_config,
            },
            "storage_events": storage_events_config,
            "metadata": metadata_config,
            "url_remapper": self.url_remapper_config,
            "url_normalizer": self.url_normalization_config,
            "notification_client": self.notification_config,
            "stat_cache": self.stat_cache_config,
            "list_cache": self.list_cache_config,
            "synthetic_dir_events": self.synthetic_dir_events_config,
            "metadata_cache": self.metadata_cache_config,
            "rest": {
                "addr": "0.0.0.0",
                "port": 0,
            },
            "grpc": {
                "addr": "0.0.0.0",
                "port": 0,
            },
            "permissions": self.permissions_config,
            "redis": self.redis_config,
        })
        .to_string();

        let service = ServiceHandle::spawn(
            env!("CARGO_BIN_EXE_storage-service"),
            ["--config-json", &config_json],
            Vec::<(String, String)>::default(),
        );

        Service {
            handle: service,
            s3_base_url: self.s3_base_url,
            az_base_url: self.az_base_url,
            remapper_mapped_base_url: self.remapper_mapped_base_url,
            remapper_original_base_url: self.remapper_original_base_url,
        }
    }
}

#[derive(Debug)]
pub struct Service {
    handle: ServiceHandle,
    s3_base_url: Option<String>,
    az_base_url: Option<String>,
    remapper_mapped_base_url: Option<String>,
    remapper_original_base_url: Option<String>,
}

impl Service {
    pub fn builder() -> Builder {
        Builder::new()
    }

    pub fn start_with_config(config: Value) -> Self {
        let handle = ServiceHandle::spawn(
            env!("CARGO_BIN_EXE_storage-service"),
            ["--config-json", &config.to_string()],
            Vec::<(String, String)>::default(),
        );

        Self {
            handle,
            s3_base_url: None,
            az_base_url: None,
            remapper_mapped_base_url: None,
            remapper_original_base_url: None,
        }
    }

    pub async fn fileobject_v1alpha_client(&self, interface: Interface) -> fileobject::v1alpha::Client {
        use fileobject::{grpc::v1alpha::Client as GrpcClient, rest::v1alpha::Client as RestClient, v1alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn fileobject_v1beta_client(&self, interface: Interface) -> fileobject::v1beta::Client {
        use fileobject::{grpc::v1beta::Client as GrpcClient, rest::v1beta::Client as RestClient, v1beta::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn filefolder_v1alpha_client(&self, interface: Interface) -> filefolder::v1alpha::Client {
        use filefolder::{grpc::v1alpha::Client as GrpcClient, rest::v1alpha::Client as RestClient, v1alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn filefolder_v1beta_client(&self, interface: Interface) -> filefolder::v1beta::Client {
        use filefolder::{grpc::v1beta::Client as GrpcClient, rest::v1beta::Client as RestClient, v1beta::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn versioning_v1alpha_client(&self, interface: Interface) -> versioning::v1alpha::Client {
        use versioning::{grpc::v1alpha::Client as GrpcClient, rest::v1alpha::Client as RestClient, v1alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn versioning_v1beta_client(&self, interface: Interface) -> versioning::v1beta::Client {
        use versioning::{grpc::v1beta::Client as GrpcClient, rest::v1beta::Client as RestClient, v1beta::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn metadata_v1alpha_client(&self, interface: Interface) -> metadata::v1alpha::Client {
        use metadata::{grpc::v1alpha::Client as GrpcClient, rest::v1alpha::Client as RestClient, v1alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn metadata_v1beta_client(&self, interface: Interface) -> metadata::v1beta::Client {
        use metadata::{grpc::v1beta::Client as GrpcClient, rest::v1beta::Client as RestClient, v1beta::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn capabilities_v1alpha_client(&self, interface: Interface) -> capabilities::v1alpha::Client {
        use capabilities::{grpc::v1alpha::Client as GrpcClient, rest::v1alpha::Client as RestClient, v1alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn capabilities_v1beta_client(&self, interface: Interface) -> capabilities::v1beta::Client {
        use capabilities::{grpc::v1beta::Client as GrpcClient, rest::v1beta::Client as RestClient, v1beta::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    pub async fn discovery_v2alpha_client(&self, interface: Interface) -> discovery::v2alpha::Client {
        use discovery::{grpc::v2alpha::Client as GrpcClient, rest::v2alpha::Client as RestClient, v2alpha::Client};

        match interface {
            Interface::Rest => Client::Rest(RestClient::new(Default::default(), self.handle.rest_endpoint())),
            Interface::Grpc => Client::Grpc(GrpcClient::new(self.create_grpc_client().await)),
        }
    }

    /// The base URL of the REST ingress, for tests that issue raw requests (e.g. checking an
    /// advertised path resolves).
    pub fn rest_endpoint(&self) -> String {
        self.handle.rest_endpoint()
    }

    /// A connected gRPC channel to the service, for tests that construct their own
    /// service client (e.g. the top-level metadata `MetadataServiceClient`).
    pub async fn grpc_channel(&self) -> Channel {
        self.create_grpc_client().await
    }

    pub fn metrics_client(&self) -> metrics::Client {
        metrics::Client::new(self.handle.metrics_endpoint())
    }

    pub fn s3_url_builder(&self) -> UrlBuilder {
        self.s3_base_url
            .as_ref()
            .map(UrlBuilder::new)
            .expect("s3 backend is disabled")
    }

    pub fn az_url_builder(&self) -> UrlBuilder {
        self.az_base_url
            .as_ref()
            .map(UrlBuilder::new)
            .expect("azure backend is disabled")
    }

    pub fn remapper_mapped_url_builder(&self) -> UrlBuilder {
        self.remapper_mapped_base_url
            .as_ref()
            .map(UrlBuilder::new)
            .expect("url remapper backend is disabled")
    }

    pub fn remapper_original_url_builder(&self) -> UrlBuilder {
        self.remapper_original_base_url
            .as_ref()
            .map(UrlBuilder::new)
            .expect("url remapper backend is disabled")
    }

    pub fn filefolder_v1alpha_rest_client(&self) -> filefolder::rest::v1alpha::Client {
        filefolder::rest::v1alpha::Client::new(Default::default(), self.handle.rest_endpoint())
    }

    pub fn filefolder_v1beta_rest_client(&self) -> filefolder::rest::v1beta::Client {
        filefolder::rest::v1beta::Client::new(Default::default(), self.handle.rest_endpoint())
    }

    /// The REST fileobject client, for tests that need the per-page `enumerate_once` entry point:
    /// a continuation handle is a REST-only wire concept, so the interface-agnostic client cannot
    /// carry one.
    pub fn fileobject_v1alpha_rest_client(&self) -> fileobject::rest::v1alpha::Client {
        fileobject::rest::v1alpha::Client::new(Default::default(), self.handle.rest_endpoint())
    }

    pub fn fileobject_v1beta_rest_client(&self) -> fileobject::rest::v1beta::Client {
        fileobject::rest::v1beta::Client::new(Default::default(), self.handle.rest_endpoint())
    }

    async fn create_grpc_client(&self) -> Channel {
        let endpoint = Bytes::from(self.handle.grpc_endpoint().into_bytes());
        connect_with_retries(move || {
            let endpoint = Endpoint::from_shared(endpoint.clone()).expect("grpc endpoint parse failed");
            async move { endpoint.connect().await }
        })
        .await
    }
}

pub trait IntoNotificationServiceClientCredentials {
    fn into_config_json(self) -> Value;
}

pub struct EmptyCredentials;

impl IntoNotificationServiceClientCredentials for EmptyCredentials {
    fn into_config_json(self) -> Value {
        Value::Null
    }
}

pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub config_endpoint: String,
}

impl ClientCredentials {
    pub fn new(
        config_endpoint: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            scope: scope.into(),
            config_endpoint: config_endpoint.into(),
        }
    }
}

impl IntoNotificationServiceClientCredentials for ClientCredentials {
    fn into_config_json(self) -> Value {
        json!({
            "oauth2_client_credentials": {
                "openid_configuration_uri": self.config_endpoint,
                "parameters": [
                    {
                        "name": "client_id",
                        "value": self.client_id,
                    },
                    {
                        "name": "client_secret",
                        "value": self.client_secret,
                    },
                    {
                        "name": "scope",
                        "value": self.scope,
                    },
                ],
            },
        })
    }
}

fn empty() -> Value {
    Value::Object(Default::default())
}

fn null() -> Value {
    Value::Null
}

fn disabled() -> Value {
    json!({"enabled": false})
}
