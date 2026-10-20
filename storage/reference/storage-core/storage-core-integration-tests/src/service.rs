// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use figment::Figment;
use fred::clients::Pool;
use storage_core::ingress::IdentityLayer;
use storage_core::storage::StorageServiceServer;

use storage_core::additional_headers::AdditionalHeadersLayer;
use storage_core::api_services::capabilities::v1alpha::CapabilitiesBuilder as CapabilitiesBuilderV1Alpha;
use storage_core::api_services::capabilities::v1beta::CapabilitiesBuilder as CapabilitiesBuilderV1Beta;
use storage_core::api_services::filefolder::v1alpha::FileFolderBuilder as FileFolderBuilderV1Alpha;
use storage_core::api_services::filefolder::v1beta::FileFolderBuilder as FileFolderBuilderV1Beta;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder as FileObjectBuilderV1Alpha;
use storage_core::api_services::fileobject::v1beta::FileObjectBuilder as FileObjectBuilderV1Beta;
use storage_core::api_services::metadata::v1alpha::MetadataBuilder as MetadataBuilderV1Alpha;
use storage_core::api_services::versioning::v1alpha::VersioningBuilder as VersioningBuilderV1Alpha;
use storage_core::api_services::versioning::v1beta::VersioningBuilder as VersioningBuilderV1Beta;
use storage_core::egress_throttle::EgressThrottleLayer;
use storage_core::layer::Layer;
use storage_core::layer::LayerArc;
use storage_core::list_cache::ListCacheLayer;
use storage_core::manifest::ManifestLayer;
use storage_core::metadata_cache::MetadataCacheLayer;
use storage_core::metadata_key_remapper::MetadataKeyRemapperLayer;
use storage_core::metadata_key_router::MetadataKeyRouter;
use storage_core::metrics::{OperationMetricsLayer, SdkMetricsLayer};
use storage_core::small_object_cache::SmallObjectCacheLayer;
use storage_core::stat_cache::StatCacheLayer;
use storage_core::storage::StorageService;
use storage_core::traits::{
    capabilities::v1alpha::Capabilities as CapabilitiesV1Alpha,
    capabilities::v1beta::Capabilities as CapabilitiesV1Beta, filefolder::v1alpha::FileFolder as FileFolderV1Alpha,
    filefolder::v1beta::FileFolder as FileFolderV1Beta, fileobject::v1alpha::FileObject as FileObjectV1Alpha,
    fileobject::v1beta::FileObject as FileObjectV1Beta, metadata::v1alpha::Metadata as MetadataV1Alpha,
    versioning::v1alpha::Versioning as VersioningV1Alpha, versioning::v1beta::Versioning as VersioningV1Beta,
};
use storage_core_util::config::ConfigArgs;
use storage_core_util::{config, secrets};

use crate::mocks::metadata::MockMetadata;

