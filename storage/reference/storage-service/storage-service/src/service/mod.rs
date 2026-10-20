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

use std::{collections::HashMap, future::pending, sync::Arc};

use aws_sdk_s3::config::ProvideCredentials;
use azure_core::auth::TokenCredential;
use futures::{future::BoxFuture, FutureExt};
use storage_core::{
    additional_headers::AdditionalHeadersLayer,
    azure,
    egress_throttle::EgressThrottleLayer,
    layer::LayerArc,
    list_cache::ListCacheLayer,
    manifest::ManifestLayer,
    metrics::{OperationMetricsLayer, SdkMetricsLayer},
    s3,
    small_object_cache::SmallObjectCacheLayer,
    stat_cache::StatCacheLayer,
    storage::{StorageService, StorageServiceServer},
};
use tokio::select;
use tracing::{debug, info, warn};

use crate::{
    api::{discovery, metadata, metadata_api},
    backend::{
        az_blob_name_rejecter::AzureBlobNameRejecter,
        builder::Skip,
        created_timestamp::CreatedTimestampLayer,
        metadata::{
            any::AnyMetadata, cache::MetadataCache, canonicalization::UriCanonicalization,
            metrics::MetadataMetricsLayer,
        },
        permissions::{filter::PermissionRequestFilter, PermissionsLayer},
        request_limits::RequestLimitsLayer,
        router::{
            self,
            selector::{RegexSelector, RegexSelectorRoute},
        },
        trace_error::TraceErrorLayer,
        url_normalizer::UrlNormalizerLayer,
        url_remapper::{service::UrlRemapperLayer, visibility::ArcAddressFilter},
    },
    common::url_remapper::{RootListing, UrlRemapper},
    config::{
        AWSAccessKeyCredentialsConfig, AWSAssumeRoleCredentialsConfig, AWSCredentialsConfig,
        AWSCredentialsFileCredentialsConfig, AWSMaybeAnonCredentialsConfig, AWSWebIdentityTokenCredentialsConfig,
        AzureClientSecretCredentialsConfig, AzureFederatedCredentialsConfig, AzureMaybeAnonStorageCredentialsConfig,
        AzureServiceBusCredentialsConfig, AzureTokenFileCredentialsConfig, Config, MetadataBackendConfig,
        PostgresCredentialsConfig,
    },
    pubsub::{
        azure::identity::AzureIdentityResolver, azure_service_bus::AzureServiceBusSubscriber,
        folder_sentinel_filter::FolderSentinelFilterLayer, notification::client::NotificationServiceClient,
        sqs::SqsSubscriber, synthetic_dir_events::SyntheticDirEvents, types::AnyConsumer,
    },
};

mod error;
mod probe;

pub use error::Error;

pub struct Service {
    config: Config,
    bootstrap_only: bool,
}

impl Service {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            bootstrap_only: false,
        }
    }

    pub fn bootstrap_only(mut self, bootstrap_only: bool) -> Self {
        self.bootstrap_only = bootstrap_only;
        self
    }

    pub async fn run(self) -> Result<(), Error> {
        let futures = Context::new(self.config)
            .build()
            .await
            .map_err(|source| Error::new_with_source("failed to build the service dependency context", source))?;
        if self.bootstrap_only {
            info!("bootstrap complete, exiting without serving");
            return Ok(());
        }

        select! {
            service_result = futures.server_future => service_result.map_err(|source| {
                Error::new_with_source("storage service servers failed", source)
            })?,
            probe_service_result = futures.probe_service_future => probe_service_result.map_err(|source| {
                Error::new_with_source("grpc probe service failed", source)
            })?,
            _ = futures.consumers_future => {},
        };

        Ok(())
    }
}

type StorageRouter = router::Service<
    router::ServiceRoute<Arc<s3::Storage>, RegexSelector>,
    router::Service<router::ServiceRoute<Arc<azure::Storage>, RegexSelector>, router::Dummy>,
>;

#[derive(Default)]
enum Slot<T> {
    #[default]
    Uninitialized,
    Initialized(T),
}

struct RuntimeFutures {
    server_future: BoxFuture<'static, Result<(), Error>>,
    consumers_future: BoxFuture<'static, ()>,
    probe_service_future: BoxFuture<'static, Result<(), Error>>,
}

#[derive(Clone)]
struct AzureBackend {
    storage: Arc<azure::Storage>,
    selector: RegexSelector,
}

#[derive(Clone)]
struct S3Backend {
    storage: Arc<s3::Storage>,
    selector: RegexSelector,
}

#[derive(Default)]
struct Context {
    config: Arc<Config>,
    azure_credentials_factory: AzureCredentialsFactory,
    aws_credentials_factory: AWSCredentialsFactory,
    metadata_backend_instances: HashMap<String, AnyMetadata>,
    azure_backend: Slot<AzureBackend>,
    s3_backend: Slot<S3Backend>,
    storage_router: Slot<Arc<StorageRouter>>,
    metadata_backend: Slot<Option<AnyMetadata>>,
    redis: Slot<(Option<fred::clients::Pool>, Option<String>)>,
    stat_cache_layer: Slot<Option<LayerArc<StatCacheLayer>>>,
    list_cache_layer: Slot<Option<LayerArc<ListCacheLayer>>>,
    small_object_cache_layer: Slot<Option<LayerArc<SmallObjectCacheLayer>>>,
    manifest_layer: Slot<Option<ManifestLayer>>,
    egress_throttle_layer: Slot<Option<LayerArc<EgressThrottleLayer>>>,
    additional_headers_layer: Slot<Option<LayerArc<AdditionalHeadersLayer>>>,
    operation_metrics_layer: Slot<LayerArc<OperationMetricsLayer>>,
    sdk_metrics_layer: Slot<LayerArc<SdkMetricsLayer>>,
    permissions_layer: Slot<Option<PermissionsLayer>>,
    azure_blob_name_rejecter_layer: Slot<AzureBlobNameRejecter>,
    metadata_cache_layer: Slot<Option<LayerArc<MetadataCache>>>,
    url_remapper_layer: Slot<Option<UrlRemapperLayer<AnyMetadata>>>,
    url_normalizer_layer: Slot<Option<UrlNormalizerLayer>>,
    created_timestamp_layer: Slot<Option<LayerArc<CreatedTimestampLayer>>>,
}

