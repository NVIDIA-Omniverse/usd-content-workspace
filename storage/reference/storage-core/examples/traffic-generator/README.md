# Traffic Generator

A CLI tool that connects to a running storage-core service and systematically exercises every API function across all supported interfaces and API versions.

## API Coverage

The traffic generator calls every method in the following service traits:

- **FileObject** — `stat`, `read`, `read_from_address`, `delete`, `abort_multipart_upload`, `complete_multipart_upload`, `upload_part`, `complete_redirect_upload`, `fetch_write_type_info`, `enumerate`, `copy`, `move`
- **FileFolder** — `list`, `list_stat`, `delete_folder`, `create_folder`, `get_folder_mode`
- **Versioning** — `enumerate_versions`
- **Capabilities** — `list_top_level_addresses`, `list_routes`

Each method is tested with realistic data (file writes, folder creation, multipart uploads, redirect uploads, etc.) and cleaned up afterward.

## Prerequisites

A running storage-core service with both gRPC and REST interfaces enabled.

## Usage

```sh
cargo run -p traffic-generator -- \
  --grpc-addr http://127.0.0.1:50051 \
  --rest-port 8011 \
  --base-url https://your-storage-backend.blob.core.windows.net/container/ \
  --interface both \
  --api-version v1-alpha
```

### CLI Options

| Flag | Default | Description |
|------|---------|-------------|
| `--grpc-addr` | `http://127.0.0.1:50051` | gRPC server address |
| `--rest-port` | `8080` | REST server port |
| `--base-url` | *(required)* | Storage backend URL prefix for all file/folder operations |
| `--interface` | `both` | Interface(s) to exercise: `grpc`, `rest`, or `both` |
| `--api-version` | `v1-alpha` | API version(s) to exercise: `v1-alpha`, `v1-beta`, or `both` |
| `-n`, `--iterations` | `1` | Number of times to run the full traffic sweep |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Controls log verbosity (e.g., `debug`, `warn`, `trace`) |

## Output

The tool prints a summary table at the end showing each API call's result and latency:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  Summary                                                                     │
├──────────────────────────────────────────────────────────────────────────────┤
│  Phase                  API                         Result   Latency         │
│  grpc/v1alpha/fo-body   write                       OK       123ms           │
│  grpc/v1alpha/fo-body   stat                        OK        45ms           │
│  ...                                                                         │
├──────────────────────────────────────────────────────────────────────────────┤
│  Total: 42 calls, 42 succeeded, 0 failed                                    │
└──────────────────────────────────────────────────────────────────────────────┘
```
