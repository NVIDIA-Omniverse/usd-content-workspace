(thumbnail-service-observability-observability)=
# Observability

Both processes — the orchestrator and `task-worker` — are instrumented with [OpenTelemetry](https://opentelemetry.io/) for metrics and traces, and emit structured logs through a shared logging layer. This page describes what each process emits, how a single thumbnail becomes a single distributed trace, which log levels are used, and how to configure all of it through the Helm chart.

(thumbnail-service-observability-opentelemetry-overview)=
## OpenTelemetry Overview

Metrics and traces are exported over the **OTLP protocol (gRPC)** to a collector you supply, for example the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/). Export is driven entirely by the standard OpenTelemetry environment variables, which the chart injects from independent `observability.tracing.*` and `observability.metrics.*` values — each signal can point at its own collector:

- When `observability.tracing.enabled` is true and `observability.tracing.endpoint.grpc` is set, both pods receive `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_TRACES_EXPORTER`, `OTEL_TRACES_SAMPLER`, and `OTEL_TRACES_SAMPLER_ARG`. Otherwise `OTEL_TRACES_EXPORTER=none` is injected, so nothing attempts to reach a collector at `localhost:4317`.
- When `observability.metrics.enabled` is true and `observability.metrics.endpoint.grpc` is set, both pods receive `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` and `OTEL_METRICS_EXPORTER`. Otherwise `OTEL_METRICS_EXPORTER=none` is injected.
- Logs are unaffected by either signal and always continue to stdout.
- `OTEL_SDK_DISABLED=true` is a full kill switch: it also suppresses stdout logging, not just traces and metrics. To disable OTLP export while keeping logs, use the per-signal `enabled` flags instead — both default to `true` but take effect only once their endpoint is set.

All exported telemetry carries a common OpenTelemetry `Resource`: `service.name` (`thumbnail-service` or `thumbnail-task-worker`), `service.namespace` (`omniverse`), `service.version` (the compiled-in service version), and `service.instance.id`, which the chart always sets to the pod name through the downward API so replicas can be told apart. The chart deliberately does **not** inject `OTEL_SERVICE_NAME`: each binary passes its own name at startup, and a code-supplied name takes priority over the environment, so the variable would be ignored.

(thumbnail-service-observability-metrics)=
## Metrics

The two processes have different metric profiles. The worker exposes **service-specific** counters, gauges, and histograms describing conversion outcomes and the health of its consume stream. The service exposes **generic transport** metrics for its REST and gRPC surfaces; it has no service-specific instruments of its own, and its operational state is read from structured log lines (see {ref}`Log levels <thumbnail-service-observability-log-levels>`).

(thumbnail-service-observability-worker-metrics)=
### Worker metrics

Every worker metric is emitted twice: as an OpenTelemetry instrument on the global meter (exported over OTLP) and as Prometheus text on `GET /metrics` on the worker's probe port (`HEALTH_PORT`, default `8080`). The Prometheus endpoint always serves, whatever the OTLP configuration. Every label value is pre-registered at zero, so an absent class reads as `0` rather than as a missing series.

| Name | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `thumbnail_worker_outcomes_total` | Counter | messages | `outcome` = `success` \| `permanent_failure` \| `retryable_failure` | Generate messages by terminal outcome. |
| `thumbnail_worker_failures_total` | Counter | failures | `class` = `validation` \| `unsupported_processor` \| `credential` \| `source_missing` \| `download` \| `unsupported_format` \| `decode` \| `encode` \| `upload` \| `associate` | Failures by the pipeline stage that produced them, permanent and retryable alike. See {ref}`Failure classes <thumbnail-service-generation-pipeline-failure-classes>`. |
| `thumbnail_worker_conversion_duration_seconds` | Histogram | `s` | — (buckets `0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, +Inf`) | Elapsed time for messages that **reached conversion** — successes and `decode`/`encode`/`upload`/`associate` failures. Earlier rejections are excluded so fast rejects do not depress the distribution. |
| `thumbnail_worker_unacknowledged_deliveries` | Gauge | deliveries | — | Deliveries received but not yet acknowledged or negatively acknowledged (in flight plus withheld). Reset to `0` on every reconnect, because the broker redelivers everything unacknowledged. |
| `thumbnail_worker_nacks_total` | Counter | deliveries | `reason` = `retryable_failure` \| `early_redelivery` | Negative acknowledgements actually sent, by cause. |
| `thumbnail_worker_stream_recycles_total` | Counter | — | — | Deliberate consume-stream teardowns used to force redelivery — the fallback when a NACK cannot be delivered, and the circuit breaker after repeated early redeliveries. A healthy worker never recycles. |
| `thumbnail_worker_session_ends_total` | Counter | — | `cause` = `recycle` \| `token_expiring` \| `auth_refreshed` \| `queue_missing` \| `interrupted` | Consume sessions ended, by cause. |
| `thumbnail_worker_orphaned_blobs_total` | Counter | — | — | Uploaded thumbnails left unreferenced because the association was rejected permanently after the upload. The blob is not deleted; reconcile from the destination named in the accompanying error log. |
| `thumbnail_worker_stream_token_reconnects_total` | Counter | — | — | Stream reopens after a near-expiry token refresh produced a usable bearer. |

