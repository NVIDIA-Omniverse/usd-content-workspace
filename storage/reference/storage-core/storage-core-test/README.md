# Storage Core Test

A comprehensive testing utility crate for NVIDIA Omniverse Storage Core that provides unified client interfaces for testing both gRPC and REST APIs with the same test code.

## Overview

The `storage-core-test` crate is designed to simplify testing of the NVIDIA Omniverse Storage Core service by providing a unified interface that abstracts away the differences between gRPC and REST implementations. This allows developers to write tests once and run them against both protocols, ensuring consistency and reducing code duplication.

## Key Features

- **Unified Client Interface**: Test both gRPC and REST APIs with identical test code
- **Service Management**: Launch and manage a storage service in process or as a separate process
- **Multiple Protocol Support**: Seamless switching between gRPC and REST interfaces
- **Header Configuration**: Flexible request/response header management and validation
- **Metrics Integration**: Built-in support for Prometheus metrics collection
- **Temporary Directory Management**: Utilities for creating and cleaning up test data
- **Process Lifecycle Management**: Automatic cleanup with death signal handling

## Client Structure

The crate is organized around trait-based abstractions that define common interfaces for different storage operations:

### Core Traits

- **`Capabilities`**: Service discovery and routing information
- **`FileObject`**: File operations (read, write, delete, stat, enumerate)
- **`FileFolder`**: Directory operations (list, list_stat, delete_folder)
- **`Versioning`**: Version management operations
- **`Write`**: Copy operations

### Implementation Structure

```
clients/
├── traits/           # Trait definitions for unified interfaces
│   ├── capabilities.rs
│   ├── fileobject.rs
│   ├── filefolder.rs
│   ├── versioning.rs
│   └── write.rs
├── grpc/            # gRPC implementations
│   ├── capabilities.rs
│   ├── fileobject.rs
│   ├── filefolder.rs
│   ├── versioning.rs
│   └── write.rs
└── rest/            # REST implementations
    ├── capabilities.rs
    ├── fileobject.rs
    ├── filefolder.rs
    ├── versioning.rs
    └── write.rs
```

## Testing Both Interfaces

The crate enables testing both gRPC and REST interfaces with the same test code through the `Interface` enum and `StorageService`. Here's how it works in practice:

```rust
use storage_core_test::{StorageService, Interface, HeadersConfig};
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core::traits::fileobject::v1alpha::*;

// Create a storage service instance
let service = StorageService::new(
    "path/to/storage-service",
    "path/to/config.json",
    logs_path,
    HeadersConfig::default(),
);

// Test function that works with any FileObject implementation
async fn test_file_operations(client: &mut Box<dyn FileObject>) -> Result<(), Box<dyn std::error::Error>> {
    // Write a file
    let data = b"Hello, World!";
    let write_params = WriteParameters {
        destination_resource_address: "s3://bucket/test-file".to_string(),
        data_object_size: data.len() as u64,
        upload_preference: Some(UploadPreference::Body.into()),
        previous_version: None,
    };
    let write_response = client.write(write_params, Some(data.into())).await?;
    
    // Read the file back
    let read_request = ReadFromAddressRequest {
        resource_address: "s3://bucket/test-file".to_string(),
        download_preference: Some(DownloadPreference::Body.into()),
    };
    let mut stream = client.read_from_address(read_request).await?;
    // Process stream...
    
    Ok(())
}

// Test with gRPC
let mut grpc_client = service.file_object_client(Interface::Grpc).await;
test_file_operations(&mut grpc_client).await?;

// Test with REST using the same code
let mut rest_client = service.file_object_client(Interface::Rest).await;
test_file_operations(&mut rest_client).await?;
```

### Benefits of This Approach

1. **Code Reuse**: Write tests once, run against both protocols
2. **Consistency Validation**: Ensure both interfaces behave identically
3. **Protocol Coverage**: Comprehensive testing of all communication paths
4. **Maintenance**: Single test suite to maintain for both protocols

## Usage Examples

### Basic Service Setup with Headers

```rust
use storage_core_test::{StorageService, HeadersConfig, Interface};
use std::collections::HashMap;

// Configure headers for authentication/authorization
let mut headers = HashMap::new();
headers.insert("x-request-id".to_string(), "test-request-id".to_string());
headers.insert("authorization".to_string(), "Bearer test-token".to_string());

let mut validation_headers = HashMap::new();
validation_headers.insert("cache_control".to_string(), "no-cache".to_string());

let headers_config = HeadersConfig::new(headers, validation_headers);

// Launch storage service
let service = StorageService::new(
    "./target/debug/storage-service",
    "./config.json",
    std::env::temp_dir(),
    headers_config,
);
```

