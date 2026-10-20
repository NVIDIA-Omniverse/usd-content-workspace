# Writing Your Own Backends

This guide explains how to create custom backends for the storage-core system. Backends are the foundational storage implementations that provide the actual storage functionality, unlike layers which add cross-cutting concerns.

## Overview

Backends in storage-core are the concrete implementations of storage services that actually perform storage operations. They are the "leaf" nodes in the service stack - they don't wrap other services, but instead implement the storage logic directly.

Key differences from layers:
- **Backends implement actual storage logic**: They connect to real storage systems (S3, Azure, local filesystem, etc.)
- **Backends return `None` from `inner()`**: This allows default implementations to return `UnimplementedError` for methods they don't support
- **Backends are the foundation**: Layers wrap backends, but backends don't wrap other services

## Implementing Storage Traits

Backends must implement the storage traits they want to support. Unlike layers, backends should return `None` from the `inner()` method, which allows the default trait implementations to return `UnimplementedError` for any methods you don't implement.

Each method receives the whole request. Caller metadata is in `request.metadata()`, and context added by the layers above is in `request.extensions()`: for example, a backend takes the configured outbound headers with `request.extensions_mut().remove::<AdditionalHeaders>()`. Likewise, `read` and `read_from_address` should take any cached `ResourceInfo` with `request.extensions_mut().remove::<ResourceInfo>()` and use it in place of a stat, because a cache layer above has already resolved it. If you add or change a trait method rather than implementing an existing one, follow [Trait method conventions](./architecture.md#trait-method-conventions).

### Implementing the `inner()` Function

Backends should always return `None` from the `inner()` method. This is how the default implementation of the trait differentiates between layer implementations (which delegate to the inner service) and backend implementations (which do not have an inner service and return `Unimplemented`). Doing a default implementation in this way is a workaround for [specialization](https://rust-lang.github.io/rfcs/1210-impl-specialization.html) not being implemented in Rust.

```rust
use storage_core_builder::traits::fileobject::v1alpha::FileObject as FileObjectV1Alpha;

#[async_trait::async_trait]
impl FileObjectV1Alpha for MyBackend {
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        None // Backends should always return None
    }

    // Only implement methods you want to support
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        // Your actual storage implementation here
        self.do_stat_operation(request).await
    }

    // For methods you don't implement, the default implementation
    // will return UnimplementedError
}
```

### Only Implement What You Support

You don't need to implement every method in a trait. The trait definition provides default implementations that return `UnimplementedError` for unimplemented methods:

```rust
// This backend only supports read operations
#[async_trait::async_trait]
impl FileObjectV1Alpha for ReadOnlyBackend {
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        None
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        // Implementation for stat
        self.do_stat(request).await
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        // Implementation for read
        self.do_read(request).await
    }
    
    // All other methods (write, delete, etc.) will automatically
    // return UnimplementedError
}
```

### Capabilities list_services

The list_services method is not included in the Capabilities trait. This is because it is done automatically by the `StorageService` builder. Backend authors do not need to implement that method, because the backend does not actually know what services will ultimately be registered with the `StorageService` builder. The `StorageService` builder automatically tracks the services that are registered with it, and the `list_services` method returns the services that are registered with it.

## Error Handling

Backends must handle errors from their underlying storage systems and convert them to the storage-core error format.

### StorageErrorExt Trait

All backend errors must implement `StorageErrorExt`. You can use any error library you prefer (thiserror, anyhow, custom implementations, etc.), but storage-core uses [snafu](https://docs.rs/snafu) internally because it provides automatic source location tracking.

Here's an example using snafu:

```rust
use snafu::prelude::*;
use storage_core_builder::traits::error::{SourceLocation, StorageErrorExt, StorageError};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum MyBackendError {
    #[snafu(display("storage not found: {message}"))]
    NotFound {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
    
    #[snafu(display("permission denied: {message}"))]
    PermissionDenied {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
    
    #[snafu(display("storage system error: {message}"))]
    StorageError {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

// Constructor methods for creating errors with automatic location capture, use the #[track_caller] attribute to ensure the location is captured correctly.
impl MyBackendError {
    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        NotFound { message }.build()
    }

    #[track_caller]
    pub fn permission_denied(message: impl Into<String>) -> Self {
        PermissionDenied { message }.build()
    }

    #[track_caller]
    pub fn storage_error(message: impl Into<String>) -> Self {
        StorageError { message }.build()
    }
}

impl StorageErrorExt for MyBackendError {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            MyBackendError::NotFound { .. } => tonic::Status::not_found(self.to_string()),
            MyBackendError::PermissionDenied { .. } => tonic::Status::permission_denied(self.to_string()),
            MyBackendError::StorageError { .. } => tonic::Status::internal(self.to_string()),
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            MyBackendError::NotFound { .. } => http::StatusCode::NOT_FOUND,
            MyBackendError::PermissionDenied { .. } => http::StatusCode::FORBIDDEN,
            MyBackendError::StorageError { .. } => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            MyBackendError::NotFound { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
            MyBackendError::PermissionDenied { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
            MyBackendError::StorageError { message, .. } => std::mem::size_of::<Self>() + message.capacity(),
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            MyBackendError::NotFound { loc, .. } => Some(loc.into()),
            MyBackendError::PermissionDenied { loc, .. } => Some(loc.into()),
            MyBackendError::StorageError { loc, .. } => Some(loc.into()),
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

### Converting External Errors

Backends often need to convert errors from external storage systems:

```rust
use aws_sdk_s3::Error as S3Error;

impl From<S3Error> for MyBackendError {
    fn from(error: S3Error) -> Self {
        match error {
            S3Error::NoSuchKey(_) => MyBackendError::not_found("Object not found"),
            S3Error::AccessDenied(_) => MyBackendError::permission_denied("Access denied"),
            _ => MyBackendError::storage_error(error.to_string()),
        }
    }
}
```

If using snafu and you want to preserve the source error, use the `#[snafu(transparent)]` attribute:

```rust
#[snafu(transparent)]
Io {
    #[snafu(source(from(std::io::Error, Arc::new)))]
    source: Arc<std::io::Error>,
    #[snafu(implicit)]
    loc: snafu::Location,
},
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

### Using the `?` Operator

`StorageError` provides a `From` implementation for all errors that implement `StorageErrorExt`. You can use this to convert your custom errors to `StorageError` using the `?` operator:

```rust
async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
    // Your implementation that returns Result<StatResponse, MyBackendError>
    self.do_stat_operation(request).await?
}
```

## Configuration

Backends typically need configuration for connection details, credentials, and behavior settings.

### Basic Configuration

```rust
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The endpoint URL for the storage service
    endpoint: String,
    
    /// Region for the storage service
    #[serde(default = "default_region")]
    region: String,
    
    /// Maximum number of concurrent connections
    #[serde(default = "default_max_connections")]
    max_connections: u32,
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_max_connections() -> u32 {
    100
}

impl Config {
    pub const CONFIG_KEY: &str = "my_backend";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}
```

### Backend Construction

Backends are typically constructed with configuration and secrets, and may need to perform initialization:

```rust
use storage_core_util::secrets::Secrets;

impl MyBackend {
    pub fn new(figment: &Figment, secrets: &Option<Secrets>) -> std::result::Result<Self, BackendInitError> {
        let config = Config::try_from(figment)?;
        
        // Extract secrets for this backend
        let secrets = match secrets {
            Some(s) => match s.get("storage-service-my-backend") {
                Some(val) => {
                    info!("My backend using secrets from config");
                    Some(val.clone())
                },
                None => {
                    info!("My backend running without secrets. Secrets exist, but no secrets for storage-service-my-backend");
                    None
                },
            },
            None => {
                info!("My backend running without secrets. No secrets provided");
                None
            },
        };
        
        // Initialize connection pool, validate credentials, etc.
        let client = Self::create_client(&config, &secrets)?;
        
        Ok(Self {
            config: Arc::new(config),
            secrets,
            client: Arc::new(client),
        })
    }

    fn create_client(config: &Config, secrets: &Option<serde_json::Value>) -> std::result::Result<StorageClient, BackendInitError> {
        // Create and configure the storage client using secrets
        match secrets {
            Some(secrets) => {
                // Extract credentials from secrets
                let access_key = secrets.get("ACCESS_KEY")
                    .and_then(|v| v.as_str())
                    .ok_or(BackendInitError::MissingCredentials("ACCESS_KEY not found in secrets"))?;
                
                let secret_key = secrets.get("SECRET_KEY")
                    .and_then(|v| v.as_str())
                    .ok_or(BackendInitError::MissingCredentials("SECRET_KEY not found in secrets"))?;
                
                // Create client with credentials
                StorageClient::new(config.endpoint.clone(), access_key, secret_key)
            },
            None => {
                // Create client without credentials (anonymous mode)
                StorageClient::new_anonymous(config.endpoint.clone())
            },
        }
    }
}
```

## Complete Example

Here's a complete example of a simple file system backend:

```rust
use std::{path::PathBuf, sync::Arc};
use storage_core_builder::{
    traits::{
        error::{Result, StorageError, StorageErrorExt},
        fileobject::v1alpha::{FileObject as FileObjectV1Alpha, StatRequest, StatResponse, ResourceInfo},
    },
};
use tonic::Request;
use thiserror::Error;
use tokio::fs;
use serde::{Deserialize, Serialize};

// Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSystemConfig {
    /// Root directory for the file system backend
    root_path: PathBuf,
    
    /// Whether to allow writes
    #[serde(default = "default_read_only")]
    read_only: bool,
}

fn default_read_only() -> bool {
    false
}

impl FileSystemConfig {
    pub const CONFIG_KEY: &str = "filesystem_backend";
}

impl TryFrom<&Figment> for FileSystemConfig {
    type Error = figment::Error;
    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

// Error handling
#[derive(Debug, Error)]
pub enum FileSystemError {
    #[error("file not found: {0}")]
    NotFound(String),
    
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum BackendInitError {
    #[error("missing credentials: {0}")]
    MissingCredentials(String),
    
    #[error("configuration error: {0}")]
    ConfigError(String),
}

impl StorageErrorExt for FileSystemError {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            FileSystemError::NotFound(msg) => tonic::Status::not_found(msg),
            FileSystemError::PermissionDenied(msg) => tonic::Status::permission_denied(msg),
            FileSystemError::IoError(e) => tonic::Status::internal(e.to_string()),
        }
    }
    
    fn http_status(&self) -> http::StatusCode {
        match self {
            FileSystemError::NotFound(_) => http::StatusCode::NOT_FOUND,
            FileSystemError::PermissionDenied(_) => http::StatusCode::FORBIDDEN,
            FileSystemError::IoError(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    
    fn get_size(&self) -> usize {
        match self {
            FileSystemError::NotFound(msg) => std::mem::size_of::<FileSystemError>() + msg.capacity(),
            FileSystemError::PermissionDenied(msg) => std::mem::size_of::<FileSystemError>() + msg.capacity(),
            FileSystemError::IoError(e) => std::mem::size_of::<FileSystemError>() + e.to_string().capacity(),
        }
    }
}

// Backend
pub struct FileSystemBackend {
    config: Arc<FileSystemConfig>,
}

impl FileSystemBackend {
    pub fn new(figment: &Figment, secrets: &Option<Secrets>) -> std::result::Result<Self, BackendInitError> {
        let config = FileSystemConfig::try_from(figment)
            .map_err(|e| BackendInitError::ConfigError(e.to_string()))?;
        
        // Ensure the root directory exists
        tokio::fs::create_dir_all(&config.root_path)
            .await
            .map_err(|e| BackendInitError::ConfigError(e.to_string()))?;
        
        Ok(Self {
            config: Arc::new(config),
        })
    }

    fn resolve_path(&self, resource_address: &str) -> PathBuf {
        self.config.root_path.join(resource_address)
    }
}

#[async_trait::async_trait]
impl FileObjectV1Alpha for FileSystemBackend {
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        None // Backends should always return None
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let resource_address = request.get_ref().resource_address.clone();
        let file_path = self.resolve_path(&resource_address);
        
        let metadata = fs::metadata(&file_path).await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => FileSystemError::NotFound(resource_address),
                std::io::ErrorKind::PermissionDenied => FileSystemError::PermissionDenied(resource_address),
                _ => FileSystemError::IoError(e),
            })?;
        
        let resource_info = ResourceInfo {
            size: metadata.len(),
            etag: None, // File systems don't have ETags
            last_modified: Some(metadata.modified()?.into()),
            content_type: None,
        };
        
        Ok(StatResponse {
            resource_info: Some(resource_info),
        })
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        // Read operations work fine in read-only mode
        // Implementation for read operation
        todo!()
    }

    async fn write(&self, request: Request<WriteParameters>, data_future: BoxFuture<Bytes>) -> Result<BoxStream<WriteResponse>> {
        if self.config.read_only {
            return Err(FileSystemError::PermissionDenied("Backend is read-only".to_string()).into());
        }
        
        // Implementation for write operation
        todo!()
    }
    
    // Other methods will automatically return UnimplementedError
}
```

## Best Practices

1. **Return `None` from `inner()`**: This is crucial for backends to allow proper error handling for unimplemented methods
2. **Implement only what you support**: Don't implement methods that your storage system doesn't support
3. **Handle external errors properly**: Convert storage system errors to appropriate storage-core errors
4. **Validate configuration**: Ensure configuration is valid during backend construction
5. **Use proper error mapping**: Map storage system errors to appropriate HTTP/gRPC status codes
6. **Implement proper resource cleanup**: Ensure connections and resources are properly cleaned up
7. **Add comprehensive logging**: Backends should log operations for debugging and monitoring
8. **Handle concurrent access**: Ensure your backend is thread-safe and handles concurrent operations

## Integration

To use your backend in a service stack:

```rust
let backend = FileSystemBackend::new(&figment, &secrets)?;
let service = backend; // Backends can be used directly as services

// Or wrap with layers
let cached_service = cache_layer.layer(backend);
```

Backends form the foundation of the storage service stack, providing the actual storage functionality that layers can then enhance with caching, logging, metrics, and other cross-cutting concerns.
