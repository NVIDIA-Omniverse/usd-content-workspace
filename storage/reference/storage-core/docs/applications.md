# Writing Applications with Storage-Core

This guide explains how to create applications using the storage-core library. Storage-core provides a flexible, layered architecture for building storage services with support for multiple backends, caching, metrics, and other cross-cutting concerns.

## Overview

Storage-core applications typically follow this pattern:
1. **Configuration setup**: Load configuration from files, environment variables, etc.
2. **Backend selection**: Choose and configure a storage backend (Azure, S3, etc.)
3. **Layer configuration**: Add layers for caching, metrics, throttling, etc.
4. **Service building**: Build and serve the storage service

## Basic Application Structure

Here's a minimal example of a storage-core application:

```rust
use std::sync::Arc;
use clap::Parser;
use tracing::info;
use storage_core::storage::StorageService;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder;
use storage_core_util::{config, secrets::Secrets};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(long, default_value = "0.0.0.0:50051")]
    pub grpc_addr: String,
    
    #[arg(long, default_value = "0.0.0.0:8011")]
    pub http_addr: String,

    #[command(flatten)]
    pub config: config::ConfigArgs,
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    let args = Args::parse();
    info!("Starting storage service with args: {:?}", args);

    // Load configuration and secrets
    let figment = config::generate_figment(&args.config)?;
    let secrets = Secrets::from_figment(&figment)?;

    // Select and configure backend
    let backend = Arc::new(MyBackend::new(&figment, &secrets)?);

    // Build and serve the service
    StorageService::builder()
        .add_service(
            FileObjectBuilder::new()
                .backend(backend)
        )
        .serve(
            Some(args.grpc_addr.parse()?),
            Some(args.http_addr.parse()?),
            None, // No metrics server
        )
        .await?;

    Ok(())
}
```

## Configuration Setup

Storage-core uses Figment for configuration management, which supports multiple sources:

### Command Line Arguments

```rust
use clap::Parser;
use storage_core_util::config;

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(long, default_value = "0.0.0.0:50051")]
    pub grpc_addr: String,
    
    #[arg(long, default_value = "0.0.0.0:8011")]
    pub http_addr: String,

    #[command(flatten)]
    pub config: config::ConfigArgs, // Provides --config, --env, etc.
}

let args = Args::parse();
let figment = config::generate_figment(&args.config)?;
```

### Configuration Sources

The `ConfigArgs` provides several configuration sources:

- `--config <file>`: Load configuration from a file (TOML, JSON, YAML)
- `--env <prefix>`: Load configuration from environment variables

Example configuration file (`config.toml`):
```toml
[azure]
emulator = false
read_redirect_size = "256KB"
write_body_max_size = "2MB"

[stat_cache]
enabled = true
capacity = "1GB"
time_to_live = "15m"

[egress_throttle]
enabled = true
```

## Backend Selection

Storage-core supports multiple backends through feature flags. You can select backends at compile time:

### Azure Backend

```rust
#[cfg(feature = "azure")]
let backend = {
    use storage_core::azure::Storage as AzureStorage;
    Arc::new(AzureStorage::new(&figment, &secrets)?)
};
```

### S3 Backend

```rust
#[cfg(feature = "s3")]
let backend = {
    use storage_core::s3::service::Storage as S3Storage;
    Arc::new(S3Storage::new(&figment, &secrets)?)
};
```

### Feature Selection

Ensure only one backend feature is enabled:

```rust
// Prevent both features from being enabled simultaneously
#[cfg(all(feature = "azure", feature = "s3"))]
tracing::warn!("Both 'azure' and 's3' features are enabled. Using 's3' feature");

#[cfg(not(any(feature = "azure", feature = "s3")))]
compile_error!("Must enable either 'azure' or 's3' feature");
```

## Layer Configuration

Layers add cross-cutting concerns to your storage service. Most layers are optional and can be conditionally applied:

### Stat Cache Layer

```rust
use storage_core::stat_cache::StatCacheLayer;
use storage_core::layer::LayerArc;

let stat_cache_layer = StatCacheLayer::from_figment(&figment)?
    .map(|l| LayerArc(Arc::new(l)));
```

Normally, you would choose either Stat Cache or List Cache, but not both.

### List Cache Layer