### Comprehensive File Operations Test

```rust
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core::traits::fileobject::v1alpha::*;
use futures::stream::StreamExt;

async fn test_file_lifecycle(client: &mut Box<dyn FileObject>) -> Result<(), Box<dyn std::error::Error>> {
    let file_url = "s3://test-bucket/test-file.txt";
    let data = b"Hello, Storage Core!";
    
    // 1. Write a file using body upload
    let write_params = WriteParameters {
        destination_resource_address: file_url.to_string(),
        data_object_size: data.len() as u64,
        upload_preference: Some(UploadPreference::Body.into()),
        previous_version: None,
    };
    let write_response = client.write(write_params, Some(data.into())).await?;
    let resource_identity = write_response.resource_info.unwrap().resource_identity.unwrap();
    
    // 2. Stat the file
    let stat_response = client.stat(StatRequest {
        resource_address: file_url.to_string(),
    }).await?;
    let metadata = stat_response.resource_info.unwrap().metadata.unwrap();
    assert_eq!(metadata.data_object_size.unwrap(), data.len() as u64);
    
    // 3. Read the file back
    let read_request = ReadFromAddressRequest {
        resource_address: file_url.to_string(),
        download_preference: Some(DownloadPreference::Body.into()),
    };
    let mut stream = client.read_from_address(read_request).await?;
    
    // First message should be metadata
    let first_response = stream.next().await.unwrap()?;
    match first_response.reply_type.unwrap() {
        read_from_address_response::ReplyType::ResourceInfo(info) => {
            assert_eq!(info.metadata.unwrap().data_object_size.unwrap(), data.len() as u64);
        }
        _ => panic!("Expected ResourceInfo"),
    }
    
    // Collect all data chunks
    let mut read_data = Vec::new();
    while let Some(response) = stream.next().await {
        match response?.reply_type.unwrap() {
            read_from_address_response::ReplyType::Chunk(chunk) => {
                read_data.extend_from_slice(&chunk.chunk);
            }
            _ => break,
        }
    }
    assert_eq!(read_data, data);
    
    // 4. Delete the file
    client.delete(DeleteRequest {
        resource_address: file_url.to_string(),
        previous_version: None,
    }).await?;
    
    // 5. Verify file is gone
    let stat_result = client.stat(StatRequest {
        resource_address: file_url.to_string(),
    }).await;
    assert!(stat_result.is_err());
    
    Ok(())
}

// Test with both interfaces
let mut grpc_client = service.file_object_client(Interface::Grpc).await;
test_file_lifecycle(&mut grpc_client).await?;

let mut rest_client = service.file_object_client(Interface::Rest).await;
test_file_lifecycle(&mut rest_client).await?;
```

### Temporary Directory Management

```rust
use storage_core_test::TempDirectory;

// Create a temporary directory for test data
let mut temp_dir = TempDirectory::new(
    service.file_object_client(Interface::Grpc).await,
    "s3://test-bucket",
    "unittests/writetests"
);

// Create test files with random names
let file_url = temp_dir.new_file();
let (file_url, relative_path) = temp_dir.new_file_with_rel_path();

// Write test data
let data = b"Test content";
let write_params = WriteParameters {
    destination_resource_address: file_url.clone(),
    data_object_size: data.len() as u64,
    upload_preference: Some(UploadPreference::Body.into()),
    previous_version: None,
};
client.write(write_params, Some(data.into())).await?;

// Clean up automatically - removes all files in the directory
temp_dir.cleanup().await;
```

### Multipart Upload Example