How to read them together:

- `thumbnail_worker_unacknowledged_deliveries` **rising** means retryable failures are accumulating faster than they retire and WorkerEventQueue is heading for its maximum length. Both an acknowledgement and a negative acknowledgement release a delivery, so the gauge reflects work genuinely in flight.
- Graph `thumbnail_worker_nacks_total` as a rate — `sum by (reason) (rate(thumbnail_worker_nacks_total[5m]))`. `retryable_failure` is the retry rate; a steady climb means a dependency is persistently failing. `early_redelivery` means the broker is redelivering ahead of the configured delay and the consumer is rejecting the redelivery without reprocessing it; sustained, that trips the circuit breaker. Failures climbing while NACKs flatline means settlements are not reaching the broker.
- Any sustained rise in `thumbnail_worker_stream_recycles_total` means negative acknowledgements are not getting through or the broker is ignoring the redelivery delay.
- A 1 Hz climb in `thumbnail_worker_stream_token_reconnects_total` means refresh is not clearing expiry — a sick identity provider, or a sidecar token file still inside its skew window.

These counters are per pod and reset on restart. Use them for outcome rates, WorkerEventQueue depth for backlog, and the Notification Service's dead-letter management for individual poison messages. There is no per-item task status to query.

(thumbnail-service-observability-generic-transport-metrics)=
### Generic transport metrics

The service applies shared REST and gRPC request-metrics middleware, which emits request metrics following the [OpenTelemetry HTTP](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/) and [RPC](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-metrics/) semantic conventions. They are exported over OTLP only; the service has no Prometheus scrape endpoint.

(thumbnail-service-observability-rest-metrics)=
#### REST metrics

Attributed with `http.request.method`, `http.route` (the matched route template, for example `/v1alpha/thumbnails`, never the raw path), `http.response.status_code`, and `network.protocol.version`:

| Name | Type | Unit | Description |
|---|---|---|---|
| `http.server.request.count` | Counter | `requests` | Total HTTP requests received. |
| `http.server.request.errors` | Counter | `errors` | Requests that returned a 4xx or 5xx status. |
| `http.server.request.duration` | Histogram | `s` | Duration of inbound requests from call entry to the start of the response. |
| `http.server.request.body.size` | Histogram | `By` | Request body size consumed by the handler. |
| `http.server.response.body.size` | Histogram | `By` | Response body size observed by the client. |

(thumbnail-service-observability-grpc-metrics)=
#### gRPC metrics

Attributed with `rpc.service`, `rpc.method`, and the final `rpc.grpc.status_code`. Every `ThumbnailService` method is known to the metrics layer up front, so no method is missing from the label set on first use:

| Name | Type | Unit | Description |
|---|---|---|---|
| `rpc.server.request.count` | Counter | `requests` | Total gRPC requests received. |
| `rpc.server.duration` | Histogram | `ms` | Duration of gRPC request handling. |
| `rpc.server.request.error.count` | Counter | `requests` | Requests that completed with a non-OK status. |

The worker's outbound calls (storage `Read`/`Write`, `AssociateThumbnail`) and the service's outbound calls (storage metadata, `Stat`, publishes) go through the matching client layers, which emit `rpc.client.*` spans and a `rpc.client.call.exception` warning for every failed attempt.

(thumbnail-service-observability-tracing)=
## Tracing

Tracing uses the same logging layer described above, bridged to OpenTelemetry. The service creates one server span per inbound REST request (named after the method and matched route) and one per gRPC RPC (named after the full method path, with the final status code); the worker creates one `thumbnail.generate` span per consumed message with child spans `thumbnail.download`, `thumbnail.convert`, `thumbnail.upload`, and `thumbnail.associate`. The storage-event consumer creates one `storage.event` span per delivery covering eligibility, fan-out, and the acknowledgement decision — ineligible deliveries are spanned too, so a skipped file is visible in the trace backend.

(thumbnail-service-observability-distributed-tracing)=
### Distributed tracing

Distributed tracing **is supported** end to end. Both processes install the **W3C Trace Context** propagator, and trace context crosses every hop this service controls:

1. An inbound REST or gRPC request's `traceparent`/`tracestate` becomes the parent of the server span. With no valid context, a new root trace starts.
2. Every `omni.thumbnail.generate` event carries the publishing span's `traceparent` and `tracestate` in its message body (see {ref}`omni.thumbnail.generate <thumbnail-service-notifications-omni-thumbnail-generate>`).
3. The worker extracts that context and makes it the **remote parent** of its `thumbnail.generate` span, so the conversion appears in the same trace as the request or storage delivery that caused it.
4. The worker's outbound `AssociateThumbnail` call injects its context, so the service's server span for that RPC is a child of the worker's span.
5. Association-event publishes run as their own `thumbnail.association_event` span with the request as remote parent — not as a local descendant — so a slow publish never inflates the request span's exported duration.

**One thumbnail is therefore one trace**, from the API call or storage delivery through the queue, the conversion, and the association. Three consequences are worth knowing before tuning:

- **Traces span queue dwell.** A trace stays open from publish until the worker finishes, so its wall-clock duration includes queue wait. A backlogged worker produces long traces whose span durations sum to seconds — that is queue depth, not slow work.
- **Worker spans inherit the producer's sampling decision.** The sampler is always `parentbased_traceidratio`; the sampled flag travels in the trace context, so lowering `observability.tracing.samplingRatio` drops conversion spans for every request that was not sampled at the root. Sample for the volume of *conversions* you want to see, not the volume of API calls.
- **The first hop is storage's to add.** The storage service does not yet stamp `omni.storage.created` with trace context, so an event-driven thumbnail's trace starts at this service's `storage.event` span rather than at whatever wrote the source object. When storage starts sending one, both `traceparent` and `tracestate` are honored on that hop.

A message with absent or malformed trace context is processed normally and roots a new trace — telemetry is never a precondition for work. Oversized inbound `traceparent` values are refused with a single bounded warning and the delivery outcome is unchanged.

(thumbnail-service-observability-logging)=
## Logging

Logs are written to **stdout as structured JSON**. Each record carries `trace_id` and `span_id` when a span is active, so a log pipeline that also ingests traces (for example Loki alongside Tempo) can pivot between the two. There is no pretty/text mode, and no logs-only toggle: the only way to suppress stdout logging is `OTEL_SDK_DISABLED=true` (see {ref}`OpenTelemetry Overview <thumbnail-service-observability-opentelemetry-overview>`), which also disables traces and metrics. The chart sets `OTEL_LOGS_EXPORTER=console` on both containers, which selects stdout rather than OTLP log export.

Log filtering uses `RUST_LOG` (Helm `observability.logging.level`): a level (`info`, `debug`), or comma-separated `<component>=<level>` pairs to set the level per component. When `RUST_LOG` is unset or unparseable the effective level is `info`; a typo never prevents startup.

(thumbnail-service-observability-log-levels)=
### Log levels

Both binaries use the standard log levels with these conventions:

- **`error`** — Failures that need investigation or leave state behind. Examples: a generate message permanently rejected (`Permanently rejecting a generate message`, naming destination and failure class — the only durable trace of a poison message before it is acknowledged); an uploaded thumbnail left orphaned by a permanent association rejection; a consume stream interrupted (`Consume stream interrupted; reconnecting`); a token refreshed and rejected again; the early-redelivery circuit breaker firing; a rollback of an association write that itself failed (names the address and key to reconcile by hand); a generated blob that could not be deleted after its metadata was removed.
- **`warn`** — Degraded but self-healing conditions. Examples: a retryable failure being scheduled for spaced redelivery; the NACK deferral bound reached; an unknown event type on a durable queue; a stored association entry skipped because it has no thumbnail address; a partial thumbnail list returned; the storage-event consumer at its concurrency bound (`[BACKPRESSURE]`); CORS enabled with no usable origins; an association-event publish skipped because no aggregation endpoint is configured.
- **`info`** — Lifecycle and operator-visible state. Examples: listener bind confirmations (`[HTTP] Server listening on …`, `[GRPC] Server listening on …`); durable queue provisioning (`[PROVISION] Durable queue created` / `already existed`); consume stream established; `Thumbnail generated and associated` with elapsed milliseconds; token refreshed and reconnecting; the storage-deleted sweep summary (`N succeeded, M failed, K blobs left as caller-owned`); the periodic StorageEventQueue `[STATS]` line every 300 s (totals, filtered, active/capacity, unacknowledged, deferred, stream age, time since last delivery); `[SHUTDOWN]` messages during drain.
- **`debug`** — Per-request diagnostics. Examples: `Skipping CREATE event` with the ineligibility reason; the resolved conversion specification; storage metadata retry attempts (`[RETRY] <op> attempt i/n failed`, demoted from `warn` because the client trace layer already reports each failed attempt); association recorded by the service; source object read and thumbnail object written.
- **`trace`** — Reserved. Neither binary emits trace-level records of its own; the level is wired up for dependencies.

