// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use clap::Parser;
use figment::Figment;
use storage_core::additional_headers::AdditionalHeadersLayer;
use storage_core::api_services::capabilities::v1alpha::CapabilitiesBuilder as CapabilitiesBuilderV1Alpha;
use storage_core::api_services::capabilities::v1beta::CapabilitiesBuilder as CapabilitiesBuilderV1Beta;
use storage_core::api_services::filefolder::v1alpha::FileFolderBuilder as FileFolderBuilderV1Alpha;
use storage_core::api_services::filefolder::v1beta::FileFolderBuilder as FileFolderBuilderV1Beta;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder as FileObjectBuilderV1Alpha;
use storage_core::api_services::fileobject::v1beta::FileObjectBuilder as FileObjectBuilderV1Beta;
use storage_core::api_services::versioning::v1alpha::VersioningBuilder as VersioningBuilderV1Alpha;
use storage_core::api_services::versioning::v1beta::VersioningBuilder as VersioningBuilderV1Beta;
use storage_core::egress_throttle::EgressThrottleLayer;
use storage_core::layer::{Layer, LayerArc};
use storage_core::manifest::ManifestLayer;
use storage_core::metrics::{OperationMetricsLayer, SdkMetricsLayer};
use storage_core::storage::StorageService;
use storage_core::traits::{
    capabilities::v1alpha::Capabilities as CapabilitiesV1Alpha,
    capabilities::v1beta::Capabilities as CapabilitiesV1Beta, filefolder::v1alpha::FileFolder as FileFolderV1Alpha,
    filefolder::v1beta::FileFolder as FileFolderV1Beta, fileobject::v1alpha::FileObject as FileObjectV1Alpha,
    fileobject::v1beta::FileObject as FileObjectV1Beta, versioning::v1alpha::Versioning as VersioningV1Alpha,
    versioning::v1beta::Versioning as VersioningV1Beta, Either,
};
use storage_core_util::{
    cache::build_redis_client_from_figment,
    config, info_limited, init_observability,
    observability::{metrics::meter, set_metric_prefix},
    secrets,
};

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "0.0.0.0:50051")]
    pub grpc_addr: String,
    #[arg(long, default_value = "0.0.0.0:8011")]
    pub http_addr: String,

    #[command(flatten)]
    pub config: config::ConfigArgs,
}

pub async fn run<F, T>(
    service_name: &str,
    metrics_backend_label: &str,
    generate_backend: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: FileObjectV1Alpha
        + FileFolderV1Alpha
        + CapabilitiesV1Alpha
        + VersioningV1Alpha
        + FileObjectV1Beta
        + FileFolderV1Beta
        + CapabilitiesV1Beta
        + VersioningV1Beta
        + secrets::SecretsReloadable,
    F: FnOnce(&Figment, &Option<Figment>) -> Result<Arc<T>, Box<dyn std::error::Error>>,
{
    let _guard = init_observability!(service_name);
    set_metric_prefix("manifest.storage")?;

    let args = Args::parse();
    info_limited!("Starting manifest-service with args: {:?}", args);

    let figment = config::generate_figment(&args.config)?;
    let secrets = secrets::Secrets::from_figment(&figment)?;
    let backend = generate_backend(&figment, secrets.figment())?;
    let _watcher_handle = secrets.watch(backend.clone());

    let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new(metrics_backend_label)));
    let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new(metrics_backend_label)));
    let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)?.map(|l| LayerArc(Arc::new(l)));
    // The manifest cache uses this shared client only when `manifest.l2_enabled` is set.
    let redis = build_redis_client_from_figment(&figment).await?;
    let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)?.map(|l| LayerArc(Arc::new(l)));
    let metered = sdk_metrics_layer.layer(backend.clone());
    let resolver = Arc::new(match &egress_throttle_layer {
        Some(layer) => Either::Left(layer.layer(metered)),
        None => Either::Right(metered),
    });
    let manifest_layer = ManifestLayer::from_figment_with_redis(&figment, redis)
        .await?
        // The resolver handles only the manifest layer's own reads; storage-service inserts
        // remapping and permissions here.
        .map(|l| LayerArc(Arc::new(l.with_resolver(resolver.clone()))));

    info_limited!(
        "Starting storage service on gRPC: {}, HTTP: {}",
        args.grpc_addr,
        args.http_addr,
    );

    StorageService::builder()
        .with_error_severity_from_figment(&figment)
        .with_trace_propagation()
        .with_metrics(meter!())
        .customize_grpc_server(|server| server.timeout(std::time::Duration::from_secs(10)))
        .add_service(
            FileObjectBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileObjectBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Alpha::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            FileFolderBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
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
                .option_layer(manifest_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .add_service(
            VersioningBuilderV1Beta::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(manifest_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone()),
        )
        .serve(
            Some(args.grpc_addr.parse::<std::net::SocketAddr>().unwrap()),
            Some(args.http_addr.parse::<std::net::SocketAddr>().unwrap()),
        )
        .await
        .unwrap();

    Ok(())
}
