# Basic Service Example

This example demonstrates a basic usage of the Storage Service Core crate to build an application.

## Building

### Build Commands

There are two feature flags for this example: `azure` and `s3`. `azure` is the default feature, and `s3` can be selected with a feature flag:

Build with Azure backend:
```bash
cargo build
```

Build with S3 backend:
```bash
cargo build --features s3 --no-default-features
```

**Note**: You must specify either the `azure` or `s3` feature - both cannot be enabled simultaneously.

## Running

### Basic Usage

Run with default settings:
```bash
# For Azure backend
cargo run

# For S3 backend  
cargo run --features s3 --no-default-features
```

### Command Line Options

- `--grpc-addr`: gRPC server address (default: 0.0.0.0:50051)
- `--http-addr`: HTTP server address (default: 0.0.0.0:8011)
- `--config`: Path to configuration file

### Example with Custom Configuration

```bash
cargo run --features azure -- --config config.toml --grpc-addr 0.0.0.0:50052 --http-addr 0.0.0.0:8012
```

## How the Remapper Works

The remapper layer intercepts storage operations and applies string replacements:

1. **Stat operations**: Remaps the resource address in the request and response
2. **List operations**: Remaps folder URIs and resource addresses in the response
3. **List stat operations**: Remaps both folder URIs and resource addresses

### Example Transformation

With the configuration `{"goodbye" = "hello"}`, a resource address like:
```
/container/goodbye/file.txt
```

Would be transformed to:
```
/container/hello/file.txt
```

### Adding New Remapping Rules

To add new remapping rules, modify the `remap_map` in your configuration:

```toml
[remapper]
remap_map = {
    "old-prefix" = "new-prefix",
    "legacy" = "modern",
    "temp" = "permanent"
}
```

## Troubleshooting

### Common Issues

1. **Feature not specified**: Ensure you specify either `--features azure` or `--features s3`
2. **Configuration errors**: Check that your `config.toml` file is valid TOML
3. **Backend connection**: Verify your Azure/S3 credentials and configuration

### Debugging

Enable debug logging:
```bash
RUST_LOG=debug cargo run --features azure
```

## License

This example is part of the NVIDIA Storage Service and is licensed under the Apache License, Version 2.0. See [LICENSE](../../LICENSE).
