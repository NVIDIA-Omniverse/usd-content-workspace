# Monitor the stack

> **Applies to:** Storage Service, Event Aggregation and Event Consumer, Thumbnail Service, Discovery operator, Live Edit (pointer), shared observability libraries · **Released:** Storage 1.2.3, Aggregation 1.6.0, Consumer 1.11.0, Thumbnail 0.1.0, Auto Discovery 1.1.1, Live Edit 0.1.1 (Developer Guide 1.3.0) · **This repo's snapshot:** Storage 1.2.3 + Unreleased (`CHANGELOG.md`); Aggregation 1.7.0 and Consumer 1.12.0 (`pyproject.toml`; changelogs show 1.6.0/1.11.0 + Unreleased); Thumbnail 0.1.0 (Cargo); Discovery operator 1.2.0 (Cargo); observability libraries 0.14.0 · **Sources:** [`storage-service/docs/helm/observability.rst`](../../../../storage/reference/storage-service/docs/helm/observability.rst), [`storage-service/helm/values.yaml`](../../../../storage/reference/storage-service/helm/values.yaml), [`storage-core` metrics](../../../../storage/reference/storage-core/storage-core/src/metrics/utils.rs), [Event Consumer values](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/values.yaml), [thumbnail chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md), [`common/observability`](../../../../common/observability/README.md); adapted from the [source skill package](../../NOTICE.md)