```rust
async fn test_multipart_upload(client: &mut Box<dyn FileObject>) -> Result<(), Box<dyn std::error::Error>> {
    let file_url = "s3://bucket/large-file.bin";
    let data = vec![0u8; 10 * 1024 * 1024]; // 10MB file
    let part_count = 4;
    
    // 1. Initiate multipart upload
    let write_params = WriteParameters {
        destination_resource_address: file_url.to_string(),
        data_object_size: data.len() as u64,
        upload_preference: Some(UploadPreference::Multipart.into()),
        previous_version: None,
    };
    let response = client.write(write_params, None).await?;
    
    let multipart_response = match response.write_response_type.unwrap() {
        WriteResponseType::MultipartUpload(mp) => mp,
        _ => panic!("Expected MultipartUpload response"),
    };
    
    // 2. Upload parts
    let chunk_size = data.len() / part_count;
    let mut completed_parts = Vec::new();
    
    for (i, chunk) in data.chunks(chunk_size).enumerate() {
        let upload_response = client.upload_part(UploadPartRequest {
            upload_id: multipart_response.upload_id.clone(),
            destination_resource_address: file_url.to_string(),
            part_number: i as u32,
            part_count: Some((part_count - 1) as u32),
        }).await?;
        
        // Perform the actual upload to the redirect URL
        let redirect = &upload_response.part_write_redirects[0];
        let http_response = reqwest::Client::new()
            .put(&redirect.redirect_target_url)
            .body(chunk.to_vec())
            .send()
            .await?;
        
        // Collect completion headers
        let mut headers = Vec::new();
        for header_name in &redirect.completion_header_names {
            if let Some(header_value) = http_response.headers().get(header_name) {
                headers.push(Header {
                    name: header_name.clone(),
                    value: header_value.to_str()?.to_string(),
                });
            }
        }
        
        completed_parts.push(CompletedUploadPart {
            part_number: i as u32,
            headers,
        });
    }
    
    // 3. Complete multipart upload
    let complete_response = client.complete_multipart_upload(CompleteMultipartUploadRequest {
        upload_id: multipart_response.upload_id,
        destination_resource_address: file_url.to_string(),
        parts: completed_parts,
    }).await?;
    
    // Verify the file exists
    let stat_response = client.stat(StatRequest {
        resource_address: file_url.to_string(),
    }).await?;
    assert_eq!(stat_response.resource_info.unwrap().metadata.unwrap().data_object_size.unwrap(), data.len() as u64);
    
    Ok(())
}
```

### Metrics Collection and Validation

```rust
use storage_core_test::metrics;

async fn validate_metrics(service: &StorageService, backend: Backend, cache: Cache) {
    // Collect Prometheus metrics
    let metrics = metrics::get_metrics(service.metrics_port).await;
    
    // Find specific metric values
    let request_count = metrics::get_metric_value(
        service.metrics_port,
        "storage_stat_requests_counter",
        &[("result", "Ok"), ("storage_backend", "azure")]
    ).await;
    
    assert!(request_count.is_some());
    
    // Get all metrics with a specific name
    let stat_metrics = metrics::get_metrics_with_name(metrics, "storage_stat_requests_counter");
    assert!(!stat_metrics.is_empty());
    
    // Find metrics by labels
    let ok_requests = metrics::find_value(&stat_metrics, &[("result", "Ok")]);
    assert!(ok_requests.is_some());
}
```

### Testing Different Backends and Access Patterns

```rust
use common::launcher::*; // From the actual test framework

#[rstest]
#[test_log::test(tokio::test)]
async fn test_across_backends(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    // Get appropriate client for the backend/interface combination
    let mut client = storage_fixture.file_object_client(backend, cache, interface).await;
    
    // Create a temporary directory for this test
    let Some(mut temp_dir) = storage_fixture.temp_directory(backend, access, cache).await else {
        // Skip test if this backend/access combination isn't supported
        return;
    };
    
    // Perform test operations
    let file_url = temp_dir.new_file();
    let data = b"Test data";
    
    // Write file
    let write_params = WriteParameters {
        destination_resource_address: file_url.clone(),
        data_object_size: data.len() as u64,
        upload_preference: Some(UploadPreference::Body.into()),
        previous_version: None,
    };
    let response = client.write(write_params, Some(data.into())).await.unwrap();
    
    assert_identity_has_object_identifier_parameters(
        response.resource_info.unwrap().resource_identity.unwrap().encoded_identity.as_ref(),
        false,
    );
    
    // Clean up
    temp_dir.cleanup().await;
    
    // Validate metrics were recorded
    assert_gt_zero!(storage_fixture.counter_value(
        backend,
        cache,
        "storage_write_requests_counter",
        &[("result", "Ok")]
    ));
}
```

## Configuration

### HeadersConfig

The `HeadersConfig` struct allows you to:

- **Add Headers**: Automatically include headers in all requests
- **Validate Headers**: Ensure expected headers are present in responses
- **Authentication**: Handle authentication tokens and credentials

```rust
use std::collections::HashMap;
use storage_core_test::HeadersConfig;

let mut headers = HashMap::new();
headers.insert("x-request-id".to_string(), "test-request-id".to_string());
headers.insert("x-omniverse-request-id".to_string(), "zomgwtfbbq".to_string());
headers.insert("authorization".to_string(), "Bearer test-token".to_string());

let mut validation_headers = HashMap::new();
validation_headers.insert("cache_control".to_string(), "no-cache".to_string());
validation_headers.insert("x-request-id".to_string(), "test-request-id".to_string());

let config = HeadersConfig::new(headers, validation_headers);
```

### Service Configuration

The `StorageService` can be configured with different backends and cache settings:

