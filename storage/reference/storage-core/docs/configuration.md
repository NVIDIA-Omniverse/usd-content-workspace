# Configuration

The storage-core system uses a flexible configuration system based on [Figment](https://github.com/SergioBenitez/Figment) that allows configuration through multiple sources including TOML files, JSON files, YAML files, and environment variables. Each layer and backend defines its own `Config` struct and can be constructed either manually using the `new` method with a `Config` struct, or automatically using the `from_figment` pattern.

## Configuration Architecture

### Config Structs

Every layer and backend in the storage-core system defines a `Config` struct that:

1. **Derives standard traits**: `Debug`, `Clone`, `Serialize`, `Deserialize`
2. **Implements `Default`**: Provides sensible default values for all fields
3. **Defines a `CONFIG_KEY`**: A string constant that identifies the configuration section
4. **Implements `TryFrom<&Figment>`**: Enables extraction from Figment configuration

### Construction Patterns

Each layer and backend supports two construction patterns:

#### Manual Construction
```rust
use my_layer::Config;

let config = Config {
    enabled: true,
    capacity: ByteSize::gib(2),
    // ... other fields
};

let layer = MyLayer::new(config);
```

#### From Figment Construction
```rust
use figment::Figment;
use my_layer::MyLayer;

let figment = Figment::new()
    .merge(("my_layer.enabled", true))
    .merge(("my_layer.capacity", "2GB"));

let layer = MyLayer::from_figment(&figment)?;
```

## Configuration Sources

The system supports multiple configuration sources with the following precedence (highest to lowest):

1. **Environment Variables** (prefixed with `STORAGE_SERVICE_` by default)
2. **Configuration Files** (TOML, JSON, YAML)
3. **Default Values** (defined in Config structs)

### Environment Variable Format

Environment variables use double underscores (`__`) as separators for nested configuration:

```bash
# For [s3.timeouts.connect] in TOML
STORAGE_SERVICE_S3__TIMEOUTS__CONNECT=30s

# For [stat_cache.capacity] in TOML  
STORAGE_SERVICE_STAT_CACHE__CAPACITY=2GB

# For [list_cache.capacity] in TOML  
STORAGE_SERVICE_LIST_CACHE__CAPACITY=4Gi
```

## Backend Configurations

### S3 Backend

**Config Key**: `s3`

```toml
[s3]
# Size threshold for redirecting reads to presigned URLs
read_redirect_size = "256KB"

# Size threshold for single upload vs multipart upload
write_body_size = "2MB"

# Size threshold for gRPC upload vs presigned URL upload
write_redirect_size = "64MB"

# Maximum object key size in bytes. Defaults to 1024 which matches the S3 spec.
# Override for S3-compatible backends with different limits (e.g. MinIO extended mode).
max_key_bytes = 1024

# Backend-wide default for whether object versioning is enabled on the buckets
# served by this backend; a per-endpoint VERSIONING_ENABLED entry in the secrets
# configuration overrides it per endpoint (see docs/secrets.md). Set to false to
# make EnumerateVersions fail fast with FAILED_PRECONDITION (HTTP 412) instead
# of a confusing backend error. true or unset (default) treats versioning as
# enabled; there is no runtime detection.
# versioning_enabled = false
#
# Whether FileObject-native metadata is enabled for this backend. Unset (the
# default) treats it as disabled: GetNativeMetadataSupport reports false, writes
# carrying non-empty native metadata are rejected with UNIMPLEMENTED, and native
# metadata is not surfaced on stat/read/list. When enabled, native metadata is
# surfaced on the free stat/read paths (no extra backend call) and on list. Writes
# carrying non-empty native metadata are still rejected with UNIMPLEMENTED on every
# upload path (direct-body, redirect and multipart) — write persistence is a
# follow-up — so GetNativeMetadataSupport still reports false in this pre-release
# state regardless of this flag. There is no runtime detection.
#
# Shared store: on S3 and Azure, FileObject native metadata and MetadataService
# entries live in the same underlying x-amz-meta-* / blob-metadata map. Enabling
# this therefore also exposes MetadataService-written keys through FileObject
# native_metadata, where they appear in their encoded form (e.g. nv_key_<hex>).
# native_metadata_enabled = false
#
# S3 only. Whether ListStat additionally fetches each listed object's native
# metadata via a HeadObject fan-out. S3 LIST returns no user metadata, so this
# issues one HeadObject per listed object (bounded concurrency, currently 16) and
# is expensive on large folders. Off by default and only has effect when
# native_metadata_enabled is set; Azure returns metadata inline and ignores it.
# Required when native_metadata_enabled is set and list_cache and/or
# small_object_cache are enabled: those caches are filled from ListStat, so
# without the fan-out a stat/read served from a cached listing would return an
# empty map while a direct stat/read returns the object's metadata. The backend
# refuses to start in that inconsistent configuration. Both caches ask for native
# metadata on the ListStat they fill from regardless of the caller's
# include_native_metadata - the records they cache go on to answer stat/read, so
# they have to be complete - which means the fan-out runs for those listings even
# when the caller did not set the flag. Uncached ListStat still honors it.
# native_metadata_list_fanout_enabled = false

# Declarative read-only state for this backend. IsReadOnly reports it; nothing
# enforces it -- writes are NOT rejected on the strength of this setting. The
# per-endpoint READ_ONLY secrets entry overrides it; unset means writable.
# read_only = true

[s3.timeouts]
connect = "60s"
read = "60s"
operation = "60s"
operation_attempt = "60s"
keepalive = "60s"

[s3.expiration]
presigned_expiration = "24h"
```

### Azure Backend

**Config Key**: `azure`

```toml
[azure]
# Connect to Azurite emulator
emulator = false

# Backend-wide default for whether object versioning is enabled on the buckets
# served by this backend; a per-endpoint VERSIONING_ENABLED entry in the secrets
# configuration overrides it per endpoint (see docs/secrets.md). Set to false to
# make EnumerateVersions fail fast with FAILED_PRECONDITION (HTTP 412) instead
# of a confusing backend error. true or unset (default) treats versioning as
# enabled; there is no runtime detection.
# versioning_enabled = false
#
# Whether FileObject-native metadata is enabled for this backend. Unset (the
# default) treats it as disabled: GetNativeMetadataSupport reports false, writes
# carrying non-empty native metadata are rejected with UNIMPLEMENTED, and native
# metadata is not surfaced on stat/read/list. When enabled, native metadata is
# surfaced on the free stat/read paths (no extra backend call) and on list (Azure
# returns it inline on list, so no fan-out toggle is needed). Writes carrying
# non-empty native metadata are still rejected with UNIMPLEMENTED on every upload
# path (direct-body, redirect and multipart) — write persistence is a follow-up —
# so GetNativeMetadataSupport still reports false in this pre-release state
# regardless of this flag. There is no runtime detection.
#
# Shared store: on S3 and Azure, FileObject native metadata and MetadataService
# entries live in the same underlying x-amz-meta-* / blob-metadata map. Enabling
# this therefore also exposes MetadataService-written keys through FileObject
# native_metadata, where they appear in their encoded form (e.g. nv_key_<hex>).
# native_metadata_enabled = false

# Declarative read-only state for this backend. IsReadOnly reports it; nothing
# enforces it -- writes are NOT rejected on the strength of this setting. The
# per-endpoint READ_ONLY secrets entry overrides it; unset means writable.
# read_only = true

# Size threshold for redirecting reads to presigned URLs
read_redirect_size = "256KB"

# Size threshold for single upload vs multipart upload
write_body_max_size = "2MB"

# Size threshold for gRPC upload vs presigned URL upload
write_redirect_max_size = "64MB"

[azure.timeouts]
connect = "60s"
read = "60s"
operation = "60s"
operation_attempt = "60s"
keepalive = "60s"

[azure.expiration]
presigned_expiration = "24h"
```

## Layer Configurations

### Stat Cache Layer

**Config Key**: `stat_cache`

```toml
[stat_cache]
enabled = true
l2_enabled = false  # Requires [redis] plus this flag to use Redis L2
capacity = "1GB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false
capacity_warn_pct = 95  # Log WARN when cache utilization exceeds this % (default: 95, 0 to disable)
```

### List Cache Layer

**Config Key**: `list_cache`

```toml
[list_cache]
enabled = true
l2_enabled = false  # Requires [redis] plus this flag to use Redis L2
capacity = "1GiB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false
page_size_limit = "3MiB"
capacity_warn_pct = 95  # Log WARN when cache utilization exceeds this % (default: 95, 0 to disable)
```

### Small Object Cache Layer

**Config Key**: `small_object_cache`

```toml
[small_object_cache]
enabled = true
l2_enabled = false  # Requires [redis] plus this flag to use Redis L2
capacity = "1GB"
time_to_live = "15m"
small_object_size = "256KB"
capacity_warn_pct = 95  # Log WARN when cache utilization exceeds this % (default: 95, 0 to disable)
```

Body entries and metadata-only hints use separate key prefixes. A
`b:{resource_identity}` entry contains complete metadata and body bytes, while
an `m:{resource_identity}` entry contains metadata from `stat`, `list_stat`, or
an uncacheable read. Metadata can therefore hint a backend read without
satisfying a body request. The two entries expire independently. In particular,
`stat` and `list_stat` only write the `m:` key and never refresh the `b:` key, so
`time_to_live` remains a hard lifetime for a body entry even when metadata is
queried frequently.

Concurrent non-redirect misses for one body key are coalesced within each
service process by Moka's same-key initialization. With L2 enabled, the L2
lookup, best-effort Redis lease, source fill, and fail-open path are all inside
that same process-local single-flight boundary. Callers in one process therefore
continue to coalesce if Redis is disconnected or a Redis operation fails. The
Redis lease arbitrates which process may write a filled value back to L2; a
process that does not acquire the lease still performs its backend fill.
Coalescing is therefore not a fleet-wide guarantee.

The cache drains and validates a candidate small response before returning its
stream, so a cacheable miss has whole-body stream-open latency. Every coalesced
caller then receives the same complete cached entry. An error encountered while
draining a candidate body is likewise returned by the read/open operation
rather than later through the stream.

A response is not stored as a body entry when any of these conditions applies:

- the request explicitly asks for a redirect, or the response contains a
  redirect;
- the response stream is empty or its first part is not `ResourceInfo`;
- `ResourceInfo` omits the encoded identity, declared body size, or
  last-modified timestamp;
- the backend-reported identity differs from the identity used for the request;
- the declared size exceeds `small_object_size`;
- a part after the initial `ResourceInfo` is anything other than a body chunk;
- the number of body bytes received is greater or less than the declared size;
  or
- the backend stream returns an error.

The caller whose cache initializer ran passes through the response it already
fetched; every coalesced caller performs its own backend read. Explicit redirect
requests bypass coalescing altogether. Redirects are therefore isolated so a
presigned URL is never shared between callers. Large, unknown-size, and other
uncacheable bodies are fetched once per caller, and responses identified as
uncacheable before EOF continue streaming after the read opens. When the
backend-reported identity differs from the request identity, complete metadata
may still be stored under `m:{backend_identity}`, but no body alias is created
under the request identity.

The small-object cache never retains backend or stream errors. Callers already
waiting on the same in-progress fill may receive that fill's error, but the next
request contacts the backend again. The legacy `errors` configuration field is
still accepted for compatibility but has no effect for this layer; omit it from
new configurations.

This rewrite changes the interpretation of cache metrics. Coalesced callers
produce one fill miss in `storage.cache.operations`, rather than one miss per
waiting request, while the separate `m:` metadata-hint lookup can add a cache
operation during a body miss. Use request metrics, not cache-operation totals,
when a per-request count is required.

The Redis key format also changes from an unprefixed resource identity to the
`b:` and `m:` keyspaces. Existing L2 entries are safe to leave in place but are
cold to the new implementation. During a rolling deployment, old and new
processes can fill both formats, causing cold misses and temporarily increasing
Redis usage toward the combined footprint until the old entries expire. No data
migration is required for correctness; allow capacity headroom for the rollout
and warm-up period.

#### Migration to 0.9

The replay-based in-flight API has been removed as an intentional breaking
change:

- `InFlightCache` no longer exists;
- `SmallObjectCacheLayer::in_flight_reads` and
  `SmallObjectCache::in_flight_reads` have been removed;
- `small_object_cache.in_flight_read_channel_capacity` and the corresponding
  public `Config` field have been removed; Moka coalescing has no replay-channel
  capacity to tune; and
- `SmallObjectCache::new` now takes four arguments: `inner`, `cache`, `config`,
  and `l2_write_semaphore`. Remove the former `in_flight_reads` third argument.

### Manifest Layer

**Config Key**: `manifest`

```toml
[manifest]
enabled = true
l2_enabled = false  # Requires [redis] plus this flag to use Redis L2
error_on_non_manifest_objects = false
capacity = "1GB"
time_to_live = "15m"
page_size_limit = "3MiB"
capacity_warn_pct = 95  # Log WARN when cache utilization exceeds this % (default: 95, 0 to disable)
```

### Egress Throttle Layer

**Config Key**: `egress_throttle`

```toml
[egress_throttle]
enabled = true
max_concurrency = 256
warn_utilization_pct = 80   # Log WARN when this % of permits are in use (default: 80, 0 to disable)
error_utilization_pct = 95  # Log ERROR when this % of permits are in use (default: 95, 0 to disable)
```

### Additional Headers Layer

**Config Key**: `additional_headers`

```toml
[additional_headers]
enabled = true
cache_control = "no-store"
cache_control_api = "no-cache, no-store"
tracing_header_names = ["x-request-id"]
env_header_names = ["MY_CUSTOM_HEADER", "DEPLOYMENT_ID"]
```

## Observability Configuration

### Error Severity

**Config Key**: `error_severity`

Maps gRPC status codes to log severity levels. Any codes not listed use the SRD-recommended defaults shown below. Set via the `[error_severity]` config section or by calling `StorageService::with_error_severity_from_figment(&figment)`.

Code names and level names are both case-insensitive. `AccessDenied` is accepted as an alias for `PermissionDenied`.

```toml
[error_severity]
# Override specific codes — unspecified codes use defaults below.
# NotFound = "DEBUG"       # Downgrade from default INFO
# Unavailable = "ERROR"    # Upgrade from default WARN
```

**Default severity mapping (per Omniverse Logging SRD 3.2):**

| Level | gRPC Codes |
|-------|------------|
| ERROR | `Ok`, `Unknown`, `Aborted`, `Unimplemented`, `Internal`, `DataLoss` |
| WARN  | `PermissionDenied`, `Unauthenticated`, `DeadlineExceeded`, `Unavailable`, `ResourceExhausted` |
| INFO  | `InvalidArgument`, `NotFound`, `AlreadyExists`, `OutOfRange`, `FailedPrecondition` |
| DEBUG | `Cancelled` |

Each gRPC status code has an independent rate limiter, so a flood of `NotFound` errors will not suppress `Internal` errors.

### Logging Environment Variables

These environment variables control the observability system. They are validated at startup — the process terminates if any are invalid.

**OTEL Log Export:**

| Variable | Description | Default |
|----------|-------------|---------|
| `OTEL_LOGS_EXPORTER` | Enable/disable OTEL log export (`otlp` or `none`) | From OTEL SDK |
| `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` | OTLP collector endpoint | From OTEL SDK |
| `OTEL_EXPORTER_OTLP_LOGS_PROTOCOL` | Transport protocol (`grpc` or `http/protobuf`). Falls back to `OTEL_EXPORTER_OTLP_PROTOCOL`, then infers from endpoint port, then defaults to `grpc` | `grpc` |
| `LOGS_LEVEL` | Global severity filter for OTLP export. Accepts level names (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`) or OTEL severity numbers (1-24) | `ERROR` |
| `LOGS_MODULES` | Per-module OTLP severity overrides. Comma-separated `module=LEVEL` pairs (e.g. `storage_core=DEBUG,hyper=ERROR`) | None |

**Rate Limiting:**

| Variable | Description | Default |
|----------|-------------|---------|
| `LOGS_RATE_LIMIT_COUNT` | Messages allowed per window before suppression | `10` |
| `LOGS_RATE_LIMIT_INTERVAL` | Window duration (humantime, e.g. `1s`, `500ms`) | `1s` |
| `LOGS_RATE_LIMIT_SILENCE` | Suppression duration after limit is hit | `60s` |
| `LOGS_RATE_LIMIT_BYPASS_LEVEL` | Severity that bypasses suppression (`ERROR`, `WARN`, etc., or `NONE` to disable) | `ERROR` |

**Slow Operation Detection:**

| Variable | Description | Default |
|----------|-------------|---------|
| `LOGS_SLOW_THRESHOLD` | Duration threshold for slow operation logging (humantime) | `10s` |

**Config Re-logging:**

| Variable | Description | Default |
|----------|-------------|---------|
| `LOGS_CONFIG_RELOG_INTERVAL` | Interval for periodically re-logging the effective config (humantime, `0s` to disable) | `60m` |

### Metadata Cache Layer

**Config Key**: `metadata_cache`

```toml
[metadata_cache]
enabled = true
l2_enabled = false  # Requires [redis] plus this flag to use Redis L2
capacity = "1GB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false
```

## Utility Configurations

### Redis L2 Cache

**Config Key**: `redis`

The `[redis]` section defines the shared Redis/Valkey client. A cache uses Redis L2 only when this section is present and that cache also sets `l2_enabled = true`.

```toml
[redis]
address = "redis://127.0.0.1:6379"
namespace = "my-service"
command_timeout_ms = 250    # caps individual Redis commands
connect_timeout_ms = 10000
pool_size = 8               # multiplexed connections; clamped to >= 1
broadcast_channel_capacity = 1024
reconnect_max_attempts = 0    # 0 = retry forever
reconnect_min_delay_ms = 100
reconnect_max_delay_ms = 30000 # capped at 600000 (10 min)
reconnect_base = 2
reconnect_jitter_ms = 50
```

### Metrics Server

**Config Key**: `metrics`

```toml
[metrics]
enabled = true
addr = "0.0.0.0"
port = 8002
tokio_poll_interval_ms = 1000
```

### Secrets Management

**Config Key**: `secrets`

```toml
[secrets]
secrets_file = "/var/secrets/secrets.json"
# OR provide secrets directly as JSON string
# secrets = '{"key": "value"}'
```

## Complete Sample Configuration

Here's a comprehensive TOML configuration file with all available options:

```toml
# S3 Backend Configuration
[s3]
read_redirect_size = "256KB"
write_body_size = "2MB"
write_redirect_size = "64MB"
max_key_bytes = 1024
# versioning_enabled = false  # backend-wide default; per-endpoint VERSIONING_ENABLED in secrets overrides. false => FAILED_PRECONDITION on EnumerateVersions
# native_metadata_enabled = false  # backend-wide default. false => GetNativeMetadataSupport reports false, native-metadata writes => UNIMPLEMENTED. true => free stat/read/list surfacing; native-metadata writes remain UNIMPLEMENTED.
# read_only = true  # backend-wide default; per-endpoint READ_ONLY in secrets overrides. Declarative only -- IsReadOnly reports it, writes are not rejected

[s3.timeouts]
connect = "60s"
read = "60s"
operation = "60s"
operation_attempt = "60s"
keepalive = "60s"

[s3.expiration]
presigned_expiration = "24h"

# Azure Backend Configuration
[azure]
emulator = false
read_redirect_size = "256KB"
write_body_max_size = "2MB"
write_redirect_max_size = "64MB"
# versioning_enabled = false  # backend-wide default; per-endpoint VERSIONING_ENABLED in secrets overrides. false => FAILED_PRECONDITION on EnumerateVersions
# native_metadata_enabled = false  # backend-wide default. false => GetNativeMetadataSupport reports false, native-metadata writes => UNIMPLEMENTED. true => free stat/read/list surfacing; native-metadata writes remain UNIMPLEMENTED.
# read_only = true  # backend-wide default; per-endpoint READ_ONLY in secrets overrides. Declarative only -- IsReadOnly reports it, writes are not rejected

[azure.timeouts]
connect = "60s"
read = "60s"
operation = "60s"
operation_attempt = "60s"
keepalive = "60s"

[azure.expiration]
presigned_expiration = "24h"

# Stat Cache Layer
[stat_cache]
enabled = false
l2_enabled = false
capacity = "1GB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false
capacity_warn_pct = 95

# List Cache Layer (^^ Stat Cache is disabled above ^^)
[list_cache]
enabled = true
l2_enabled = false
capacity = "1GiB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false
page_size_limit = "3MiB"
capacity_warn_pct = 95

# Small Object Cache Layer
[small_object_cache]
enabled = true
l2_enabled = false
capacity = "1GB"
time_to_live = "15m"
small_object_size = "256KB"
capacity_warn_pct = 95

# Manifest Layer
[manifest]
enabled = true
l2_enabled = false
error_on_non_manifest_objects = false
capacity = "1GB"
time_to_live = "15m"
page_size_limit = "3MiB"
capacity_warn_pct = 95

# Egress Throttle Layer
[egress_throttle]
enabled = true
max_concurrency = 256
warn_utilization_pct = 80
error_utilization_pct = 95

# Error Severity Overrides (optional — defaults per SRD 3.2)
# [error_severity]
# NotFound = "DEBUG"
# Unavailable = "ERROR"

# Additional Headers Layer
[additional_headers]
enabled = true
cache_control = "no-store"
cache_control_api = "no-cache, no-store"
tracing_header_names = ["x-request-id"]
env_header_names = ["MY_CUSTOM_HEADER", "DEPLOYMENT_ID"]

# Metadata Cache Layer
[metadata_cache]
enabled = true
l2_enabled = false
capacity = "1GB"
time_to_live = "15m"
errors = ["AccessDenied", "NotFound"]
invalidate_on_update = false

# Optional Redis L2 Cache
# [redis]
# address = "redis://127.0.0.1:6379"
# namespace = "my-service"
# command_timeout_ms = 250
# connect_timeout_ms = 10000
# pool_size = 8
# broadcast_channel_capacity = 1024
# reconnect_max_attempts = 0    # 0 = retry forever
# reconnect_min_delay_ms = 100
# reconnect_max_delay_ms = 30000 # capped at 600000 (10 min)
# reconnect_base = 2
# reconnect_jitter_ms = 50

# Metrics Server
[metrics]
enabled = true
addr = "0.0.0.0"
port = 8002
tokio_poll_interval_ms = 1000

# Secrets Management
[secrets]
secrets_file = "/var/secrets/secrets.json"
```

## Configuration Loading

The system uses the `storage-core-util::config` module to load configuration:

```rust
use storage_core_util::config::{ConfigArgs, generate_figment};

let args = ConfigArgs {
    configuration_file: Some("config.toml".into()),
    env_prefix: "STORAGE_SERVICE_".to_string(),
};

let figment = generate_figment(&args)?;
```

This will:
1. Load the specified configuration file (TOML, JSON, or YAML)
2. Merge environment variables with the `STORAGE_SERVICE_` prefix
3. Return a `Figment` instance that can be used to construct layers and backends

## Error Handling

Configuration errors are handled through the `figment::Error` type, which provides detailed information about what went wrong during configuration parsing or extraction.
