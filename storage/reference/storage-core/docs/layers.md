# Writing Your Own Layers

This guide explains how to create custom layers for the storage-core system. Layers provide a way to add cross-cutting concerns like caching, logging, metrics, and error handling to storage services.

## Overview

Layers in storage-core follow the Tower middleware pattern, where a layer wraps a service and can modify requests/responses or add functionality. The key components are:

- **Layer**: Holds shared state and creates service instances
- **Service**: Implements the actual storage traits and contains the business logic
- **Traits**: Define the interface that services must implement

## Implementing the Layer Trait

The `Layer<S>` trait is the foundation for all layers. It takes an inner service `S` and returns a new service that wraps it.

```rust
use storage_core_builder::layer::Layer;
use std::sync::Arc;

pub struct MyLayer {
    // Shared state that will be used by all service instances
    config: Arc<MyConfig>,
    cache: Arc<MyCache>,
}

impl<S> Layer<S> for MyLayer {
    type Service = MyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MyService {
            inner,
            config: self.config.clone(),
            cache: self.cache.clone(),
        }
    }
}
```

### Key Points:

- **Keep shared state in the layer**: The layer should hold any state that needs to be shared across multiple service instances (like caches, configuration, etc.). Each implementation of a Storage trait will have its own Service struct, so any state that must be shared across multiple service instances should be held in the layer and passed to the service.
- **Pass state to service**: Use `Arc` to share immutable state between the layer and service instances
- **Clone for sharing**: The layer should implement `Clone` if you need to create multiple instances

## Implementing Storage Traits

Your service only needs to implement the traits that you want to modify or intercept. Your layer will only be able to be added to the services that you implement the trait of.

