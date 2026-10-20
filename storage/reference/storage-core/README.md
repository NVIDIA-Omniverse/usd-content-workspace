# Storage Core

A Rust framework for building high-performance storage services with unified gRPC and REST APIs. Storage Core provides composable middleware layers, multiple backend support (Azure, S3), and shared business logic across both protocols.

## Overview

Storage Core is designed to simplify the development of storage services by providing:

- **Unified Protocol Support**: Serve both gRPC and REST endpoints from a single service implementation
- **Composable Middleware**: Build complex service stacks with reusable middleware layers
- **Multiple Backend Support**: Azure Blob Storage, AWS S3, and custom backends
- **Built-in Features**: Caching, metrics, throttling, and more

## Quick Start

```rust
use std::sync::Arc;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder;
use storage_core::azure::Storage as AzureStorage;
use storage_core::storage::StorageService;
use storage_core_util::secrets::Secrets;

use figment::{
    providers::{Format, Toml},
    Figment,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let figment = Figment::new().merge(Toml::file("config.toml"));
    let secrets = Secrets::from_figment(&figment)?;

    // Initialize backend
    let backend = Arc::new(AzureStorage::new(&figment, &secrets)?);

    // Build and serve the service
    StorageService::builder()
        .add_service(FileObjectBuilder::new().backend(backend))
        .serve(
            Some("127.0.0.1:50051".parse::<std::net::SocketAddr>().unwrap()),
            Some("127.0.0.1:8011".parse::<std::net::SocketAddr>().unwrap()),
            None,
        )
        .await?;

    Ok(())
}
```

## Configuration

See the [configuration](./docs/configuration.md) documentation for all the configuration options and how to use them.

## Secrets

See the [secrets](./docs/secrets.md) documentation for all the secrets options and how to use them.


## Documentation

- **[Writing Applications](./docs/applications.md)**: Complete guide to building storage services with storage-core
- **[Writing Backends](./docs/backends.md)**: How to implement custom storage backends
- **[Writing Layers](./docs/layers.md)**: Creating middleware layers for cross-cutting concerns
- **[Error Handling](./docs/errors.md)**: Errors returned by each gRPC/REST method
- **[Metrics](./docs/metrics/metrics.md)**: How to use the metrics system

## Architecture

See the [architecture](./docs/architecture.md) for more details.

## License

This project is licensed under the Apache License, Version 2.0. See [LICENSE](./LICENSE).