```rust
use storage_core::list_cache::ListCacheLayer;
use storage_core::layer::LayerArc;

let list_cache_layer = ListCacheLayer::from_figment(&figment)?
    .map(|l| LayerArc(Arc::new(l)));
```

### Egress Throttle Layer

```rust
use storage_core::egress_throttle::EgressThrottleLayer;

let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)?
    .map(|l| LayerArc(Arc::new(l)));
```

### Metrics Layers

```rust
use storage_core::metrics::{OperationMetricsLayer, SdkMetricsLayer};

let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
```

### Additional Headers Layer

```rust
use storage_core::additional_headers::AdditionalHeadersLayer;

let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)?
    .map(|l| LayerArc(Arc::new(l)));
```

### Small Object Cache Layer

```rust
use storage_core::small_object_cache::SmallObjectCacheLayer;

let small_object_cache_layer = SmallObjectCacheLayer::from_figment(&figment)?
    .map(|l| LayerArc(Arc::new(l)));
```

## Service Building

Storage-core provides a builder pattern for constructing services with layers:

### Basic Service

```rust
StorageService::builder()
    .add_service(
        FileObjectBuilder::new()
            .backend(backend)
    )
    .serve(grpc_addr, http_addr, None)
    .await?;
```

### Service with Layers

```rust
StorageService::builder()
    .add_service(
        FileObjectBuilder::new()
            .layer(operation_metrics_layer.clone())
            .option_layer(additional_headers_layer.clone())
            .option_layer(stat_cache_layer.clone())
            .option_layer(small_object_cache_layer.clone())
            .option_layer(egress_throttle_layer.clone())
            .layer(sdk_metrics_layer.clone())
            .backend(backend.clone())
    )
    .serve(grpc_addr, http_addr, metrics_server)
    .await?;
```

### Multiple Services

Storage-core supports multiple API services:

```rust
StorageService::builder()
    .add_service(
        FileObjectBuilder::new()
            .layer(operation_metrics_layer.clone())
            .option_layer(stat_cache_layer.clone())
            .backend(backend.clone())
    )
    .add_service(
        FileFolderBuilder::new()
            .layer(operation_metrics_layer.clone())
            .backend(backend.clone())
    )
    .add_service(
        CapabilitiesBuilder::new()
            .layer(operation_metrics_layer.clone())
            .backend(backend.clone())
    )
    .add_service(
        VersioningBuilder::new()
            .layer(operation_metrics_layer.clone())
            .backend(backend.clone())
    )
    .add_service(
        WriteBuilder::new()
            .layer(operation_metrics_layer.clone())
            .backend(backend.clone())
    )
    .serve(grpc_addr, http_addr, metrics_server)
    .await?;
```

## Metrics and Monitoring

Storage-core includes built-in metrics support:

### Metrics Server Setup

```rust
use storage_core_util::metrics::MetricsServer;

#[cfg(feature = "azure")]
let metrics_server = MetricsServer::from_figment(&figment, "azure")?;

#[cfg(feature = "s3")]
let metrics_server = MetricsServer::from_figment(&figment, "s3")?;
```

### Custom gRPC Server Configuration

```rust
StorageService::builder()
    .customize_grpc_server(|server| {
        server.timeout(std::time::Duration::from_secs(10))
    })
    .add_service(/* ... */)
    .serve(grpc_addr, http_addr, metrics_server)
    .await?;
```

## Complete Example

Here's a complete example showing all the patterns:

```rust
use std::sync::Arc;
use clap::Parser;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use storage_core::additional_headers::AdditionalHeadersLayer;
use storage_core::api_services::capabilities::v1alpha::CapabilitiesBuilder;
use storage_core::api_services::filefolder::v1alpha::FileFolderBuilder;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder;
use storage_core::api_services::versioning::v1alpha::VersioningBuilder;
use storage_core::egress_throttle::EgressThrottleLayer;
use storage_core::layer::LayerArc;
use storage_core::list_cache::ListCacheLayer;
use storage_core::metrics::{OperationMetricsLayer, SdkMetricsLayer};
use storage_core::small_object_cache::SmallObjectCacheLayer;
use storage_core::stat_cache::StatCacheLayer;
use storage_core::storage::StorageService;
use storage_core_util::metrics::MetricsServer;
use storage_core_util::{config, secrets::Secrets};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(long, default_value = "0.0.0.0:50051")]
    pub grpc_addr: String,
    
    #[arg(long, default_value = "0.0.0.0:8011")]
    pub http_addr: String,

    #[command(flatten)]
    pub config: config::ConfigArgs,
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    info!("Starting storage service with args: {:?}", args);

    // Load configuration and secrets
    let figment = config::generate_figment(&args.config)?;
    let secrets = Secrets::from_figment(&figment)?;

    // Prevent multiple backend features
    #[cfg(all(feature = "azure", feature = "s3"))]
    tracing::warn!("Both 'azure' and 's3' features are enabled. Using 's3' feature");

    // Setup metrics server
    #[cfg(feature = "azure")]
    let metrics_server = MetricsServer::from_figment(&figment, "azure")?;

    #[cfg(feature = "s3")]
    let metrics_server = MetricsServer::from_figment(&figment, "s3")?;

    // Select backend
    #[cfg(feature = "azure")]
    let backend = {
        use storage_core::azure::Storage as AzureStorage;
        Arc::new(AzureStorage::new(&figment, &secrets)?)
    };

    #[cfg(feature = "s3")]
    let backend = {
        use storage_core::s3::service::Storage as S3Storage;
        Arc::new(S3Storage::new(&figment, &secrets)?)
    };

    #[cfg(not(any(feature = "azure", feature = "s3")))]
    compile_error!("Must enable either 'azure' or 's3' feature");

    // Configure layers (for stat and list cache, assumption is that only one of these layers has enabled=true)
    let list_cache_layer = ListCacheLayer::from_figment(&figment)?
        .map(|l| LayerArc(Arc::new(l)));
    let stat_cache_layer = StatCacheLayer::from_figment(&figment)?
        .map(|l| LayerArc(Arc::new(l)));
    let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)?
        .map(|l| LayerArc(Arc::new(l)));
    let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    let additional_headers_layer = AdditionalHeadersLayer::from_figment(&figment)?
        .map(|l| LayerArc(Arc::new(l)));
    let small_object_cache_layer = SmallObjectCacheLayer::from_figment(&figment)?
        .map(|l| LayerArc(Arc::new(l)));

    info!(
        "Starting storage service on gRPC: {}, HTTP: {}, Metrics: {:?}",
        args.grpc_addr, args.http_addr, metrics_server
    );

    // Build and serve the service
    StorageService::builder()
        .customize_grpc_server(|server| {
            server.timeout(std::time::Duration::from_secs(10))
        })
        .add_service(
            FileObjectBuilder::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(stat_cache_layer.clone())
                .option_layer(small_object_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone())
        )
        .add_service(
            FileFolderBuilder::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(list_cache_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone())
        )
        .add_service(
            CapabilitiesBuilder::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone())
        )
        .add_service(
            VersioningBuilder::new()
                .layer(operation_metrics_layer.clone())
                .option_layer(additional_headers_layer.clone())
                .option_layer(egress_throttle_layer.clone())
                .layer(sdk_metrics_layer.clone())
                .backend(backend.clone())
        )
        .serve(
            Some(args.grpc_addr.parse()?),
            Some(args.http_addr.parse()?),
            metrics_server,
        )
        .await?;

    Ok(())
}
```

## Best Practices

1. **Use feature flags for backends**: This allows compile-time selection and reduces binary size
2. **Make layers optional**: Use `.option_layer()` for layers that might not be configured
3. **Reuse layer instances**: Clone layer instances across multiple services to share state
4. **Configure logging early**: Set up tracing before any other operations
5. **Handle errors gracefully**: Use proper error handling throughout the application
6. **Use configuration files**: Store configuration in files rather than hardcoding values
7. **Monitor with metrics**: Always include metrics for production applications
8. **Test with different backends**: Ensure your application works with multiple backend types

## Deployment Considerations

- **Configuration management**: Use environment variables for sensitive configuration
- **Health checks**: Implement health check endpoints for container orchestration
- **Graceful shutdown**: Handle shutdown signals properly
- **Resource limits**: Configure appropriate timeouts and connection limits
- **Security**: Use TLS for production deployments
- **Monitoring**: Set up proper logging and metrics collection

This pattern provides a flexible, maintainable way to build storage services with storage-core, allowing you to easily add or remove functionality through layers while maintaining a clean separation of concerns.