A layer receives the whole request and passes it on. Context that the layer derives for the layers and backend below, such as `AdditionalHeaders` or `CacheControl`, goes into `request.extensions()`, and caller metadata stays in `request.metadata()`. If you add or change a trait method rather than implementing an existing one, follow [Trait method conventions](./architecture.md#trait-method-conventions).

### Implementing the `inner()` Function

Every storage trait requires an `inner()` method that returns an optional reference to the inner service. This is how the default implementation of the trait differentiates between layer implementations (which delegate to the inner service) and backend implementations (which do not have an inner service and return `Unimplemented`). Doing a default implementation in this way is a workaround for [specialization](https://rust-lang.github.io/rfcs/1210-impl-specialization.html) not being implemented in Rust.

```rust
use storage_core_builder::traits::fileobject::v1alpha::FileObject as FileObjectV1Alpha;

#[async_trait::async_trait]
impl<S> FileObjectV1Alpha for MyService<S>
where
    S: FileObjectV1Alpha,
{
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        Some(&self.inner)
    }

    // Only implement methods you want to modify
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        // Your custom logic here
        // You can call self.inner.stat(request).await to delegate
    }

    // For methods you don't implement, the default implementation
    // will automatically delegate to self.inner()
}
```

### Only Implement What You Need

You don't need to implement every method in a trait. The trait definition provides default implementations that delegate to the inner service:

```rust
// This is all you need if you only want to modify the 'stat' method
#[async_trait::async_trait]
impl<S> FileObjectV1Alpha for MyService<S>
where
    S: FileObjectV1Alpha,
{
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        // Your custom stat logic
        let response = self.inner.stat(request).await?;
        // Modify response if needed
        Ok(response)
    }
    
    // All other methods will automatically delegate to self.inner()
}
```

## Error Handling

Storage-core uses a custom error handling system based on the `StorageErrorExt` trait.

### StorageErrorExt Trait

All layer errors must implement `StorageErrorExt`. You can use any error library you prefer (thiserror, anyhow, custom implementations, etc.), but storage-core uses [snafu](https://docs.rs/snafu) internally because it provides automatic source location tracking.

Here's an example using snafu:

```rust
use snafu::prelude::*;
use storage_core_builder::traits::error::{SourceLocation, StorageErrorExt, StorageError};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum MyLayerError {
    #[snafu(display("cache error: {message}"))]
    CacheError {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
    
    #[snafu(display("validation error: {message}"))]
    ValidationError {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

// Constructor methods for creating errors with automatic location capture, use the #[track_caller] attribute to ensure the location is captured correctly.
impl MyLayerError {
    #[track_caller]
    pub fn cache_error(message: impl Into<String>) -> Self {
        CacheError { message }.build()
    }

    #[track_caller]
    pub fn validation_error(message: impl Into<String>) -> Self {
        ValidationError { message }.build()
    }
}

impl StorageErrorExt for MyLayerError {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            MyLayerError::CacheError { .. } => tonic::Status::internal(self.to_string()),
            MyLayerError::ValidationError { .. } => tonic::Status::invalid_argument(self.to_string()),
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            MyLayerError::CacheError { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
            MyLayerError::ValidationError { .. } => http::StatusCode::BAD_REQUEST,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            MyLayerError::CacheError { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
            MyLayerError::ValidationError { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            MyLayerError::CacheError { loc, .. } => Some(loc.into()),
            MyLayerError::ValidationError { loc, .. } => Some(loc.into()),
        }
    }
}
```

### Source Location Tracking

The `source_location` method is optional (it has a default implementation that returns `None`), but implementing it helps with debugging by recording where errors originated.

If using snafu, location tracking is automatic via the `#[snafu(implicit)]` attribute. With other error libraries, you can use `#[track_caller]` and `std::panic::Location`, or simply return `None` if you don't need this feature.

Key points:
- The `source_location()` method returns `Option<SourceLocation>` - return `None` if you don't track locations
- With snafu: the `loc: snafu::Location` field with `#[snafu(implicit)]` automatically captures the call site
- For errors that wrap other `StorageError` types, delegate to the inner error: `source.source_location()`

### Converting to StorageError

`StorageError` provides a `From` implementation for all errors that implement `StorageErrorExt`. You can use this to convert your custom errors to `StorageError` using the `?` operator:

```rust
async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
    self.do_stat(request).await?
}
```

### Logging Errors

The ingress code for both GRPC and REST will automatically log any errors that happen, including the URL(s) involved in the request and the source location where the error originated. Layer and Backend authors can include any additional information in the `display()` method of their errors, but they do not have to include the request context of the error. The pseudocode for printing error messages is:

```rust
// Gather context and perform the operation
let context = request.get_ref().resource_address;
let error = self.inner.stat();

// Get error code and source location
let code = error.tonic_status().code(); // or error.http_status() for REST
let location = error
    .source_location()
    .map_or_else(|| "unknown".to_string(), |l| l.to_string());

// Log the error with source location
error!(code = ?code, context, source_location = %location, error = %e)
```

This results in the following log message:
```
2026-01-16T18:22:53.861334Z ERROR grpc::stat{request_id=0}: storage_core_builder::api_services::error: code=NotFound context="https://example-bucket.s3.us-west-2.amazonaws.com/tmp_2/abc.txt" source_location=storage-core/src/s3/fileobject/mod.rs:613:42 error=sdk error source=not found: No elements found
```

The `source_location` field shows the file, line, and column where the error was created, making it easier to trace errors back to their origin in the code.

## Configuration

Layers can be configured using Figment, which supports multiple configuration sources (files, environment variables, etc.).

### Basic Configuration

```rust
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the layer
    #[serde(default = "default_enabled")]
    enabled: bool,
    
    /// Cache capacity in bytes
    #[serde(default = "default_capacity")]
    capacity: ByteSize,
    
    /// Time to live for cached items
    #[serde(default = "default_ttl", with = "humantime_serde")]
    time_to_live: Duration,
}

fn default_enabled() -> bool {
    true
}

fn default_capacity() -> ByteSize {
    ByteSize::gib(1)
}

fn default_ttl() -> Duration {
    Duration::from_secs(900) // 15 minutes
}

impl Config {
    pub const CONFIG_KEY: &str = "my_layer";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}
```

### Optional Layers

You can make your layer optional by returning `Option<Self>` from the constructor:

```rust
impl MyLayer {
    pub fn new(config: Config) -> Option<Self> {
        if config.enabled {
            Some(Self {
                config: Arc::new(config),
                cache: Arc::new(MyCache::new()),
            })
        } else {
            None
        }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}
```

### Required Layers

For layers that should always be present, return `Self` directly:

```rust
impl MyLayer {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            cache: Arc::new(MyCache::new()),
        }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Self, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}
```

## Complete Example

Here's a complete example of a caching layer:

```rust
use std::sync::Arc;
use snafu::prelude::*;
use storage_core_builder::{
    layer::Layer,
    traits::{
        error::{Result, SourceLocation, StorageError, StorageErrorExt},
        fileobject::v1alpha::{FileObject as FileObjectV1Alpha, StatRequest, StatResponse},
    },
};
use tonic::Request;

// Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    
    #[serde(default = "default_capacity")]
    capacity: ByteSize,
}

fn default_enabled() -> bool { true }
fn default_capacity() -> ByteSize { ByteSize::gib(1) }

impl CacheConfig {
    pub const CONFIG_KEY: &str = "cache_layer";
}

impl TryFrom<&Figment> for CacheConfig {
    type Error = figment::Error;
    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

// Error handling
#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum CacheError {
    #[snafu(display("cache miss: {message}"))]
    CacheMiss {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl CacheError {
    pub fn cache_miss(message: impl Into<String>) -> Self {
        CacheMiss { message }.build()
    }
}

impl StorageErrorExt for CacheError {
    fn tonic_status(&self) -> tonic::Status {
        tonic::Status::internal(self.to_string())
    }
    
    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::INTERNAL_SERVER_ERROR
    }
    
    fn get_size(&self) -> usize {
        match self {
            CacheError::CacheMiss { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            CacheError::CacheMiss { loc, .. } => Some(loc.into()),
        }
    }
}

// Layer
pub struct CacheLayer {
    config: Arc<CacheConfig>,
    cache: Arc<Cache<String, StatResponse>>,
}

impl CacheLayer {
    pub fn new(config: CacheConfig) -> Option<Self> {
        if config.enabled {
            Some(Self {
                config: Arc::new(config),
                cache: Arc::new(Cache::new(&config.into(), "cache_layer")),
            })
        } else {
            None
        }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = CacheConfig::try_from(figment)?;
        Ok(Self::new(config))
    }
}

impl<S> Layer<S> for CacheLayer {
    type Service = CacheService<S>;
    
    fn layer(&self, inner: S) -> Self::Service {
        CacheService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

// Service
pub struct CacheService<S> {
    inner: S,
    cache: Arc<Cache<String, StatResponse>>,
}

#[async_trait::async_trait]
impl<S> FileObjectV1Alpha for CacheService<S>
where
    S: FileObjectV1Alpha,
{
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let key = request.get_ref().resource_address.clone();
        
        Ok(self.cache
            .try_get_with_by_ref(
                &key,
                async move { self.inner.stat(request).await },
            )
            .await
            .map_err(|e| CacheError::cache_miss(e.to_string()))?)
    }
    
    // Other methods automatically delegate to self.inner()
}
```

## Best Practices

1. **Keep layers focused**: Each layer should have a single responsibility
2. **Use Arc for shared state**: This allows efficient sharing between layer and service instances
3. **Implement only what you need**: Don't implement methods you don't modify
4. **Handle errors properly**: Always implement `StorageErrorExt` for custom errors
5. **Make configuration flexible**: Use sensible defaults and allow disabling the layer
6. **Use tracing**: Add instrumentation to your methods for observability
7. **Test thoroughly**: Layers can affect all operations, so comprehensive testing is important

## Integration

To use your layer in a service stack:

```rust
let layer = CacheLayer::from_figment(&figment)?;
let service = if let Some(layer) = layer {
    layer.layer(inner_service)
} else {
    inner_service
};
```

This pattern allows layers to be conditionally applied based on configuration, making your storage service flexible and configurable.