/// Create a storage service for integration tests. Async because cache layer
/// construction may need to connect to Redis when `redis_client` is `Some`.
pub async fn create_storage_service<F, T>(
    config_file: String,
    backend_label: &str,
    generate_backend: F,
    redis_client: Option<Pool>,
) -> StorageServiceServer<IdentityLayer>
where
    T: CapabilitiesV1Alpha
        + CapabilitiesV1Beta
        + FileObjectV1Alpha
        + FileObjectV1Beta
        + FileFolderV1Alpha
        + FileFolderV1Beta
        + MetadataV1Alpha
        + VersioningV1Alpha
        + VersioningV1Beta,
    F: FnOnce(&Figment, &Option<Figment>) -> Result<Arc<T>, Box<dyn std::error::Error>> + Send + 'static,
{
    let figment = config::generate_figment(&ConfigArgs {
        configuration_file: Some(config_file.into()),
        env_prefix: "".to_string(),
    })
    .unwrap();
    let secrets = secrets::Secrets::from_figment(&figment).unwrap();
    let backend = generate_backend(&figment, secrets.figment()).unwrap();

    let list_cache_layer = ListCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let stat_cache_layer = StatCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new(backend_label)));
    let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new(backend_label)));
    let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let small_object_cache_layer = SmallObjectCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let metadata_cache_layer = MetadataCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let metadata_key_remapper_layer = MetadataKeyRemapperLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));

    StorageService::builder()
        .with_error_severity_from_figment(&figment)
        .add_service(
            FileObjectBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(small_object_cache_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(stat_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileObjectBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(small_object_cache_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(stat_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            MetadataBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(metadata_cache_layer.clone())
                .option_layer(metadata_key_remapper_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
}

pub async fn create_manifest_service<F, T>(
    config_file: String,
    generate_backend: F,
    redis_client: Option<Pool>,
) -> StorageServiceServer<IdentityLayer>
where
    T: CapabilitiesV1Alpha
        + CapabilitiesV1Beta
        + FileObjectV1Alpha
        + FileObjectV1Beta
        + FileFolderV1Alpha
        + FileFolderV1Beta
        + VersioningV1Alpha
        + VersioningV1Beta,
    F: FnOnce(&Figment, &Option<Figment>) -> Result<Arc<T>, Box<dyn std::error::Error>> + Send + 'static,
{
    let figment = config::generate_figment(&ConfigArgs {
        configuration_file: Some(config_file.into()),
        env_prefix: "".to_string(),
    })
    .unwrap();
    let secrets = secrets::Secrets::from_figment(&figment).unwrap();
    let backend = generate_backend(&figment, secrets.figment()).unwrap();

    let manifest_layer = ManifestLayer::from_figment_with_redis(&figment, redis_client)
        .await
        .unwrap()
        // The resolver handles only the manifest layer's own reads; storage-service inserts
        // remapping and permissions here. This chain has only the backend.
        .map(|l| LayerArc(Arc::new(l.with_resolver(backend.clone()))));

    let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));

    StorageService::builder()
        .with_error_severity_from_figment(&figment)
        .add_service(
            FileObjectBuilderV1Alpha::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileObjectBuilderV1Beta::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Alpha::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Beta::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Alpha::new()
                .option_layer(additional_headers_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Beta::new()
                .option_layer(additional_headers_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Alpha::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Beta::new()
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .backend(backend.clone()),
        )
}

/// Create a storage service whose metadata service is the `MetadataKeyRouter`.
///
/// `nv-omniverse-nucleus-hash` is routed to the real backend (with the
/// `metadata_key_remapper` layer in front, mirroring the production wiring sketch in
/// `docs/routing-layer-design.md`); every other key falls through to `mock`.
///
/// All non-metadata services use the real backend, identical to
/// [`create_storage_service`].
pub async fn create_metadata_router_storage_service<F, T>(
    config_file: String,
    backend_label: &str,
    generate_backend: F,
    mock: Arc<MockMetadata>,
    redis_client: Option<Pool>,
) -> StorageServiceServer<IdentityLayer>
where
    T: CapabilitiesV1Alpha
        + CapabilitiesV1Beta
        + FileObjectV1Alpha
        + FileObjectV1Beta
        + FileFolderV1Alpha
        + FileFolderV1Beta
        + MetadataV1Alpha
        + VersioningV1Alpha
        + VersioningV1Beta,
    F: FnOnce(&Figment, &Option<Figment>) -> Result<Arc<T>, Box<dyn std::error::Error>> + Send + 'static,
{
    let figment = config::generate_figment(&ConfigArgs {
        configuration_file: Some(config_file.into()),
        env_prefix: "".to_string(),
    })
    .unwrap();
    let secrets = secrets::Secrets::from_figment(&figment).unwrap();
    let backend = generate_backend(&figment, secrets.figment()).unwrap();

    let list_cache_layer = ListCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let stat_cache_layer = StatCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new(backend_label)));
    let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new(backend_label)));
    let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let small_object_cache_layer = SmallObjectCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));
    let metadata_cache_layer = MetadataCacheLayer::from_figment_with_redis(&figment, redis_client.clone())
        .await
        .unwrap()
        .map(|l| LayerArc(Arc::new(l)));

    // Build the routed branch: the real backend with the metadata_key_remapper layer
    // applied directly. The remapper layer is loaded fresh here (not shared with the
    // metadata service stack) because it lives *inside* the router on this branch.
    let remapper = MetadataKeyRemapperLayer::from_figment(&figment)
        .unwrap()
        .expect("metadata_key_remapper config required for router runner");
    let routed_branch: Arc<dyn MetadataV1Alpha + Send + Sync + 'static> = Arc::new(remapper.layer(backend.clone()));
    let default_branch: Arc<dyn MetadataV1Alpha + Send + Sync + 'static> = mock;

    let router = MetadataKeyRouter::builder()
        .route(["nv-omniverse-nucleus-hash"], routed_branch)
        .default(default_branch)
        .build()
        .expect("MetadataKeyRouter build succeeds for fixed test wiring");

    StorageService::builder()
        .with_error_severity_from_figment(&figment)
        .add_service(
            FileObjectBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(small_object_cache_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(stat_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileObjectBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(small_object_cache_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(stat_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            CapabilitiesBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            MetadataBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(metadata_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(router),
        )
}