```rust
// Example configuration for different backends
let service = StorageService::new(
    "./target/debug/storage-service-azure", // Azure backend binary
    "./azure_config.toml",                  // Azure-specific config
    logs_path,
    headers_config,
);

// Or for S3 backend
let service = StorageService::new(
    "./target/debug/storage-service-s3",    // S3 backend binary
    "./s3_config.toml",                     // S3-specific config
    logs_path,
    headers_config,
);
```

## Integration with Test Frameworks

The crate is designed to work seamlessly with popular Rust testing frameworks:

### With rstest

```rust
use rstest::*;
use storage_core_test::service::Interface;

#[rstest]
#[test_log::test(tokio::test)]
async fn test_file_operations(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let mut client = storage_fixture.file_object_client(backend, Cache::Disabled, interface).await;
    // Test implementation...
}
```

### With test_log

The crate integrates with `test_log` for proper logging during tests:

```rust
#[test_log::test(tokio::test)]
async fn my_test() {
    // Logs will be captured and displayed properly
    tracing::info!("Starting test");
    // Test implementation...
}
```

## Using StorageServiceRunner for In-Process Testing

The `StorageServiceRunner` provides a way to run a storage service in-process for testing, which is particularly useful for integration tests and unit tests that need to test the full service stack without external dependencies.

### Key Benefits

- **In-Process Execution**: Run the storage service in the same process as your tests
- **Automatic Port Management**: Automatically finds available ports for gRPC, REST, and metrics servers
- **Signal Handling**: Proper cleanup with signal handling for graceful shutdown
- **Multiple Backend Support**: Works with different storage backends (S3, Azure, etc.)
- **Metrics Integration**: Built-in Prometheus metrics server for monitoring

### Basic Usage

```rust
use storage_core_test::storage_service_runner::StorageServiceRunner;
use storage_core_test::headers::HeadersConfig;
use storage_core::storage::StorageServiceServer;
use std::collections::HashMap;

// Create headers configuration
let mut headers = HashMap::new();
headers.insert("x-request-id".to_string(), "test-request-id".to_string());
headers.insert("authorization".to_string(), "Bearer test-token".to_string());

let headers_config = HeadersConfig::new(headers, HashMap::new());

// Create a storage service server (this would be your actual service)
let storage_service = create_storage_service(
    "config.toml".to_string(),
    |figment, secrets| {
        Ok(std::sync::Arc::new(storage_core::s3::Storage::from_figment(
            figment, secrets,
        )?))
    },
);

// Launch the service in a new thread
let runner = StorageServiceRunner::new::<tower::layer::util::Identity, tonic::body::Body>(
    storage_service,
    "s3".to_string(), // metrics backend label
    headers_config,
);

// Get clients for testing
let mut file_object_client = runner.file_object_client(Interface::Grpc).await;
let mut capabilities_client = runner.capabilities_client(Interface::Rest).await;
let mut write_client = runner.write_client(Interface::Grpc).await;

// Use the clients for testing...
```

### Testing with Multiple Configurations

The `StorageServiceRunner` is particularly useful for testing different service configurations:

```rust
#[rstest]
#[test_log::test(tokio::test)]
async fn test_with_different_configs(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    // Get the appropriate client based on configuration
    let mut client = storage_fixture.file_object_client(backend, cache, interface).await;
    
    // Test your functionality
    let file_url = "s3://test-bucket/test-file.txt";
    let data = b"Test data";
    
    // Write file
    let write_params = WriteParameters {
        destination_resource_address: file_url.to_string(),
        data_object_size: data.len() as u64,
        upload_preference: Some(UploadPreference::Body.into()),
        previous_version: None,
    };
    let response = client.write(write_params, Some(data.into())).await.unwrap();
    
    // Verify metrics are being collected
    let metrics_port = storage_fixture.metrics_port(backend, cache);
    let request_count = storage_fixture.counter_value(
        backend,
        cache,
        "storage_write_requests_counter",
        &[("result", "Ok")]
    ).await;
    
    assert!(request_count > 0.0);
}
```

### Metrics and Monitoring

The `StorageServiceRunner` automatically sets up a metrics server:

```rust
// Access metrics port
let metrics_port = runner.metrics_port;

// Collect metrics
let metrics = storage_core_test::metrics::get_metrics(metrics_port).await;

// Get specific metric values
let request_count = storage_core_test::metrics::get_metric_value(
    metrics_port,
    "storage_write_requests_counter",
    &[("result", "Ok"), ("storage_backend", "s3")]
).await;


### When to Use StorageServiceRunner vs StorageService

- **Use `StorageServiceRunner`** when:
  - You want to test the full service stack in-process

- **Use `StorageService`** when:
  - You want to test against an external binary

This crate provides a robust foundation for testing storage core services while maintaining the flexibility to test both communication protocols with minimal code duplication.