`info` is the recommended steady-state level; raise it to `debug` for focused troubleshooting — in particular, CREATE skip reasons are only visible at `debug`.

(thumbnail-service-observability-configuring-observability-with-the-helm-chart)=
## Configuring Observability with the Helm Chart

All observability settings live under the root `observability` key and apply to **both** Deployments. The chart translates them into `RUST_LOG` and the `OTEL_*` variables listed in the overview.

(thumbnail-service-observability-helm-values-reference)=
### Helm values reference

| Value | Default | Description |
|---|---|---|
| `observability.logging.level` | `"info"` | `RUST_LOG` filter for both binaries: a level (`info`, `debug`), or comma-separated `<component>=<level>` pairs to set the level per component. |
| `observability.tracing.enabled` | `true` | Takes effect only when `observability.tracing.endpoint.grpc` is also set; otherwise the trace exporter is set to `none`. |
| `observability.tracing.endpoint.grpc` | `""` | OTLP/gRPC collector endpoint for trace spans, for example `http://otel-collector:4317`. |
| `observability.tracing.samplingRatio` | `"0.1"` | Sampling ratio for `parentbased_traceidratio` sampling, `0.0`–`1.0`. `0.1` samples 10 % of root traces. |
| `observability.metrics.enabled` | `true` | Takes effect only when `observability.metrics.endpoint.grpc` is also set; otherwise the metrics exporter is set to `none`. |
| `observability.metrics.endpoint.grpc` | `""` | OTLP/gRPC collector endpoint for metrics. Independent of `observability.tracing.endpoint.grpc` — each signal can point at its own collector. |
| `observability.resourceAttributes` | `""` | Additional `key=value,key2=value2` pairs appended to `OTEL_RESOURCE_ATTRIBUTES`. `service.instance.id` is always set to the pod name and need not be listed. |

(thumbnail-service-observability-example-export-metrics-and-traces)=
### Example: export metrics and traces

The following values block points both exporters at a collector inside the cluster, raises the log level for focused troubleshooting, and samples half of all root traces:

```{code-block} yaml
:caption: thumbnail-values.yaml

observability:
  logging:
    level: "debug"
  tracing:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
    samplingRatio: "0.5"
  metrics:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
  resourceAttributes: "deployment.environment=production"
```

(thumbnail-service-observability-additional-environment-variables)=
### Additional environment variables

The OpenTelemetry SDK honors its [standard environment variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) beyond the ones the chart exposes as first-class values — for example `OTEL_EXPORTER_OTLP_HEADERS` or `OTEL_METRIC_EXPORT_INTERVAL`. Inject them through each component's `extraVars` map when a deployment needs to authenticate to a managed collector or tune an interval the chart doesn't surface:

```{code-block} yaml
:caption: thumbnail-values.yaml

thumbnailService:
  extraVars:
    OTEL_EXPORTER_OTLP_HEADERS: "authorization=Bearer <token>"
thumbnailWorker:
  extraVars:
    OTEL_EXPORTER_OTLP_HEADERS: "authorization=Bearer <token>"
```

(thumbnail-service-observability-references)=
## References

- [OpenTelemetry](https://opentelemetry.io/) — the metrics and tracing standard both processes instrument
- [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) — the collector both exporters send OTLP to
- [OpenTelemetry HTTP semantic conventions](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/) — REST request-metric attributes
- [OpenTelemetry RPC semantic conventions](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-metrics/) — gRPC request-metric attributes
- [OpenTelemetry SDK environment variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) — the standard `OTEL_*` variables both binaries honor
- [W3C Trace Context](https://www.w3.org/TR/trace-context/) — the propagation format both processes use for distributed tracing

(thumbnail-service-observability-see-also)=
## See also

- {ref}`Conformance and smoke tests <thumbnail-service-conformance-and-smoke-tests-conformance-and-smoke-tests>` — the next topic: the standalone test binaries that verify a deployment.
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the pipeline stages and failure classes the worker metrics are labeled with, and the `/healthz`, `/readyz`, `/metrics` probe surface.
- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — the `[STATS]`, `[BACKPRESSURE]`, and `[PROVISION]` log lines that carry the service-side consumer's state.
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>` — how trace context rides the `omni.thumbnail.generate` message.
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>` — which metric or log line to look at for each symptom.
- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — the full Helm chart reference including `observability` in its wider context.
