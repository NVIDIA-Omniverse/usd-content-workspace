# Storage Core Metrics

This document describes the metrics system used in the Storage Core project, including how metrics are defined, exported, and configured.

## Overview

The Storage Core metrics system provides comprehensive observability into the performance and behavior of storage operations. It uses OpenTelemetry (OTel) for metrics collection and export, supporting OTLP (OpenTelemetry Protocol) for pushing metrics to collectors.

## 1. Initializing Observability

Observability (including metrics, tracing, and logging) is initialized using the `init_observability!` macro from `storage_core_util`:

```rust
use storage_core_util::init_observability;

fn main() {
    // Simplest form: uses CARGO_BIN_NAME as service name
    let _guard = init_observability!();
    
    // Or specify a custom service name
    let _guard = init_observability!("my-storage-service");
    
    // Your application code...
}
```

The macro automatically captures:
- `CARGO_BIN_NAME` as the service name (when called with no arguments)
- `CARGO_PKG_VERSION` as the service version

The returned guard should be kept alive for the duration of your application to ensure proper cleanup of OpenTelemetry resources.

### Resource Attributes

The `init_observability!` macro automatically configures the following [OpenTelemetry Service resource attributes](https://opentelemetry.io/docs/specs/semconv/resource/#service):

| Attribute | Description |
|-----------|-------------|
| `service.name` | The name of the service (from `CARGO_BIN_NAME` or explicit parameter) |
| `service.version` | The semantic version of the service (from `CARGO_PKG_VERSION`) |
| `service.namespace` | Set to `"omniverse"` for all Omniverse services |

### Configuration via Environment Variables

The observability system is configured via standard OpenTelemetry environment variables:

| Variable | Description |
|----------|-------------|
| `OTEL_SDK_DISABLED` | Set to `"true"` to disable OpenTelemetry SDK entirely |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | The endpoint for OTLP export (e.g., `http://localhost:4317`) |
| `OTEL_METRICS_EXPORTER` | Set which metrics exporter to use (only `"otlp"` is supported) |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Override endpoint specifically for metrics |
| `OTEL_TRACES_EXPORTER` | Set which traces exporter to use |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Override endpoint specifically for traces |
| `OTEL_LOGS_EXPORTER` | Set which logs exporter to use |
| `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` | Override endpoint specifically for logs |
| `OTEL_PROPAGATORS` | Configure context propagation (e.g., `tracecontext,baggage`) |

## 2. The OperationMetrics Layer

The `OperationMetrics` layer provides Rate, Error, Duration (RED) metrics for all storage operations. This layer should be placed at the top of your service stack to ensure comprehensive coverage.

### Purpose

- **Rate**: Tracks request frequency for each operation
- **Error**: Monitors error rates and types
- **Duration**: Measures operation latency and performance

### Implementation

```rust
use storage_core::metrics::OperationMetricsLayer;

let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("azure")));

// Add to your service builder
.add_service(
    FileObjectBuilder::new()
        .layer(operation_metrics_layer.clone())
        .backend(backend.clone()),
)
```

### Metrics Provided

| Metric | Type | Description | Attributes |
|--------|------|-------------|------------|
| `storage.requests` | Counter | Total number of storage API requests | `method`, `api_version`, `storage_backend`, `result`, `pod_name`, `pkg_version` |
| `storage.request.duration` | Timer | Duration of storage API request processing | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |
| `storage.enumeration.items` | Histogram | Number of items returned per enumeration operation | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |
| `storage.read.object.size` | Histogram | Total size of read objects in bytes | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |
| `storage.read.chunk.size` | Histogram | Size of data chunks received from backend in bytes | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |
| `storage.read.redirects` | Counter | Number of redirect URLs returned for read operations | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |
| `storage.write.operations` | Counter | Number of write operations by upload method | `method`, `api_version`, `storage_backend`, `upload_preference`, `pod_name`, `pkg_version` |

## 3. The SdkMetrics Layer

The `SdkMetrics` layer provides metrics specifically for backend SDK operations. This layer should be placed right before the backend to capture only operations that actually reach the storage backend.

### Purpose

- Tracks backend-specific performance
- Monitors SDK-level errors and latency
- Provides insights into backend behavior separate from cached or short-circuited requests

### Implementation

```rust
use storage_core::metrics::SdkMetricsLayer;

let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("azure")));

// Add to your service builder (after other layers, before backend)
.add_service(
    FileObjectBuilder::new()
        .layer(operation_metrics_layer.clone())
        .layer(sdk_metrics_layer.clone())  // Right before backend
        .backend(backend.clone()),
)
```

### Metrics Provided

| Metric | Type | Description | Attributes |
|--------|------|-------------|------------|
| `storage.sdk.requests` | Counter | Total number of requests to the storage backend SDK | `method`, `api_version`, `storage_backend`, `result`, `pod_name`, `pkg_version` |
| `storage.sdk.request.duration` | Timer | Duration of storage backend SDK requests | `method`, `api_version`, `storage_backend`, `pod_name`, `pkg_version` |

## 4. Defining Custom Metrics

Individual layers and backends can export their own custom metrics using the `metric!` and `metric_base!` macros.

### Using the `metric!` Macro

For simple, static metrics:

```rust
use storage_core_util::observability::metrics::metric;

// Simple counter
metric!(UIntCounter,
    name = "my_counter",
    description = "Number of operations",
).add(1);

// Gauge with attributes
metric!(FloatGauge,
    name = "fuel_level",
    unit = "gallons",
    description = "How much fuel is in the tank",
    { vehicle_id: "truck_1" }
).record(19.3);
```

### Using the `metric_base!` Macro

For metrics that need dynamic attribute values at runtime:

```rust
use storage_core_util::observability::metrics::{metric_base, MetricBinder};

// Define a base metric with attribute placeholders
metric_base!(
    static CACHE_ACCESS_BASE: UIntCounter,
    name = "storage.cache.operations",
    description = "Cache accesses",
    { name, access_type, pod_name, pkg_version }
);

// Bind attributes at runtime
let cache_hit_counter = CACHE_ACCESS_BASE.bind([
    ("name", "my_cache".to_string()),
    ("access_type", "hit".to_string()),
    ("pod_name", pod_name.clone()),
    ("pkg_version", pkg_version.clone()),
]);

cache_hit_counter.add(1);
```

### Available Instrument Types

| Type | Rust Type | Description | Operation |
|------|-----------|-------------|-----------|
| Counter | `UIntCounter` | Monotonically increasing integer | `.add(n)` |
| Counter | `FloatCounter` | Monotonically increasing float | `.add(n)` |
| Gauge | `UIntGauge` | Integer that can go up and down | `.record(n)` |
| Gauge | `FloatGauge` | Float that can go up and down | `.record(n)` |
| Histogram | `InitializedHistogramU64` | Distribution of integer values | `.record(n)` |
| Histogram | `InitializedHistogramF64` | Distribution of float values | `.record(n)` |
| Timer | `TimerMetric` | Measures duration automatically | `.start()` returns guard |

### Histogram Buckets

When creating histograms, use buckets from `HistogramCommonBucket` for consistency across services:

```rust
use storage_core_util::observability::metrics::{metric, HistogramCommonBucket};

metric!(
    Histogram,
    name = "operation_time",
    buckets = HistogramCommonBucket::Time,
).record(elapsed.as_secs_f64());
```

Available bucket types:
- `HistogramCommonBucket::Time` - For timing histograms (durations)
- `HistogramCommonBucket::Data` - For data size histograms (bytes)
- `HistogramCommonBucket::CallCount` - For counting method invocations

## 5. Built-in Component Metrics

### Semaphore Metrics

The `Semaphore` utility automatically exports metrics:

```rust
use storage_core_util::semaphore::Semaphore;

let semaphore = Semaphore::new(100, "connection_pool");
```

| Metric | Description |
|--------|-------------|
| `storage.semaphore.permits` | Current number of available semaphore permits |
| `storage.semaphore.waiters` | Current number of tasks waiting to acquire permits |
| `storage.semaphore.acquire.duration` | Time spent waiting to acquire semaphore permits |
| `storage.semaphore.waiters.max` | Peak number of waiting tasks observed since startup |
| `storage.semaphore.permits.min` | Lowest number of available permits observed since startup |

### Cache Metrics

The `Cache` utility automatically exports metrics:

```rust
use storage_core_util::cache::Cache;

let cache = Cache::new(&cache_args, "stat_cache");
```

| Metric | Description |
|--------|-------------|
| `storage.cache.operations` | Number of cache operations (hits and misses), with `access_type` attribute |
| `storage.cache.entries` | Number of entries in the cache |
| `storage.cache.size` | Estimated size of the cache in bytes |
| `storage.cache.l2.operations` | Number of L2 cache operations, with `op` and `result` attributes |
| `storage.cache.l2.latency_ms` | Latency of L2 cache operations in milliseconds, with `op` attribute |
| `storage.cache.l2.lost_lease` | Number of L2 writes skipped because the fill lease was lost |
| `storage.cache.l2.tombstone_blocked` | Number of L2 writes blocked by an invalidation tombstone |
| `storage.cache.l2.pubsub_message_dropped` | Number of invalidation pubsub messages dropped before they could be applied |
| `storage.cache.l2.pubsub_reconnect` | Number of L2 invalidation pubsub reconnects |

## 6. Protocol-Level Metrics (gRPC and HTTP)

When metrics are enabled via `StorageService::with_metrics()`, protocol-level metrics are automatically collected for both gRPC and HTTP/REST endpoints. These metrics follow the OpenTelemetry semantic conventions.

### gRPC Server Metrics

gRPC metrics follow the [A66: OpenTelemetry Metrics](https://github.com/grpc/proposal/blob/master/A66-otel-stats.md) specification:

| Metric | Type | Description | Unit |
|--------|------|-------------|------|
| `grpc.server.call.started` | Counter | Number of server calls started | `call` |
| `grpc.server.call.duration` | Histogram | End-to-end time taken to complete a call | `s` |
| `grpc.server.call.rcvd_total_compressed_message_size` | Histogram | Total compressed bytes received per RPC | `By` |
| `grpc.server.call.sent_total_compressed_message_size` | Histogram | Total compressed bytes sent per RPC | `By` |

**Attributes:**

| Attribute | Description |
|-----------|-------------|
| `grpc.method` | Full gRPC method path (e.g., `nvidia.omniverse.storage/Stat`) |
| `grpc.service` | gRPC service name (e.g., `nvidia.omniverse.storage`) |
| `grpc.name` | Method name only (e.g., `Stat`) |
| `grpc.status` | gRPC status code (on completion metrics) |

### HTTP Server Metrics

HTTP metrics follow the [OpenTelemetry HTTP Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/):

| Metric | Type | Description |
|--------|------|-------------|
| `http.server.request.duration` | Histogram | Duration of HTTP server requests |
| `http.server.active_requests` | UpDownCounter | Number of active HTTP server requests |
| `http.server.request.body.size` | Histogram | Size of HTTP server request bodies |
| `http.server.response.body.size` | Histogram | Size of HTTP server response bodies |

**Attributes:**

| Attribute | Description |
|-----------|-------------|
| `http.request.method` | HTTP request method (GET, POST, etc.) |
| `http.response.status_code` | HTTP response status code |
| `url.scheme` | The URI scheme (http, https) |
| `http.route` | The matched route template (if available) |
| `network.protocol.version` | The HTTP protocol version (1.1, 2, 3) |

### Enabling Protocol Metrics

```rust
use storage_core_util::observability::metrics::meter;

StorageService::builder()
    .with_metrics(meter!())  // Enables both gRPC and HTTP metrics
    .with_trace_propagation()
    .add_service(my_service)
    .serve(Some(grpc_addr), Some(rest_addr))
    .await?;
```

## 7. Adding Metrics to Custom Interfaces

When adding metrics to your own service interfaces, you should create a dedicated metrics layer and struct that initializes all metric instances at startup. This is important because Prometheus/Grafana will ignore the initial increase in a metric value if it's the first value in the time series ([prometheus/prometheus#1673](https://github.com/prometheus/prometheus/issues/1673)). By pre-creating metric instances when your service starts, you ensure that all metric time series exist from the beginning and initial changes are properly captured.

You should **not** implement your custom trait for the existing `OperationMetrics<S>` struct. The `OperationMetrics` struct pre-initializes metrics for all the storage-core traits in its constructor. If you implement your own trait for it, your metrics would only be created when the methods are first called, not at startup—causing the initial metric values to be lost in Grafana dashboards.

Instead, create your own metrics layer that follows the same pattern: a layer struct, a wrapper struct that holds `Arc<MethodMetrics>` fields initialized in the constructor, and trait implementations that use `record_operation!` to track results.

See the [greeter_service example](../../examples/greeter_service/src/helloworld/metrics.rs) for a complete implementation.

## 8. Common Attributes

All metrics include these standard attributes:

| Attribute | Description | Source |
|-----------|-------------|--------|
| `pod_name` | The name of the pod running the service | `STORAGE_POD_NAME` env var |
| `pkg_version` | The Cargo package version | `CARGO_PKG_VERSION` at compile time |

Operation-specific metrics also include:

| Attribute | Description | Values |
|-----------|-------------|--------|
| `storage_backend` | The storage backend name | Configured per layer |
| `method` | The operation being performed | `stat`, `read`, `write`, `list`, etc. |
| `api_version` | The API version | `v1alpha`, `v1beta` |
| `result` | The operation result | `Ok`, `NotFound`, `PermissionDenied`, etc. |

## 9. OTLP Export

Metrics are exported via OTLP (OpenTelemetry Protocol) to a collector. This is a push-based model where the application sends metrics to an endpoint rather than exposing them for scraping.

### Local Development Setup

For local development, a typical observability stack includes:

1. **OpenTelemetry Collector** (`otel/opentelemetry-collector-contrib`) - Receives OTLP metrics and can export to various backends
2. **Prometheus** (`prom/prometheus`) - Scrapes metrics from the collector and stores them
3. **Grafana** (`grafana/grafana`) - Visualizes metrics from Prometheus

Configure your application to send metrics to the collector:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4317"
export OTEL_METRICS_EXPORTER="otlp"
```

For detailed setup instructions, refer to the [OpenTelemetry Collector documentation](https://opentelemetry.io/docs/collector/getting-started/).

## 10. Example Grafana Dashboard

An example Grafana dashboard is available at [example_dashboard.json](example_dashboard.json). Import it into Grafana to visualize all storage service metrics.