Metric names, value keys and defaults change between versions. Confirm the deployed versions ([capture-a-deployment](../versions/capture-a-deployment.md)) and scrape the running service before writing alerts. Notifications operations (queues, DLQ, broker) are in [operate/notifications.md](notifications.md); Live Edit metrics are in [operate/live-edit.md](live-edit.md#metrics).

## Conventions shared by the Rust services

The Rust services use the repo's [`common/observability`](../../../../common/observability/README.md) crates (`otel-tools`, `tonic-observability`, `axum-observability`). Exporters are configured only through `OTEL_*` environment variables, which the charts set.

| Layer | Metrics (OpenTelemetry names) | Spec |
| --- | --- | --- |
| gRPC server (A66) | `grpc.server.call.started`, `grpc.server.call.duration`, `grpc.server.call.sent_total_compressed_message_size`, `grpc.server.call.rcvd_total_compressed_message_size`, `grpc.server.handler.active`, `grpc.server.request.active`, `grpc.server.response.active`; attributes `grpc.method`, `grpc.status` | [grpc-server-metrics-a66.md](../../../../common/observability/docs/layer/grpc-server-metrics-a66.md) |
| gRPC client (A66) | `grpc.client.call.duration`, `grpc.client.attempt.*` | [grpc-client-metrics-a66.md](../../../../common/observability/docs/layer/grpc-client-metrics-a66.md) |
| HTTP server | `http.server.request.started`, `http.server.request.duration`, `http.server.request.body.size`, `http.server.response.body.size`, `http.server.active_requests`; attributes `http.request.method`, `http.route`, `http.response.status_code` | [http-server-metrics.md](../../../../common/observability/docs/layer/http-server-metrics.md) |

Prometheus exposition rewrites dots to underscores and may add unit suffixes; match names from an actual scrape. The Unreleased library change removes the deprecated OTel gRPC server metric layer and reports A66 message sizes without framing bytes, so dashboards built on the old layer or on byte sizes can shift after upgrading.

Traces propagate W3C Trace Context automatically through these layers.

## Storage Service

### Delivery modes

| Mode | Values | Notes |
| --- | --- | --- |
| Prometheus scrape | `service.metrics.enabled: true`, `service.metrics.port` (`8013`) | Default is `false`. The port is added to the Services only when enabled. |
| OTLP push | `observability.metrics.enabled: true`, `observability.metrics.endpoint.grpc`, `exportIntervalMs` (`15000`) | Takes precedence; when both are on, Prometheus is not the exporter |
| Traces | `observability.tracing.enabled`, `.endpoint.grpc`, optional `.samplingRatio` (string `0`–`1`, parent-based) | OTLP/gRPC only |
| Logs | stdout; `observability.logging.structured.enabled` for JSON; `config.logging.level`, `config.logging.extraTargets`, `config.logging.backtrace` | |
| Metric prefix | `config.metrics.prefix` (default `storage`) | Empty string emits unprefixed names |

The source package said `/metrics` on 8013 is always on through `OTEL_METRICS_EXPORTER=prometheus`. This snapshot defaults `service.metrics.enabled` to `false`; enable it explicitly.

```yaml
service:
  metrics:
    enabled: true
    port: 8013
podAnnotations:
  prometheus.io/scrape: "true"
  prometheus.io/path: "/metrics"
  prometheus.io/port: "8013"
```

Header propagation: `config.additionalHeaders.tracingHeaderNames` (default `[x-request-id]`) goes to storage backends; `observability.headers.forward` goes to the Permission Service. Forwarded headers are caller-controlled unless a trusted proxy sets or strips them.

### Metric families in this snapshot

Names below are as defined in source, before the prefix and exporter normalization.

| Family | Instruments | Defined in |
| --- | --- | --- |
| Requests | `requests`, `request.duration`, `sdk.requests`, `sdk.request.duration` | `storage-core/src/metrics/utils.rs` |
| Objects | `enumeration.items`, `read.redirects`, `read.chunk.size`, `read.object.size`, `write.operations` | same |
| Metadata cache | `metadata_cache_access`, `metadata_cache_miss`, `metadata_cache_size`, `metadata_cache_entries` | `storage-service/src/backend/metadata/cache/metrics.rs` |
| PostgreSQL metadata | `postgres_metadata_query_latency`, `postgres_metadata_query_results` | `backend/metadata/postgres/metrics.rs` |
| CosmosDB / DynamoDB metadata | `azure_cosmosdb_metadata_operation_*`, `aws_dynamodb_metadata_operation_*` | `backend/metadata/*/metrics.rs` |
| SQS / Service Bus subscribers | `sqs_operation_results`, `sqs_operation_latency`, `sqs_operation_active_requests`; `azure_service_bus_operation_*` | `pubsub/*/metrics.rs` |
| Notification publish | `notification_service_batch_publish_events_latency`, `..._active_requests` | `pubsub/notification/metrics.rs` |
| Synthetic directory events | `storage_synthetic_dir_events_publish_results` | `pubsub/synthetic_dir_events/metrics.rs` |
| Permission client | `storage_permission_grpc_check_permission_batch_latency`, `storage_permission_rest_check_permission_batch_latency` | `storage-service-permissions/src/client/*/metrics.rs` |

The source package listed per-call S3/Azure-Blob/Azure-Table metadata latencies and per-RPC SQS/Service Bus histograms. The S3, Azure Blob and Azure Table metadata backends were removed in 1.2.0, and this snapshot consolidates subscriber metrics into `*_operation_*` instruments. Dashboards built from the old list need rework.

Ingress load shedding returns `429` (REST) or `RESOURCE_EXHAUSTED` (gRPC); alert on those statuses when `config.ingressLoadShed.enabled` is `true` (default) ([request-throttling.rst](../../../../storage/reference/storage-service/docs/helm/request-throttling.rst)).

## Event Aggregation and Event Consumer

Both services prefer the signal-specific block; either enabled signal overrides the deprecated `telemetry` block.

```yaml
observability:
  metrics:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
  tracing:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
```

- gRPC transport only; `endpoint.http` is ignored.
- Log export turns on whenever either signal is on.
- `observability.tracing.samplingRatio` is intentionally absent; all traces export.
- To turn export off, disable both `observability` signals and legacy `telemetry.enabled`.
- Env mapping: `OMNI_EVENTS_METRICS_ENABLED`, `OMNI_EVENTS_TRACING_ENABLED`, `OMNI_EVENTS_OTLP_METRICS_ENDPOINT`, `OMNI_EVENTS_OTLP_TRACING_ENDPOINT`.

Event Consumer instruments (`app/lib/observability/`): `events_by_type_total` (label `event_type`), `events_processed_successfully_total`, `events_processing_failed_total`, `event_processing_duration_ms`; SSE `sse.server.active_connections`, `sse.server.connections_total`, `sse.server.connection_duration`, `sse.server.events_sent_total`, `sse.server.events_per_connection`, `sse.server.connection_errors_total`. Shared gRPC metrics come from `notification-common`. Broker and queue signals: [operate/notifications.md](notifications.md).

## Thumbnail Service

Both Deployments share `observability.*` values.

| Key | Default | Effect |
| --- | --- | --- |
| `observability.logging.level` | `info` | `RUST_LOG` on both; JSON logs are always on |
| `observability.otel.endpoint` | `""` | When set, enables OTLP traces and metrics; when empty, exporters are `none` |
| `observability.otel.tracesSampler` / `tracesSamplerArg` | `parentbased_traceidratio` / `0.1` | Worker spans inherit the producer's decision |
| `observability.otel.resourceAttributes` | `""` | Appended to `OTEL_RESOURCE_ATTRIBUTES`; `service.instance.id` is the pod name |

The orchestrator has no Prometheus endpoint; use OTLP and the HTTP/gRPC layer metrics. The worker serves Prometheus text on `:8080/metrics` (`workerRuntime.healthPort`):

| Metric | Watch for |
| --- | --- |
| `thumbnail_worker_outcomes_total{outcome="success"\|"permanent_failure"\|"retryable_failure"}` | Throughput and failure ratio |
| `thumbnail_worker_failures_total{class="validation"\|"unsupported_processor"\|"unsupported_format"\|"source_missing"\|"decode"\|"upload"\|"associate"}` | Cause of failures |
| `thumbnail_worker_conversion_milliseconds_total` | Conversion time |
| `thumbnail_worker_unacknowledged_deliveries` | Rising: retryables accumulating; queue heading to max length |
| `thumbnail_worker_nacks_total{reason="retryable_failure"\|"early_redelivery"}` | Use `sum by (reason) (rate(...[5m]))`; failures climbing while NACKs flatline means settlements are not reaching the broker |
| `thumbnail_worker_stream_recycles_total` | Sustained rise: NACKs not getting through or broker ignoring the delay |
| `thumbnail_worker_stream_token_reconnects_total` | Climbing near 1 Hz: token refresh is not clearing expiry |

Counters are per pod and reset on restart. Pair them with WorkerEventQueue depth and its dead-letter count from the Notification side. Traces span queue dwell: a trace lasting minutes whose spans sum to seconds indicates backlog, not slow conversion. Storage CREATE events do not yet carry trace context, so event-driven traces start at the orchestrator.

Probes: orchestrator `/health/live` (liveness) and `/health` (readiness, `503` until gRPC serves) on 3000; worker `/healthz` and `/readyz` (Ready only with an established consume stream).

```bash
kubectl -n <namespace> port-forward deploy/thumbnail-service-thumbnail-task-worker 8080:8080
curl -s localhost:8080/metrics | grep -E 'thumbnail_worker_(outcomes|unacknowledged|nacks)'
```

## Discovery operator

`observability.logging.level`, `observability.tracing.*` and `observability.metrics.*` (OTLP push) in the [operator values](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml). Health: `GET /api/v2/services` on the internal port (8080) returns `"schema-version": 2`. Check that every expected service appears and that its advertised URLs work from the client network ([deploy/discovery.md](../deploy/discovery.md)).

## Quick checks

```bash
NS=<namespace>
kubectl -n "$NS" get pods,svc,endpoints
kubectl -n "$NS" get events --sort-by=.lastTimestamp | tail -30
kubectl -n "$NS" top pods                                   # needs metrics-server
kubectl -n "$NS" port-forward svc/storage-service 8013:8013 # only with service.metrics.enabled
curl -s localhost:8013/metrics | head
kubectl -n "$NS" logs deploy/<name> --since=30m
kubectl -n "$NS" logs deploy/<name> --previous               # after a crash
```

## Starting alert set

| Signal | Condition |
| --- | --- |
| Storage request latency | p99 of `request.duration` above baseline |
| gRPC errors | rate of `grpc.server.call.duration` with non-`OK` `grpc.status` |
| Load shedding | any sustained `429` / `RESOURCE_EXHAUSTED` |
| Metadata cache | `metadata_cache_miss` / `metadata_cache_access` rising |
| Event processing | `events_processing_failed_total` rate above zero |
| Thumbnail backlog | `thumbnail_worker_unacknowledged_deliveries` rising, or WorkerEventQueue depth growing while `outcome="success"` is flat |
| Live Edit | see [operate/live-edit.md](live-edit.md#metrics) |
| Pods | restarts, `CrashLoopBackOff`, readiness flapping |