impl Context {
    fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            ..Default::default()
        }
    }

    async fn build(mut self) -> Result<RuntimeFutures, Error> {
        let server_future = self
            .build_server_future()
            .await
            .map_err(|source| Error::new_with_source("failed to build the server future", source))?;
        let consumers_future = self
            .build_event_consumers_future()
            .await
            .map_err(|source| Error::new_with_source("failed to build the event consumers future", source))?;
        let probe_service_future = self
            .build_probe_service_future()
            .map_err(|source| Error::new_with_source("failed to build the grpc probe service future", source))?;

        Ok(RuntimeFutures {
            server_future,
            consumers_future,
            probe_service_future,
        })
    }

    async fn build_server_future(&mut self) -> Result<BoxFuture<'static, Result<(), Error>>, Error> {
        let maybe_ingress_throttle = if self.config.ingress_throttle.enabled {
            let layer = tower::limit::GlobalConcurrencyLimitLayer::new(self.config.ingress_throttle.concurrency_limit);
            tower::util::option_layer(Some(layer))
        } else {
            tower::util::option_layer(None)
        };

        let maybe_ingress_load_shed_rest = if self.config.ingress_load_shed.enabled {
            let layer = crate::common::axum::LoadShedLayer;
            tower::util::option_layer(Some(layer))
        } else {
            tower::util::option_layer(None)
        };

        let service_builder = StorageService::builder().customize_grpc_server(|service| {
            service
                .load_shed(self.config.ingress_load_shed.enabled)
                .layer(maybe_ingress_throttle.clone())
        });

        let service_builder = if self.config.rest.serve_openapi {
            service_builder
        } else {
            service_builder.without_swagger_ui()
        };

        let service_builder = self
            .register_discovery_endpoints(service_builder)
            .await
            .map_err(|source| Error::new_with_source("failed to register discovery endpoints", source))?;
        let service_builder = self
            .register_v1alpha_endpoints(service_builder)
            .await
            .map_err(|source| Error::new_with_source("failed to register v1alpha endpoints", source))?;
        let service_builder = self
            .register_v1beta_endpoints(service_builder)
            .await
            .map_err(|source| Error::new_with_source("failed to register v1beta endpoints", source))?;
        let service_builder = self
            .register_metadata_endpoints(service_builder)
            .await
            .map_err(|source| Error::new_with_source("failed to register metadata endpoints", source))?;

        let service_builder = service_builder.customize_rest_server(|service| {
            service
                .layer(maybe_ingress_throttle)
                .layer(maybe_ingress_load_shed_rest)
        });

        let grpc_listen_addr = self
            .config
            .grpc
            .listen_addr()
            .map_err(|source| Error::new_with_source("grpc listen address is invalid", source))?;
        let rest_listen_addr = self
            .config
            .rest
            .listen_addr()
            .map_err(|source| Error::new_with_source("rest listen address is invalid", source))?;

        Ok(Box::pin(async move {
            service_builder
                .serve(grpc_listen_addr, rest_listen_addr)
                .await
                .map_err(|source| Error::new_with_source("failed to run the storage service servers", source))
        }))
    }

    async fn build_event_consumers_future(&mut self) -> Result<BoxFuture<'static, ()>, Error> {
        let event_subscribers = self.config.storage_events.clone().unwrap_or_default();
        if event_subscribers.is_empty() {
            info!("no event subscribers are configured");
            return Ok(Box::pin(pending()));
        }

        let notification_client = NotificationServiceClient::try_from_config(
            self.config.notification_client.client_config.clone(),
            self.config.notification_client.credentials.clone(),
        )
        .await
        .map_err(|source| Error::new_with_source("failed to configure the notification service client", source))?;

        let synthetic_dir_events = SyntheticDirEvents::from_config(self.config.synthetic_dir_events.clone());
        if synthetic_dir_events.is_some() && notification_client.is_none() {
            warn!(
                "synthetic directory events are enabled, but the notification client is disabled; \
                 synthetic directory events will not be published"
            );
        }

        let mut event_consumer = notification_client
            .map(AnyConsumer::from_consumer)
            .map(|consumer| consumer.decorate(FolderSentinelFilterLayer));

        if let Some(layer) = self.get_or_create_stat_cache_layer().await? {
            event_consumer = Some(event_consumer.unwrap_or(AnyConsumer::noop()).decorate(layer));
        }

        if let Some(layer) = self.get_or_create_list_cache_layer().await? {
            event_consumer = Some(event_consumer.unwrap_or(AnyConsumer::noop()).decorate(layer));
        }

        event_consumer = event_consumer.map(|consumer| consumer.maybe_decorate(synthetic_dir_events));

        let Some(event_consumer) = event_consumer else {
            return Ok(Box::pin(pending()));
        };

        let url_remapper_layer = self.get_or_create_url_remapper_layer().await?;
        let event_consumer = event_consumer.maybe_decorate(url_remapper_layer);

        info!("configuring event subscribers");

        let mut futures: Vec<BoxFuture<'static, ()>> = Vec::with_capacity(event_subscribers.len());
        for (subscriber_id, subscriber_config) in event_subscribers {
            info!(?subscriber_id, "configuring event subscriber");

            if let Some(config) = subscriber_config.azure_service_bus {
                let credentials = self
                    .azure_credentials_factory
                    .try_create_service_bus_credentials(config.credentials)
                    .map_err(|source| {
                        Error::new_with_source(
                            format!("failed to create credentials for event subscriber {subscriber_id:?}"),
                            source,
                        )
                    })?;

                debug!(
                    ?credentials,
                    config = ?config.client_config,
                    ?subscriber_id,
                    "configuring azure service bus subscriber",
                );

                let identity_resolver = self.create_azure_identity_resolver().await?;
                let subscriber = AzureServiceBusSubscriber::from_config(
                    &subscriber_id,
                    config.client_config,
                    credentials,
                    identity_resolver,
                )
                .await;

                futures.push(Box::pin(subscriber.run(event_consumer.clone())));
            } else if let Some(config) = subscriber_config.sqs {
                let credentials = self
                    .aws_credentials_factory
                    .try_create_credentials_provider(config.credentials)
                    .await
                    .map_err(|source| {
                        Error::new_with_source(
                            format!("failed to create credentials for event subscriber {subscriber_id:?}"),
                            source,
                        )
                    })?;

                debug!(
                    ?credentials,
                    config = ?config.client_config,
                    ?subscriber_id,
                    "configuring the sqs event subscriber",
                );

                let subscriber = SqsSubscriber::from_config(&subscriber_id, config.client_config, credentials).await;

                futures.push(Box::pin(subscriber.run(event_consumer.clone())));
            } else {
                return Err(Error::new(format!(
                    "event subscriber configuration for {subscriber_id:?} is missing"
                )));
            }

            info!(?subscriber_id, "configured event subscriber");
        }

        info!("configured event subscribers");

        Ok(Box::pin(futures::future::join_all(futures).map(|_| ())))
    }

    fn build_probe_service_future(&self) -> Result<BoxFuture<'static, Result<(), Error>>, Error> {
        if let Some(addr) = self
            .config
            .grpc_probe_service
            .listen_addr()
            .map_err(|source| Error::new_with_source("grpc probe service listen address is invalid", source))?
        {
            probe::create_probe_service(addr)
        } else {
            Ok(Box::pin(async move {
                info!("grpc probe service is disabled");

                futures::future::pending::<()>().await;
                Ok(())
            }))
        }
    }

    async fn get_or_create_redis(&mut self) -> Result<(Option<fred::clients::Pool>, Option<String>), Error> {
        if let Slot::Initialized(redis) = &self.redis {
            return Ok(redis.clone());
        }

        let config = self.config.redis.clone();
        let namespace = config.as_ref().and_then(|config| config.namespace.clone());
        let client = match config {
            Some(config) => {
                let config: storage_core_util::cache::RedisConfig = config.into();

                Some(
                    storage_core_util::cache::build_redis_client_with_config(&config)
                        .await
                        .map_err(|source| Error::new_with_source("failed to connect to redis", source))?,
                )
            },
            None => None,
        };

        let redis = (client, namespace);
        self.redis = Slot::Initialized(redis.clone());

        Ok(redis)
    }

    async fn get_or_create_stat_cache_layer(&mut self) -> Result<Option<LayerArc<StatCacheLayer>>, Error> {
        if let Slot::Initialized(layer) = &self.stat_cache_layer {
            return Ok(layer.clone());
        }

        let config = self.config.stat_cache.clone();
        let (redis, namespace) = if config.enabled && config.l2_enabled {
            self.get_or_create_redis().await?
        } else {
            (None, None)
        };

        let layer = StatCacheLayer::new(config, redis, namespace.as_deref())
            .await
            .map(|value| value.map(layer_arc))
            .map_err(|source| Error::new_with_source("failed to initialize the stat cache layer", source))?;
        self.stat_cache_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    async fn get_or_create_list_cache_layer(&mut self) -> Result<Option<LayerArc<ListCacheLayer>>, Error> {
        if let Slot::Initialized(layer) = &self.list_cache_layer {
            return Ok(layer.clone());
        }

        let config = self.config.list_cache.clone();
        let (redis, namespace) = if config.enabled && config.l2_enabled {
            self.get_or_create_redis().await?
        } else {
            (None, None)
        };

        let layer = ListCacheLayer::new(config, redis, namespace.as_deref())
            .await
            .map(|value| value.map(layer_arc))
            .map_err(|source| Error::new_with_source("failed to initialize the list cache layer", source))?;
        self.list_cache_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    async fn get_or_create_small_object_cache_layer(
        &mut self,
    ) -> Result<Option<LayerArc<SmallObjectCacheLayer>>, Error> {
        if let Slot::Initialized(layer) = &self.small_object_cache_layer {
            return Ok(layer.clone());
        }

        let config = self.config.small_object_cache.clone();
        let (redis, namespace) = if config.enabled && config.l2_enabled {
            self.get_or_create_redis().await?
        } else {
            (None, None)
        };

        let layer = SmallObjectCacheLayer::new(config, redis, namespace.as_deref())
            .await
            .map(|value| value.map(layer_arc))
            .map_err(|source| Error::new_with_source("failed to initialize the small object cache layer", source))?;
        self.small_object_cache_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    async fn get_or_create_manifest_layer(&mut self) -> Result<Option<ManifestLayer>, Error> {
        if let Slot::Initialized(layer) = &self.manifest_layer {
            return Ok(layer.clone());
        }

        let config = self.config.manifest.clone();
        let (redis, namespace) = if config.enabled && config.l2_enabled {
            self.get_or_create_redis().await?
        } else {
            (None, None)
        };

        let layer = ManifestLayer::new(config, redis, namespace.as_deref())
            .await
            .map_err(|source| Error::new_with_source("failed to initialize the manifest layer", source))?;
        self.manifest_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    fn get_or_create_egress_throttle_layer(&mut self) -> Option<LayerArc<EgressThrottleLayer>> {
        if let Slot::Initialized(layer) = &self.egress_throttle_layer {
            return layer.clone();
        }

        let layer = EgressThrottleLayer::new(self.config.egress_throttle.clone()).map(layer_arc);
        self.egress_throttle_layer = Slot::Initialized(layer.clone());

        layer
    }

    fn get_or_create_additional_headers_layer(&mut self) -> Option<LayerArc<AdditionalHeadersLayer>> {
        if let Slot::Initialized(layer) = &self.additional_headers_layer {
            return layer.clone();
        }

        let layer = AdditionalHeadersLayer::new(self.config.additional_headers.clone()).map(layer_arc);
        self.additional_headers_layer = Slot::Initialized(layer.clone());

        layer
    }

    fn get_or_create_operation_metrics_layer(&mut self) -> LayerArc<OperationMetricsLayer> {
        if let Slot::Initialized(layer) = &self.operation_metrics_layer {
            return layer.clone();
        }

        let layer = layer_arc(OperationMetricsLayer::new("any"));
        self.operation_metrics_layer = Slot::Initialized(layer.clone());

        layer
    }

    fn get_or_create_sdk_metrics_layer(&mut self) -> LayerArc<SdkMetricsLayer> {
        if let Slot::Initialized(layer) = &self.sdk_metrics_layer {
            return layer.clone();
        }

        let layer = layer_arc(SdkMetricsLayer::new("any"));
        self.sdk_metrics_layer = Slot::Initialized(layer.clone());

        layer
    }

    fn get_or_create_azure_blob_name_rejecter_layer(&mut self) -> Result<AzureBlobNameRejecter, Error> {
        if let Slot::Initialized(layer) = &self.azure_blob_name_rejecter_layer {
            return Ok(layer.clone());
        }

        let layer = AzureBlobNameRejecter::try_from_azure_endpoints(self.config.storage.azure_blob.endpoints.keys())
            .map_err(|source| Error::new_with_source("failed to initialize the azure blob name rejecter", source))?;
        self.azure_blob_name_rejecter_layer = Slot::Initialized(layer.clone());
        Ok(layer)
    }

    async fn get_or_create_permissions_layer(&mut self) -> Result<Option<PermissionsLayer>, Error> {
        if let Slot::Initialized(layer) = &self.permissions_layer {
            return Ok(layer.clone());
        }

        if !self.config.permissions.enabled {
            self.permissions_layer = Slot::Initialized(None);
            return Ok(None);
        }

        let filter = PermissionRequestFilter::try_from_endpoints(
            self.config
                .storage
                .s3
                .endpoints
                .iter()
                .filter(|(_, config)| !config.permissions_enabled)
                .map(|(endpoint, _)| endpoint.as_ref()),
            self.config
                .storage
                .azure_blob
                .endpoints
                .iter()
                .filter(|(_, config)| !config.permissions_enabled)
                .map(|(endpoint, _)| endpoint.as_ref()),
        )
        .map_err(|source| Error::new_with_source("failed to build the permission request filter", source))?;

        let layer = PermissionsLayer::try_from_config(self.config.permissions.clone(), filter)
            .await
            .map_err(|source| Error::new_with_source("failed to initialize the permissions layer", source))?;
        self.permissions_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    async fn get_or_create_metadata_cache_layer(&mut self) -> Result<Option<LayerArc<MetadataCache>>, Error> {
        if let Slot::Initialized(layer) = &self.metadata_cache_layer {
            return Ok(layer.clone());
        }

        let config = self.config.metadata_cache.clone();
        let (redis, namespace) = if config.enabled && config.l2_enabled {
            self.get_or_create_redis().await?
        } else {
            (None, None)
        };

        let layer = MetadataCache::try_from_config(config, redis, namespace.as_deref())
            .await
            .map(|value| value.map(layer_arc))
            .map_err(|source| Error::new_with_source("failed to initialize the metadata cache", source))?;
        self.metadata_cache_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    async fn get_or_create_url_remapper_layer(&mut self) -> Result<Option<UrlRemapperLayer<AnyMetadata>>, Error> {
        if let Slot::Initialized(layer) = &self.url_remapper_layer {
            return Ok(layer.clone());
        }

        if !self.config.url_remapper.enabled {
            self.url_remapper_layer = Slot::Initialized(None);
            return Ok(None);
        }

        let metadata = self.get_or_create_metadata_backend().await?;
        let permissions = self.get_or_create_permissions_layer().await?;
        let address_filter = permissions
            .as_ref()
            .map(|layer| Arc::new(layer.address_filter()) as ArcAddressFilter);
        let endpoint_roots: Vec<(String, RootListing)> = self
            .config
            .storage
            .azure_blob
            .endpoints
            .keys()
            .map(|endpoint| (endpoint.as_ref().url().to_owned(), RootListing::HostingOnly))
            .chain(self.config.storage.s3.endpoints.iter().map(|(endpoint, config)| {
                let listing = if config.list_in_top_level_addresses && config.list_in_routes {
                    RootListing::SelfListed
                } else {
                    RootListing::HostingOnly
                };

                (endpoint.as_ref().url().to_owned(), listing)
            }))
            .collect();

        let layer = UrlRemapper::new(metadata, self.config.url_remapper.clone(), endpoint_roots)
            .map(|value| value.map(|remapper| UrlRemapperLayer::new(remapper, address_filter)))
            .map_err(|source| Error::new_with_source("failed to configure the url remapper", source))?;
        self.url_remapper_layer = Slot::Initialized(layer.clone());

        Ok(layer)
    }

    fn get_or_create_url_normalizer_layer(&mut self) -> Option<UrlNormalizerLayer> {
        if let Slot::Initialized(layer) = &self.url_normalizer_layer {
            return layer.clone();
        }

        let layer = UrlNormalizerLayer::new(self.config.url_normalizer.clone());
        self.url_normalizer_layer = Slot::Initialized(layer.clone());

        layer
    }

    fn get_or_create_created_timestamp_layer(
        &mut self,
        metadata: Option<AnyMetadata>,
    ) -> Option<LayerArc<CreatedTimestampLayer>> {
        if let Slot::Initialized(layer) = &self.created_timestamp_layer {
            return layer.clone();
        }

        let layer = metadata.map(|metadata| layer_arc(CreatedTimestampLayer::new(metadata)));
        self.created_timestamp_layer = Slot::Initialized(layer.clone());

        layer
    }

    async fn register_discovery_endpoints<L: Clone>(
        &mut self,
        service_builder: StorageService<L>,
    ) -> Result<StorageServiceServer<L>, Error> {
        let metadata_enabled = self.get_or_create_metadata_backend().await?.is_some();

        Ok(service_builder.add_service(discovery::v2alpha::Capabilities::new(
            discovery::v2alpha::service_entries(metadata_enabled),
        )))
    }

    async fn register_v1alpha_endpoints<L: Clone>(
        &mut self,
        service_builder: StorageServiceServer<L>,
    ) -> Result<StorageServiceServer<L>, Error> {
        use storage_core::api_services::{
            capabilities::v1alpha::Capabilities, filefolder::v1alpha::FileFolder, fileobject::v1alpha::FileObject,
            versioning::v1alpha::Versioning,
        };

        use crate::backend::builder::{
            capabilities::v1alpha::AnyCapabilities, filefolder::v1alpha::AnyFileFolder,
            fileobject::v1alpha::AnyFileObject, v1alpha::AnyServiceSet, versioning::v1alpha::AnyVersioning,
        };

        let storage_backend = self.get_or_create_storage_router().await?;
        let metadata_backend = self.get_or_create_metadata_backend().await?;
        let permissions_layer = self.get_or_create_permissions_layer().await?;
        let azure_blob_name_rejecter_layer = self.get_or_create_azure_blob_name_rejecter_layer()?;
        let url_remapper_layer = self.get_or_create_url_remapper_layer().await?;
        let url_normalizer_layer = self.get_or_create_url_normalizer_layer();
        let created_timestamp_layer = self.get_or_create_created_timestamp_layer(metadata_backend);
        let stat_cache_layer = self.get_or_create_stat_cache_layer().await?;
        let list_cache_layer = self.get_or_create_list_cache_layer().await?;
        let small_object_cache_layer = self.get_or_create_small_object_cache_layer().await?;
        let manifest_layer = self.get_or_create_manifest_layer().await?;

        let base = AnyServiceSet {
            fileobject: AnyFileObject::new(storage_backend.clone()),
            filefolder: AnyFileFolder::new(storage_backend.clone()),
            capabilities: AnyCapabilities::new(storage_backend.clone()),
            versioning: AnyVersioning::new(storage_backend),
        }
        .decorate(&self.get_or_create_sdk_metrics_layer())
        .decorate(&self.get_or_create_egress_throttle_layer())
        .decorate_each(&small_object_cache_layer, &Skip, &Skip, &Skip)
        .decorate_each(&stat_cache_layer, &Skip, &Skip, &Skip)
        .decorate_each(&list_cache_layer, &list_cache_layer, &Skip, &Skip);

        let resolver = base
            .clone()
            .decorate(&permissions_layer)
            .decorate_each(
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &Skip,
            )
            .decorate(&url_remapper_layer)
            .decorate_each(
                &url_normalizer_layer,
                &url_normalizer_layer,
                &url_normalizer_layer,
                &Skip,
            );
        let manifest = manifest_layer.map(|layer| layer.with_resolver(resolver));

        let service_set = base
            .decorate_each(&manifest, &manifest, &manifest, &Skip)
            .decorate(&permissions_layer)
            .decorate_each(
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &Skip,
            )
            .decorate_each(&created_timestamp_layer, &created_timestamp_layer, &Skip, &Skip)
            .decorate(&url_remapper_layer)
            .decorate_each(
                &url_normalizer_layer,
                &url_normalizer_layer,
                &url_normalizer_layer,
                &Skip,
            )
            .decorate(&self.get_or_create_additional_headers_layer())
            .decorate_each(&RequestLimitsLayer, &RequestLimitsLayer, &RequestLimitsLayer, &Skip)
            .decorate(&self.get_or_create_operation_metrics_layer())
            .decorate(&TraceErrorLayer);

        Ok(service_builder
            .add_service(FileObject::new(service_set.fileobject))
            .add_service(FileFolder::new(service_set.filefolder))
            .add_service(Capabilities::new(service_set.capabilities))
            .add_service(Versioning::new(service_set.versioning)))
    }

    async fn register_v1beta_endpoints<L: Clone>(
        &mut self,
        service_builder: StorageServiceServer<L>,
    ) -> Result<StorageServiceServer<L>, Error> {
        use storage_core::api_services::{
            capabilities::v1beta::Capabilities, filefolder::v1beta::FileFolder, fileobject::v1beta::FileObject,
            versioning::v1beta::Versioning,
        };

        use crate::backend::builder::{
            capabilities::v1beta::AnyCapabilities, filefolder::v1beta::AnyFileFolder,
            fileobject::v1beta::AnyFileObject, v1beta::AnyServiceSet, versioning::v1beta::AnyVersioning,
        };

        let storage_backend = self.get_or_create_storage_router().await?;
        let metadata_backend = self.get_or_create_metadata_backend().await?;
        let permissions_layer = self.get_or_create_permissions_layer().await?;
        let azure_blob_name_rejecter_layer = self.get_or_create_azure_blob_name_rejecter_layer()?;
        let url_remapper_layer = self.get_or_create_url_remapper_layer().await?;
        let url_normalizer_layer = self.get_or_create_url_normalizer_layer();
        let created_timestamp_layer = self.get_or_create_created_timestamp_layer(metadata_backend);
        let stat_cache_layer = self.get_or_create_stat_cache_layer().await?;
        let list_cache_layer = self.get_or_create_list_cache_layer().await?;
        let small_object_cache_layer = self.get_or_create_small_object_cache_layer().await?;
        let manifest_layer = self.get_or_create_manifest_layer().await?;

        let base = AnyServiceSet {
            fileobject: AnyFileObject::new(storage_backend.clone()),
            filefolder: AnyFileFolder::new(storage_backend.clone()),
            capabilities: AnyCapabilities::new(storage_backend.clone()),
            versioning: AnyVersioning::new(storage_backend),
        }
        .decorate(&self.get_or_create_sdk_metrics_layer())
        .decorate(&self.get_or_create_egress_throttle_layer())
        .decorate_each(&small_object_cache_layer, &Skip, &Skip, &Skip)
        .decorate_each(&stat_cache_layer, &Skip, &Skip, &Skip)
        .decorate_each(&list_cache_layer, &list_cache_layer, &Skip, &Skip);

        let resolver = base
            .clone()
            .decorate(&permissions_layer)
            .decorate_each(
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &Skip,
            )
            .decorate(&url_remapper_layer)
            .decorate_each(
                &url_normalizer_layer,
                &url_normalizer_layer,
                &url_normalizer_layer,
                &Skip,
            );
        let manifest = manifest_layer.map(|layer| layer.with_resolver(resolver));

        let service_set = base
            .decorate_each(&manifest, &manifest, &manifest, &Skip)
            .decorate(&permissions_layer)
            .decorate_each(
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &azure_blob_name_rejecter_layer,
                &Skip,
            )
            .decorate_each(&created_timestamp_layer, &created_timestamp_layer, &Skip, &Skip)
            .decorate(&url_remapper_layer)
            .decorate_each(
                &url_normalizer_layer,
                &url_normalizer_layer,
                &url_normalizer_layer,
                &Skip,
            )
            .decorate(&self.get_or_create_additional_headers_layer())
            .decorate(&self.get_or_create_operation_metrics_layer())
            .decorate(&TraceErrorLayer);

        Ok(service_builder
            .add_service(FileObject::new(service_set.fileobject))
            .add_service(FileFolder::new(service_set.filefolder))
            .add_service(Capabilities::new(service_set.capabilities))
            .add_service(Versioning::new(service_set.versioning)))
    }

    async fn register_metadata_endpoints<L: Clone>(
        &mut self,
        service_builder: StorageServiceServer<L>,
    ) -> Result<StorageServiceServer<L>, Error> {
        let Some(metadata_backend) = self.get_or_create_metadata_backend().await? else {
            return Ok(service_builder);
        };

        let permissions_layer = self.get_or_create_permissions_layer().await?;
        let azure_blob_name_rejecter_layer = self.get_or_create_azure_blob_name_rejecter_layer()?;
        let url_remapper_layer = self.get_or_create_url_remapper_layer().await?;
        let url_normalizer_layer = self.get_or_create_url_normalizer_layer();
        let metadata_metrics = MetadataMetricsLayer::new();

        let metadata_backend = metadata_backend
            .decorate(&permissions_layer)
            .decorate(&azure_blob_name_rejecter_layer)
            .decorate(&url_remapper_layer)
            .decorate(&url_normalizer_layer)
            .decorate(&RequestLimitsLayer)
            .decorate(&metadata_metrics)
            .decorate(&TraceErrorLayer);

        Ok(service_builder
            .add_service(metadata::v1alpha::Metadata::new(metadata_backend.clone()))
            .add_service(metadata_api::v1beta::Metadata::new(metadata_backend)))
    }

    async fn get_or_create_azure_backend(&mut self) -> Result<AzureBackend, Error> {
        use crate::common::azure::StorageCredentials;
        use storage_core::azure::secrets::{ClientCredentials, CredentialOverride, Secrets};

        if let Slot::Initialized(backend) = &self.azure_backend {
            return Ok(backend.clone());
        }

        info!("configuring azure blob backend");

        let backend = {
            let config = &self.config.storage.azure_blob;
            let credentials_factory = &mut self.azure_credentials_factory;
            let selector = RegexSelector::try_from_azure_endpoints(
                config.endpoints.iter().map(|(url, config)| RegexSelectorRoute {
                    endpoint: url,
                    show_tla: config.list_in_top_level_addresses && config.list_in_routes,
                    show_route: config.list_in_routes,
                }),
                config.client_config.allow_unconfigured_endpoints,
            )
            .map_err(|source| Error::new_with_source("failed to create the azure blob route selector", source))?;

            let mut secrets = HashMap::new();
            for (endpoint, config) in config.endpoints.clone() {
                info!(endpoint = %endpoint.0, "configuring azure endpoint");

                let credentials = credentials_factory
                    .try_create_maybe_anon_storage_credentials(config.credentials)
                    .map_err(|source| {
                        Error::new_with_source(format!("failed to create azure credentials for {endpoint:?}"), source)
                    })?;
                debug!(?credentials, ?endpoint, "azure endpoint credentials selected");

                let anonymous_credentials = credentials.is_none();
                let (storage_key, credential_override) = match credentials {
                    Some(StorageCredentials::StorageKey(key)) => (Some(key), None),
                    Some(StorageCredentials::TokenCredential(credential)) => {
                        (None, Some(CredentialOverride::TokenCredential(credential)))
                    },

                    None => (None, Some(CredentialOverride::Anonymous)),
                };

                secrets.insert(
                    endpoint.0.into_inner_without_trailing_slash(),
                    ClientCredentials {
                        storage_account: config.storage_account,
                        front_door_url: config.front_door.map(|config| config.url),
                        storage_key: storage_key.map(|secret| secret.into_inner()),
                        credential_override,
                        authority_host: None,
                        client_id: None,
                        client_secret: None,
                        tenant_id: None,
                        versioning_enabled: config.versioning_enabled,
                        read_only: anonymous_credentials.then_some(true).or(config.read_only),
                    },
                );
            }

            let storage = azure::Storage::new(config.client_config.clone(), Some(Secrets(secrets)))
                .map_err(|source| Error::new_with_source("failed to configure the azure blob backend", source))?;

            AzureBackend {
                storage: Arc::new(storage),
                selector,
            }
        };

        info!("configured azure blob backend");

        self.azure_backend = Slot::Initialized(backend.clone());

        Ok(backend)
    }

    async fn get_or_create_s3_backend(&mut self) -> Result<S3Backend, Error> {
        use storage_core::s3::secrets::{ClientCredentials, Cloudfront, CredentialsOverride, Secrets};

        if let Slot::Initialized(backend) = &self.s3_backend {
            return Ok(backend.clone());
        }

        info!("configuring s3 backend");

        let backend = {
            let config = &self.config.storage.s3;
            let credentials_factory = &mut self.aws_credentials_factory;
            let selector = RegexSelector::try_from_s3_endpoints(
                config.endpoints.iter().map(|(url, config)| RegexSelectorRoute {
                    endpoint: url,
                    show_tla: config.list_in_top_level_addresses && config.list_in_routes,
                    show_route: config.list_in_routes,
                }),
                config.client_config.allow_unconfigured_endpoints,
            )
            .map_err(|source| Error::new_with_source("failed to create the s3 route selector", source))?;

            let mut secrets = HashMap::new();
            for (endpoint, config) in config.endpoints.clone() {
                info!(endpoint = %endpoint.0, "configuring s3 endpoint");

                let credentials = credentials_factory
                    .try_create_storage_credentials(config.credentials)
                    .await
                    .map_err(|source| {
                        Error::new_with_source(format!("failed to create s3 credentials for {endpoint:?}"), source)
                    })?;
                debug!(?endpoint, ?credentials, "s3 endpoint credentials selected");

                let anonymous_credentials = matches!(credentials, CredentialsOverride::Anonymous);

                secrets.insert(
                    endpoint.0.into_inner_without_trailing_slash(),
                    ClientCredentials {
                        bucket: config.bucket,
                        region: config.region,
                        addressing_style: config.addressing_style,
                        endpoint_url: config.endpoint_url,
                        cloudfront: config.cloudfront.map(|config| Cloudfront {
                            distribution_url: Some(config.distribution_url),
                            key_pair_id: config.key_pair_id,
                            private_key_pem: config.private_key_pem.map(|secret| secret.into_inner()),
                        }),
                        credentials_override: Some(credentials),
                        access_key_id: None,
                        secret_access_key: None,
                        session_token: None,
                        role_session_name: None,
                        role_session_length: None,
                        role_arn: None,
                        external_id: None,
                        versioning_enabled: config.versioning_enabled,
                        read_only: anonymous_credentials.then_some(true).or(config.read_only),
                    },
                );
            }

            let storage = s3::Storage::new(config.client_config.clone(), Some(Secrets(secrets)))
                .map_err(|source| Error::new_with_source("failed to configure the s3 backend", source))?;

            S3Backend {
                storage: Arc::new(storage),
                selector,
            }
        };

        info!("configured s3 backend");

        self.s3_backend = Slot::Initialized(backend.clone());

        Ok(backend)
    }

    async fn get_or_create_storage_router(&mut self) -> Result<Arc<StorageRouter>, Error> {
        if let Slot::Initialized(router) = &self.storage_router {
            return Ok(router.clone());
        }

        let azure = self.get_or_create_azure_backend().await?;
        let s3 = self.get_or_create_s3_backend().await?;
        let router = Arc::new(
            router::Builder
                .backend(azure.storage, azure.selector)
                .backend(s3.storage, s3.selector),
        );
        self.storage_router = Slot::Initialized(router.clone());

        Ok(router)
    }

    async fn create_azure_identity_resolver(&mut self) -> Result<Arc<AzureIdentityResolver>, Error> {
        let azure = self.get_or_create_azure_backend().await?;
        let resolver = Arc::new(AzureIdentityResolver::new(azure.storage));

        Ok(resolver)
    }

    async fn get_or_create_metadata_backend(&mut self) -> Result<Option<AnyMetadata>, Error> {
        use crate::backend::metadata::{reject_duplicates::RejectDuplicates, router::Router as MetadataRouter};

        if let Slot::Initialized(backend) = &self.metadata_backend {
            return Ok(backend.clone());
        }

        let config = self.config.clone();
        let mut azure_metadata_routes = Vec::new();
        for (endpoint, config) in &config.storage.azure_blob.endpoints {
            if let Some(backend_id) = &config.metadata_backend {
                let backend = self.get_or_create_metadata_backend_instance(backend_id).await?;
                azure_metadata_routes.push((backend, &endpoint.0));
            }
        }

        let mut s3_metadata_routes = Vec::new();
        for (endpoint, config) in &config.storage.s3.endpoints {
            if let Some(backend_id) = &config.metadata_backend {
                let backend = self.get_or_create_metadata_backend_instance(backend_id).await?;
                s3_metadata_routes.push((backend, &endpoint.0));
            }
        }

        if azure_metadata_routes.is_empty() && s3_metadata_routes.is_empty() {
            info!("storage endpoints do not reference metadata backends");
            self.metadata_backend = Slot::Initialized(None);

            return Ok(None);
        }

        let router = MetadataRouter::try_from_routes(azure_metadata_routes, s3_metadata_routes)
            .map_err(|source| Error::new_with_source("failed to create the metadata router", source))?;
        let metadata_cache = self.get_or_create_metadata_cache_layer().await?;
        let egress_throttle = self.get_or_create_egress_throttle_layer();
        let backend = Some(
            AnyMetadata::from_metadata(router)
                .decorate(&egress_throttle)
                .decorate(&metadata_cache)
                .decorate(&RejectDuplicates)
                .decorate(&UriCanonicalization),
        );
        self.metadata_backend = Slot::Initialized(backend.clone());

        Ok(backend)
    }

    async fn get_or_create_metadata_backend_instance(&mut self, backend_id: &str) -> Result<AnyMetadata, Error> {
        if let Some(instance) = self.metadata_backend_instances.get(backend_id) {
            return Ok(instance.clone());
        }

        info!(?backend_id, "configuring metadata backend");

        let config = self
            .config
            .metadata
            .as_ref()
            .and_then(|instances| instances.get(backend_id))
            .cloned()
            .ok_or_else(|| Error::new(format!("metadata backend {backend_id:?} is not defined")))?;
        let aws_credentials = &mut self.aws_credentials_factory;
        let azure_credentials = &mut self.azure_credentials_factory;
        debug!(?backend_id, "initializing metadata backend");
        let instance =
            Self::try_create_metadata_backend_from_config(backend_id, config, aws_credentials, azure_credentials)
                .await
                .map_err(|source| {
                    Error::new_with_source(format!("failed to configure metadata backend {backend_id:?}"), source)
                })?;

        info!(?backend_id, "configured metadata backend");

        self.metadata_backend_instances
            .insert(backend_id.to_owned(), instance.clone());

        Ok(instance)
    }

    async fn try_create_metadata_backend_from_config(
        backend_id: &str,
        config: MetadataBackendConfig,
        #[allow(unused)] aws_credentials: &mut AWSCredentialsFactory,
        #[allow(unused)] azure_credentials: &mut AzureCredentialsFactory,
    ) -> Result<AnyMetadata, Error> {
        use crate::backend::metadata::postgres;

        #[cfg(feature = "dynamodb")]
        if let Some(config) = config.dynamodb {
            use crate::backend::metadata::dynamodb;

            let credentials = aws_credentials
                .try_create_credentials_provider(config.credentials)
                .await
                .map_err(|source| Error::new_with_source("failed to create dynamodb client credentials", source))?;

            debug!(?credentials, config = ?config.client_config, "configuring the dynamodb metadata backend");
            let backend =
                dynamodb::DynamoDBMetadataBackend::from_config(backend_id, config.client_config, credentials).await;

            return Ok(AnyMetadata::from_metadata(backend));
        }

        #[cfg(feature = "cosmosdb")]
        if let Some(config) = config.cosmosdb {
            use crate::backend::metadata::azure_cosmosdb;

            let credentials = azure_credentials
                .try_create_cosmosdb_credentials(config.credentials)
                .map_err(|source| Error::new_with_source("failed to create cosmosdb client credentials", source))?;

            debug!(?credentials, config = ?config.client_config, "configuring the cosmosdb metadata backend");
            let backend = azure_cosmosdb::AzureCosmosDBMetadataBackend::try_from_config(
                backend_id,
                config.client_config,
                credentials,
            )
            .map_err(|source| Error::new_with_source("failed to create cosmosdb metadata backend", source))?;

            return Ok(AnyMetadata::from_metadata(backend));
        }

        if let Some(config) = config.postgres {
            let credentials_provider = Self::try_create_pg_meta_credentials_provider(config.credentials)?;

            debug!(?credentials_provider, config = ?config.client_config, "configuring the postgres metadata backend");
            let backend = postgres::PostgresMetadataBackend::try_from_config(
                backend_id,
                config.client_config,
                credentials_provider,
            )
            .await
            .map_err(|source| Error::new_with_source("failed to configure the postgres metadata backend", source))?;

            return Ok(AnyMetadata::from_metadata(backend));
        }

        Err(Error::new("unknown metadata backend"))
    }

    fn try_create_pg_meta_credentials_provider(
        config: PostgresCredentialsConfig,
    ) -> Result<crate::backend::metadata::postgres::credentials::ArcCredentialsProvider, Error> {
        use crate::backend::metadata::postgres::credentials::{
            AccessTokenFileCredentialsProvider, PasswordCredentialsProvider,
        };

        if let Some(config) = config.password {
            Ok(Arc::new(PasswordCredentialsProvider::new(config.user, config.password)))
        } else if let Some(config) = config.access_token_file {
            Ok(Arc::new(AccessTokenFileCredentialsProvider::new(
                config.user,
                config.path,
            )))
        } else {
            Err(Error::new("postgres client credentials config is missing"))
        }
    }
}

#[derive(Default)]
struct AzureCredentialsFactory {
    http_client: reqwest::Client,
    federated_credential_cache: HashMap<AzureFederatedCredentialsConfig, Arc<dyn TokenCredential>>,
    client_secret_credential_cache: HashMap<AzureClientSecretCredentialsConfig, Arc<dyn TokenCredential>>,
    token_file_credential_cache: HashMap<AzureTokenFileCredentialsConfig, Arc<dyn TokenCredential>>,
}

impl AzureCredentialsFactory {
    fn try_create_maybe_anon_storage_credentials(
        &mut self,
        config: AzureMaybeAnonStorageCredentialsConfig,
    ) -> Result<Option<crate::common::azure::StorageCredentials>, Error> {
        use crate::common::azure::StorageCredentials;

        let credentials = if config.anonymous {
            None
        } else if let Some(config) = config.client_secret {
            Some(StorageCredentials::TokenCredential(
                self.create_client_secret_credential(config),
            ))
        } else if let Some(config) = config.storage_key {
            Some(StorageCredentials::StorageKey(config.storage_key))
        } else if let Some(config) = config.federated {
            Some(StorageCredentials::TokenCredential(
                self.create_federated_credential(config),
            ))
        } else if let Some(config) = config.access_token_file {
            Some(StorageCredentials::TokenCredential(
                self.create_access_token_file_credential(config),
            ))
        } else {
            return Err(Error::new("client credentials are not configured"));
        };

        Ok(credentials)
    }

    #[cfg(feature = "cosmosdb")]
    fn try_create_cosmosdb_credentials(
        &mut self,
        config: crate::config::AzureCosmosDBCredentialsConfig,
    ) -> Result<crate::backend::metadata::azure_cosmosdb::Credentials, Error> {
        use crate::backend::metadata::azure_cosmosdb::Credentials;

        if let Some(config) = config.client_secret {
            Ok(Credentials::TokenCredential(
                self.create_client_secret_credential(config),
            ))
        } else if let Some(config) = config.primary_key {
            Ok(Credentials::PrimaryKey(config.key))
        } else if let Some(config) = config.federated {
            Ok(Credentials::TokenCredential(self.create_federated_credential(config)))
        } else if let Some(config) = config.access_token_file {
            Ok(Credentials::TokenCredential(
                self.create_access_token_file_credential(config),
            ))
        } else {
            Err(Error::new("client credentials are not configured"))
        }
    }

    fn try_create_service_bus_credentials(
        &mut self,
        config: AzureServiceBusCredentialsConfig,
    ) -> Result<crate::pubsub::azure_service_bus::Credentials, Error> {
        use crate::pubsub::azure_service_bus::Credentials;

        if let Some(config) = config.client_secret {
            Ok(Credentials::TokenCredential(
                self.create_client_secret_credential(config),
            ))
        } else if let Some(config) = config.shared_access_policy_key {
            Ok(Credentials::SASPolicy {
                policy_name: config.policy_name,
                policy_key: config.policy_key,
            })
        } else if let Some(config) = config.federated {
            Ok(Credentials::TokenCredential(self.create_federated_credential(config)))
        } else if let Some(config) = config.access_token_file {
            Ok(Credentials::TokenCredential(
                self.create_access_token_file_credential(config),
            ))
        } else {
            Err(Error::new("client credentials are not configured"))
        }
    }

    fn create_client_secret_credential(
        &mut self,
        config: AzureClientSecretCredentialsConfig,
    ) -> Arc<dyn TokenCredential> {
        use crate::common::azure::ClientSecretCredential;

        self.client_secret_credential_cache
            .entry(config)
            .or_insert_with_key(|config| {
                let credential = ClientSecretCredential::new(
                    Arc::new(self.http_client.clone()),
                    config.authority_host.clone(),
                    config.tenant_id.clone(),
                    config.client_id.clone(),
                    config.client_secret.inner().clone(),
                );

                Arc::new(credential)
            })
            .clone()
    }

    fn create_federated_credential(&mut self, config: AzureFederatedCredentialsConfig) -> Arc<dyn TokenCredential> {
        use crate::common::azure::FederatedCredentialsFlowCredential;

        self.federated_credential_cache
            .entry(config)
            .or_insert_with_key(|config| {
                let credential = FederatedCredentialsFlowCredential::new(
                    Arc::new(self.http_client.clone()),
                    config.federated_token_file.clone(),
                    config.client_id.clone(),
                    config.tenant_id.clone(),
                    config.authority_host.clone(),
                );

                Arc::new(credential)
            })
            .clone()
    }

    fn create_access_token_file_credential(
        &mut self,
        config: AzureTokenFileCredentialsConfig,
    ) -> Arc<dyn TokenCredential> {
        use crate::common::azure::AccessTokenFileCredential;

        self.token_file_credential_cache
            .entry(config)
            .or_insert_with_key(|config| {
                let credential = AccessTokenFileCredential::new(&config.path);

                Arc::new(credential)
            })
            .clone()
    }
}

#[derive(Default)]
struct AWSCredentialsFactory {
    access_key_credentials_cache: HashMap<AWSAccessKeyCredentialsConfig, Arc<dyn ProvideCredentials>>,
    web_identity_token_credentials_cache: HashMap<AWSWebIdentityTokenCredentialsConfig, Arc<dyn ProvideCredentials>>,
}

impl AWSCredentialsFactory {
    async fn try_create_storage_credentials(
        &mut self,
        config: AWSMaybeAnonCredentialsConfig,
    ) -> Result<storage_core::s3::secrets::CredentialsOverride, Error> {
        use storage_core::s3::secrets::CredentialsOverride;

        let credentials = if config.anonymous {
            CredentialsOverride::Anonymous
        } else if let Some(config) = config.access_key {
            CredentialsOverride::CredentialsProvider(self.create_static_credentials_provider(config))
        } else if let Some(config) = config.web_identity_token {
            CredentialsOverride::CredentialsProvider(self.create_web_identity_token_credentials_provider(config).await)
        } else if let Some(config) = config.credentials_file {
            CredentialsOverride::CredentialsProvider(Self::create_credentials_file_credentials_provider(config))
        } else {
            return Err(Error::new("client credentials are not configured"));
        };

        let credentials = if let Some(config) = config.assume_role {
            let CredentialsOverride::CredentialsProvider(credentials) = credentials else {
                return Err(Error::new(
                    "assume role credentials cannot be used with anonymous authentication",
                ));
            };

            CredentialsOverride::CredentialsProvider(
                self.create_assume_role_credentials_provider(credentials, config).await,
            )
        } else {
            credentials
        };

        Ok(credentials)
    }

    async fn try_create_credentials_provider(
        &mut self,
        config: AWSCredentialsConfig,
    ) -> Result<Arc<dyn ProvideCredentials>, Error> {
        let provider = if let Some(config) = config.access_key {
            self.create_static_credentials_provider(config)
        } else if let Some(config) = config.web_identity_token {
            self.create_web_identity_token_credentials_provider(config).await
        } else if let Some(config) = config.credentials_file {
            Self::create_credentials_file_credentials_provider(config)
        } else {
            return Err(Error::new("client credentials are not configured"));
        };

        let provider = if let Some(config) = config.assume_role {
            self.create_assume_role_credentials_provider(provider, config).await
        } else {
            provider
        };

        Ok(provider)
    }

    async fn create_assume_role_credentials_provider(
        &self,
        provider: Arc<dyn ProvideCredentials>,
        config: AWSAssumeRoleCredentialsConfig,
    ) -> Arc<dyn ProvideCredentials> {
        use aws_config::sts::AssumeRoleProvider;

        // todo: share http clients.

        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(provider)
            .load()
            .await;

        let builder = AssumeRoleProvider::builder(config.role_arn).configure(&sdk_config);
        let builder = if let Some(external_id) = config.external_id {
            builder.external_id(external_id)
        } else {
            builder
        };

        let builder = if let Some(session_name) = config.session_name {
            builder.session_name(session_name)
        } else {
            builder
        };

        let builder = if let Some(session_length) = config.session_length {
            builder.session_length(session_length)
        } else {
            builder
        };

        Arc::new(builder.build().await)
    }

    fn create_static_credentials_provider(
        &mut self,
        config: AWSAccessKeyCredentialsConfig,
    ) -> Arc<dyn ProvideCredentials> {
        use aws_credential_types::Credentials;

        self.access_key_credentials_cache
            .entry(config)
            .or_insert_with_key(|config| {
                let provider = Credentials::new(
                    config.access_key_id.clone(),
                    config.secret_access_key.inner().clone(),
                    config.session_token.clone(),
                    None,
                    "static",
                );

                Arc::new(provider)
            })
            .clone()
    }

    async fn create_web_identity_token_credentials_provider(
        &mut self,
        config: AWSWebIdentityTokenCredentialsConfig,
    ) -> Arc<dyn ProvideCredentials> {
        use aws_config::{
            provider_config::ProviderConfig,
            web_identity_token::{StaticConfiguration, WebIdentityTokenCredentialsProvider},
        };
        use std::collections::hash_map::Entry;
        use time::OffsetDateTime;

        match self.web_identity_token_credentials_cache.entry(config) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let config = entry.key();
                let session_name = config
                    .role_session_name
                    .clone()
                    .unwrap_or_else(|| format!("web-identity-{}", OffsetDateTime::now_utc().unix_timestamp()));

                let provider_config = ProviderConfig::with_default_region().await;
                let provider = WebIdentityTokenCredentialsProvider::builder()
                    .configure(&provider_config)
                    .static_configuration(StaticConfiguration {
                        web_identity_token_file: config.web_identity_token_file.clone(),
                        role_arn: config.role_arn.clone(),
                        session_name,
                    })
                    .build();

                entry.insert(Arc::new(provider)).clone()
            },
        }
    }

    fn create_credentials_file_credentials_provider(
        config: AWSCredentialsFileCredentialsConfig,
    ) -> Arc<dyn ProvideCredentials> {
        use crate::common::aws::FileCredentials;

        Arc::new(FileCredentials::new(config.path))
    }
}

fn layer_arc<T>(layer: T) -> LayerArc<T> {
    LayerArc(Arc::new(layer))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use assert_matches::assert_matches;
    use serde::Serialize;
    use tempfile::{tempdir, TempDir};
    use time::{format_description::well_known::Rfc3339, macros::datetime, OffsetDateTime};
    use tokio::io::AsyncWriteExt;

    use super::{AWSCredentialsFactory, AzureCredentialsFactory};
    use crate::config::AzureMaybeAnonStorageCredentialsConfig;
    use crate::{
        common::secret::Secret,
        config::{
            AWSAccessKeyCredentialsConfig, AWSCredentialsConfig, AWSCredentialsFileCredentialsConfig,
            AzureSASPolicyCredentialsConfig, AzureServiceBusCredentialsConfig, AzureStorageKeyCredentialsConfig,
            AzureTokenFileCredentialsConfig,
        },
    };

    #[cfg(feature = "cosmosdb")]
    use crate::config::{AzureCosmosDBCredentialsConfig, AzureCosmosDBPrimaryKeyCredentialsConfig};

    const AZURE_ACCESS_TOKEN: &str = "token";

    struct ScopedTokenFile {
        file_path: PathBuf,
        _temp_dir: TempDir,
    }

    impl ScopedTokenFile {
        async fn create(token: &impl Serialize) -> Self {
            let temp_dir = tempdir().unwrap();
            let file_path = temp_dir.path().join("token");
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&file_path)
                .await
                .unwrap();

            let encoded_token = serde_json::to_vec(token).unwrap();
            file.write_all(&encoded_token).await.unwrap();

            Self {
                _temp_dir: temp_dir,
                file_path,
            }
        }

        async fn create_az() -> (Self, OffsetDateTime) {
            let expires_on = OffsetDateTime::now_utc().truncate_to_second() + Duration::from_hours(1);
            let token_file = Self::create(&serde_json::json!({
                "access_token": AZURE_ACCESS_TOKEN,
                "expires_on": expires_on.format(&Rfc3339).unwrap(),
            }))
            .await;

            (token_file, expires_on)
        }
    }

    #[test_log::test(tokio::test)]
    async fn test_aws_credentials_factory_static_credentials() {
        let mut factory = AWSCredentialsFactory::default();
        let provider = factory
            .try_create_credentials_provider(AWSCredentialsConfig {
                access_key: Some(AWSAccessKeyCredentialsConfig {
                    access_key_id: "ak".into(),
                    secret_access_key: Secret::new("sk"),
                    session_token: Some("st".into()),
                }),
                ..Default::default()
            })
            .await
            .unwrap();

        let credentials = provider.provide_credentials().await.unwrap();

        assert_eq!(credentials.access_key_id(), "ak");
        assert_eq!(credentials.secret_access_key(), "sk");
        assert_eq!(credentials.session_token(), Some("st"));
        assert_eq!(credentials.expiry(), None);
    }

    #[test_log::test(tokio::test)]
    async fn test_aws_credentials_factory_credentials_file() {
        let expires_on = datetime!(2999-03-03 10:10:10 UTC);
        let token = ScopedTokenFile::create(&serde_json::json!({
            "access_key_id": "ak",
            "secret_access_key": "sk",
            "session_token": "st",
            "expires_on": "2999-03-03T10:10:10Z",
        }))
        .await;

        let mut factory = AWSCredentialsFactory::default();
        let provider = factory
            .try_create_credentials_provider(AWSCredentialsConfig {
                credentials_file: Some(AWSCredentialsFileCredentialsConfig {
                    path: token.file_path.clone(),
                }),
                ..Default::default()
            })
            .await
            .unwrap();

        let credentials = provider.provide_credentials().await.unwrap();

        assert_eq!(credentials.access_key_id(), "ak");
        assert_eq!(credentials.secret_access_key(), "sk");
        assert_eq!(credentials.session_token(), Some("st"));
        assert_eq!(credentials.expiry(), Some(expires_on.into()));
    }

    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_storage_storage_key() {
        use crate::common::azure::StorageCredentials;

        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_maybe_anon_storage_credentials(AzureMaybeAnonStorageCredentialsConfig {
                storage_key: Some(AzureStorageKeyCredentialsConfig {
                    storage_key: Secret::new("sk"),
                }),
                ..Default::default()
            })
            .unwrap();

        assert_matches!(credentials, Some(StorageCredentials::StorageKey(key)) if key.inner() == "sk");
    }

    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_storage_access_token_file() {
        use crate::common::azure::StorageCredentials;

        let (token, expires_on) = ScopedTokenFile::create_az().await;
        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_maybe_anon_storage_credentials(AzureMaybeAnonStorageCredentialsConfig {
                access_token_file: Some(AzureTokenFileCredentialsConfig {
                    path: token.file_path.clone(),
                }),
                ..Default::default()
            })
            .unwrap();

        let token_credential = match credentials {
            Some(StorageCredentials::TokenCredential(credential)) => credential,
            _ => panic!("unexpected credential kind"),
        };

        let access_token = token_credential.get_token(&["a", "b", "c"]).await.unwrap();

        assert_eq!(access_token.token.secret(), AZURE_ACCESS_TOKEN);
        assert_eq!(access_token.expires_on, expires_on);
    }

    #[cfg(feature = "cosmosdb")]
    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_cosmosdb_primary_key() {
        use crate::backend::metadata::azure_cosmosdb::Credentials;

        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_cosmosdb_credentials(AzureCosmosDBCredentialsConfig {
                primary_key: Some(AzureCosmosDBPrimaryKeyCredentialsConfig { key: Secret::new("pk") }),
                ..Default::default()
            })
            .unwrap();

        assert_matches!(credentials, Credentials::PrimaryKey(key) if key.inner() == "pk");
    }

    #[cfg(feature = "cosmosdb")]
    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_cosmosdb_access_token_file() {
        use crate::backend::metadata::azure_cosmosdb::Credentials;

        let (token, expires_on) = ScopedTokenFile::create_az().await;
        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_cosmosdb_credentials(AzureCosmosDBCredentialsConfig {
                access_token_file: Some(AzureTokenFileCredentialsConfig {
                    path: token.file_path.clone(),
                }),
                ..Default::default()
            })
            .unwrap();

        let token_credential = match credentials {
            Credentials::TokenCredential(credential) => credential,
            _ => panic!("unexpected credential kind"),
        };

        let access_token = token_credential.get_token(&["a", "b", "c"]).await.unwrap();

        assert_eq!(access_token.token.secret(), AZURE_ACCESS_TOKEN);
        assert_eq!(access_token.expires_on, expires_on);
    }

    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_service_bus_policy_key() {
        use crate::pubsub::azure_service_bus::Credentials;

        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_service_bus_credentials(AzureServiceBusCredentialsConfig {
                shared_access_policy_key: Some(AzureSASPolicyCredentialsConfig {
                    policy_name: "pn".into(),
                    policy_key: Secret::new("pk"),
                }),
                ..Default::default()
            })
            .unwrap();

        assert_matches!(
            credentials,
            Credentials::SASPolicy{ policy_name, policy_key }
            if policy_name == "pn" && policy_key.inner() == "pk"
        );
    }

    #[test_log::test(tokio::test)]
    async fn test_az_credentials_factory_service_bus_access_token_file() {
        use crate::pubsub::azure_service_bus::Credentials;

        let (token, expires_on) = ScopedTokenFile::create_az().await;
        let mut factory = AzureCredentialsFactory::default();

        let credentials = factory
            .try_create_service_bus_credentials(AzureServiceBusCredentialsConfig {
                access_token_file: Some(AzureTokenFileCredentialsConfig {
                    path: token.file_path.clone(),
                }),
                ..Default::default()
            })
            .unwrap();

        let token_credential = match credentials {
            Credentials::TokenCredential(credential) => credential,
            _ => panic!("unexpected credential kind"),
        };

        let access_token = token_credential.get_token(&["a", "b", "c"]).await.unwrap();

        assert_eq!(access_token.token.secret(), AZURE_ACCESS_TOKEN);
        assert_eq!(access_token.expires_on, expires_on);
    }
}
