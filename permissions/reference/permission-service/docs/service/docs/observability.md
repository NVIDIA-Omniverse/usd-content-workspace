(permission-service-observability-observability)=
# Observability

Permission Service is instrumented end-to-end with [OpenTelemetry](https://opentelemetry.io/) for metrics and traces, and writes structured logs to standard output. This page describes what the service emits, how distributed tracing is wired up, which log levels are used, and how to configure all three through the Helm chart.

(permission-service-observability-opentelemetry-overview)=
## OpenTelemetry Overview

Metrics and traces are exported using the **OTLP protocol over gRPC** to a user-supplied collector (for example, the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/)). Each exporter is configured independently:

- Metrics are exported periodically to the endpoint in `observability.metrics.endpoint.grpc`, and only when `observability.metrics.enabled` is `true`.
- Traces are exported in batches to the endpoint in `observability.tracing.endpoint.grpc`, and only when `observability.tracing.enabled` is `true`.

Both exporters are disabled by default (`observability.metrics.enabled` and `observability.tracing.enabled` default to `false`); while an exporter is disabled its data is dropped locally and no collector is required. Enabling an exporter requires setting both its `enabled` flag and the matching `endpoint.grpc` value — turning on the flag without an endpoint leaves the exporter with nowhere to send data. Logs are always written to stdout regardless of OpenTelemetry configuration.

All exported telemetry carries the same OpenTelemetry resource attributes: `service.name` from `observability.serviceName`, `service.instance.id` from the pod's hostname, and `service.version` from the version of the running build. Data from individual pods can therefore be distinguished in a multi-replica deployment and correlated with a specific build during rollouts or incident response.

(permission-service-observability-metrics)=
## Metrics

The service exposes two kinds of metrics: **service-specific** metrics that describe authorization behavior and internal caches, and **generic transport** metrics that describe the REST and gRPC surfaces.

All metrics are OpenTelemetry instruments, and their names follow the [OpenTelemetry semantic conventions](https://opentelemetry.io/docs/specs/semconv/) where applicable.

(permission-service-observability-service-specific-metrics)=
### Service-specific metrics

These metrics capture the service's authorization decisions and the state of its in-memory caches. They describe authorization behavior rather than the REST and gRPC surfaces.

| Name                             | Type      | Unit       | Attributes | Description                                                                                                     |
|----------------------------------|-----------|------------|------------|-------------------------------------------------------------------------------------------------------------------|
| `authorization.request.allowed`  | Counter   | `requests` | `service`  | Total number of authorization requests that resulted in an `allow` decision.                                    |
| `authorization.request.denied`   | Counter   | `requests` | `service`, `deny_reason` = `explicit_forbid` \| `implicit` \| `cross_principal_denied` \| `evaluation_error` | Total number of authorization requests that resulted in a `deny` decision, categorized by why. |
| `evaluation.cache.size`          | Gauge     | —          | —          | Current number of entries held in the authorization-result cache.                                               |
| `evaluation.cache.hit`           | Counter   | `hits`     | —          | Evaluation-cache lookups that found a cached decision.                                                          |
| `evaluation.cache.miss`          | Counter   | `misses`   | —          | Evaluation-cache lookups that required a fresh policy evaluation.                                               |
| `metadata.cache.size`            | Gauge     | —          | —          | Current number of entries held in the service-metadata cache.                                                   |
| `metadata.cache.hit`             | Counter   | `hits`     | —          | Metadata-cache lookups that found cached metadata.                                                              |
| `metadata.cache.miss`            | Counter   | `misses`   | —          | Metadata-cache lookups that required a storage fetch.                                                           |
| `policy_set.cache.size`          | Gauge     | —          | —          | Current number of parsed Cedar policy sets held in the policy-set cache.                                        |
| `policy_set.cache.hit`           | Counter   | `hits`     | —          | Policy-set cache lookups that found an already parsed policy set.                                               |
| `policy_set.cache.miss`          | Counter   | `misses`   | —          | Policy-set cache lookups that required parsing policy text.                                                     |
| `token.cache.size`               | Gauge     | —          | —          | Current number of validated access tokens and principals held in the token cache.                               |
| `service.build_info`             | Gauge     | —          | `version`  | Always `1`. Presence plus the `version` attribute lets dashboards join other metrics by deployed version.       |

The cache gauges are refreshed on every scrape, so their sampled values reflect live cache utilization. The
evaluation-cache, metadata-cache, and policy-set-cache hit/miss counters are incremented on every lookup and can be
combined into a hit ratio (`hit / (hit + miss)`) to gauge cache effectiveness.

`service` is the target service name from the evaluated action (`action.service`), a small, bounded-cardinality
value that lets dashboards break decisions down per target service without attaching per-request identifiers
(principal, action name, or resource) that would explode time-series cardinality.

`deny_reason` distinguishes four structurally different deny paths: `explicit_forbid` (a policy explicitly denied the
request), `implicit` (no policy matched at all), `cross_principal_denied` (a cross-principal `permissions:view` check
failed), and `evaluation_error` (an internal evaluation error was converted into a fail-closed deny on the batch
path). It is always one of these four fixed values, never free-text from a policy body or error message.

(permission-service-observability-policy-evaluation-metrics)=
### Policy evaluation metrics

These metrics capture how much work a single **uncached** authorization decision required — see
{ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` for the order/priority evaluation model that
`order_groups_scanned` refers to. Because an evaluation-cache hit skips policy retrieval and Cedar evaluation
entirely, these histograms only reflect the cost of cache misses, which is the case operators most need to diagnose.

| Name                                             | Type      | Unit       | Attributes                          | Description                                                                                  |
|---------------------------------------------------|-----------|------------|---------------------------------------|--------------------------------------------------------------------------------------------------|
| `authorization.evaluation.policies_evaluated`     | Histogram | `policies` | `service`                             | Number of policies passed into the evaluator for a single action evaluation.                     |
| `authorization.evaluation.order_groups_scanned`   | Histogram | `groups`   | `service`                             | Number of distinct `order` groups iterated before a decision was reached (short-circuit depth).  |
| `authorization.policy_fetch.size`                 | Histogram | `policies` | `service`, `source` = `single` \| `batch` | Number of policies returned by a single storage fetch.                                           |

`service` is the target service name from the evaluated action (`action.service`), a small, bounded-cardinality
value. `source` distinguishes single-action fetches (single authorization calls, diagnostics calls, and
meta-permission checks) from the batched per-`(service, principal)` fetch used by batch authorization calls.

(permission-service-observability-policy-management-metrics)=
### Policy management metrics

These metrics give a business-level view of policy create/update/delete activity — separate from
the {ref}`generic database metrics <permission-service-observability-service-specific-metrics>`,
which only describe storage-call health, not whether administrators are submitting policies that
fail validation or lack `permissions:edit`. See {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` for the endpoints these cover.

| Name                          | Type      | Unit       | Attributes                                                                                                          | Description                                                                       |
|-------------------------------|-----------|------------|-----------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------|
| `policy.mutation.count`       | Counter   | `policies` | `operation` = `create` \| `update` \| `delete` \| `batch`, `outcome` = `success` \| `invalid` \| `forbidden` \| `error` | Every policy create/update/delete attempt, by outcome.                                |
| `policy.mutation.batch_size`  | Histogram | `policies` | —                                                                                                                       | Number of policy items submitted in a single `PUT /v1beta/policies/batch/` call.      |

`operation="batch"` is only used for a whole-batch failure that occurs before per-item results
exist (a request-level validation failure, or a permission check that fails for one item and
aborts the whole batch, per the all-or-nothing semantics of `PUT /v1beta/policies/batch/`).
Successful batch items are recorded individually as `operation="create"` / `operation="update"`,
so batch and single-item activity roll up into the same counts. `outcome="forbidden"` also covers
policy mutations rejected for lacking `permissions:edit`, which otherwise produce no metric signal
on the CRUD surface.

(permission-service-observability-notification-metrics)=
### Notification metrics

These metrics cover the outcome of publishing policy-change and metadata-change events to the Notification Service
(Event Aggregation Service), and consuming them back for cross-replica cache invalidation — see
{ref}`Service notifications <permission-service-notifications-service-notifications>` for when events are emitted and what they contain. The publish metrics
are only recorded when an event publisher is configured (`notifications.publisher.enabled: true`); the consume metrics only
when the consumer is configured (`notifications.consumer.enabled: true`).

| Name                              | Type      | Unit     | Attributes                                                                | Description                                                                                 |
|------------------------------------|-----------|----------|----------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| `notification.publish.count`      | Counter   | `events` | `outcome` = `success` \| `retried_success` \| `failure` \| `retried_failure`; `event_type` = `omni.permissions.changed` \| `omni.permissions.meta.changed` | Outcome of each publish attempt, including the retry-after-token-refresh case. `event_type` distinguishes policy-change from metadata-change events. |
| `notification.publish.duration`   | Histogram | `ms`     | —                                                                            | Wall-clock time from the first publish attempt to the final outcome (including any retry).   |

`retried_success` and `retried_failure` cover the case where the first attempt received `Unauthenticated`, the
client-credentials token was refreshed, and the publish was retried once — see
{ref}`When events are published <permission-service-notifications-when-events-are-published>`. A rising rate of
`retried_success` is a useful early-warning signal even though it isn't user-visible as an error: it usually means
the client-credentials token is expiring earlier than expected, or being invalidated server-side.

The following metrics cover the built-in cross-replica cache-invalidation consumer — see
{ref}`Consuming change events for cross-replica cache invalidation <permission-service-notifications-cross-replica-cache-invalidation>`.
They are only recorded when the consumer is configured (`notifications.consumer.enabled: true`).

| Name                                   | Type      | Unit         | Attributes                                                                                     | Description                                                                                                                                             |
|----------------------------------------|-----------|--------------|-------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| `notification.consume.count`           | Counter   | `events`     | `event_type` = `omni.permissions.changed` \| `omni.permissions.meta.changed`; `outcome` = `invalidated_targeted` \| `invalidated_service` \| `invalidated_all` \| `invalidated_all_parse_error` | One per consumed event that triggered a local invalidation. `outcome` records whether a targeted key, a service, or the whole cache was invalidated (the `parse_error` variant is a full-flush fallback). |
| `notification.consume.errors`          | Counter   | `errors`     | `reason` = `unauthenticated` \| `transport` \| `stream_closed`                                  | Recorded whenever the subscription stream ends or errors.                                                                                              |
| `notification.consume.reconnect.count` | Counter   | `reconnects` | `reason` = `initial` \| `token_expiry` \| `unauthenticated` \| `error` \| `stream_end`          | Recorded immediately before each (re)open of the subscription stream.                                                                                  |
| `notification.consume.connected`       | Gauge     | —            | —                                                                                               | `1` while this replica has an active subscription, `0` otherwise. A replica stuck at `0` silently goes stale until cache TTL — a good alerting signal. |
| `notification.consume.delivery_lag`    | Histogram | `ms`         | —                                                                                               | Delay between an event being published and this replica consuming it (`now - published_at`), i.e. end-to-end cross-replica propagation delay.          |

(permission-service-observability-generic-transport-metrics)=
### Generic transport metrics

These metrics are recorded for every REST and gRPC request the service handles. They are useful for general request-rate, latency, and error-rate dashboards regardless of which authorization endpoint is called.

(permission-service-observability-rest-metrics)=
#### REST metrics

The following REST metrics are recorded for every inbound HTTP request and are attributed with the matched route `path` and the HTTP `method`. The error counter additionally carries the `status_code` attribute:

| Name                   | Type      | Unit       | Description                                                               |
|------------------------|-----------|------------|---------------------------------------------------------------------------|
| `http.server.request.count`   | Counter   | `requests` | Total number of HTTP requests received.                                   |
| `http.server.request.errors`  | Counter   | `errors`   | Total number of HTTP requests that returned a 4xx or 5xx status code.     |

End-to-end HTTP request latency is not part of this set; it is reported as `http.server.request.duration` (in
seconds) described below, so the two are not duplicated.

`http.server.request.active` (`UpDownCounter`, unit `requests`, no attributes) tracks the number of HTTP requests
currently being handled: it rises when a request starts and falls when the request finishes or is abandoned early
(for example on client disconnect). Unlike the metrics above, it reflects concurrency and saturation rather than
throughput — a rising, sustained value indicates the service is handling more requests at once than it can drain,
which is a useful signal for sizing the service worker threads (`config.workerThreads`) or the database connection
pool (`database.postgres.connections`), or for alerting before request queuing turns into timeouts.

The service additionally reports the inbound HTTP metrics defined by the [OpenTelemetry HTTP semantic conventions](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/), attributed with `http.request.method`, `http.route` (when a matched path is available), `http.response.status_code` (for the duration metric), and `network.protocol.version`:

| Name                              | Type      | Unit  | Description                                                                                               |
|-----------------------------------|-----------|-------|-------------------------------------------------------------------------------------------------------------|
| `http.server.request.duration`    | Histogram | `s`   | Duration of inbound HTTP server requests, measured from call entry to the start of the response.          |
| `http.server.request.body.size`   | Histogram | `By`  | Size of the request body consumed by the handler (headers and trailers excluded).                         |
| `http.server.response.body.size`  | Histogram | `By`  | Size of the response body observed by the client (headers and trailers excluded).                         |

(permission-service-observability-grpc-metrics)=
#### gRPC metrics

The following metrics are recorded for every inbound gRPC call and are attributed with `rpc.service`, `rpc.method`, and the final `rpc.grpc.status_code`. The naming follows the [OpenTelemetry RPC semantic conventions](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-metrics/):

| Name                              | Type      | Unit       | Description                                                                                                                                |
|-----------------------------------|-----------|------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `rpc.server.request.count`        | Counter   | `requests` | Total number of gRPC server requests received.                                                                                             |
| `rpc.server.duration`             | Histogram | `ms`       | Duration of gRPC request handling.                                                                                                         |
| `rpc.server.request.error.count`  | Counter   | `requests` | Total number of gRPC requests that completed with a non-OK status (non-zero `grpc-status`, or a transport failure reported as `UNKNOWN`).  |

`rpc.server.request.active` (`UpDownCounter`, unit `requests`, no attributes) tracks the number of gRPC requests
currently being handled, rising and falling with each call exactly like `http.server.request.active`, and is
likewise a concurrency and saturation signal rather than a throughput counter.

(permission-service-observability-tracing)=
## Tracing

Every inbound request opens a server span, and the stages that run while the request is processed open child spans beneath it, so a trace shows where a request spent its time.

For each inbound REST request the service opens an `http_request` span carrying the HTTP method, the matched route, and the OpenTelemetry attributes `otel.name = "<METHOD> <ROUTE>"` and `otel.kind = "server"`. For each inbound gRPC request it opens a `grpc_request` span with `otel.name = "<path>"`, `otel.kind = "server"`, and a `trace_id` field taken from the active trace context.

Child spans cover the individual stages of request processing — policy and metadata reads and writes, the authorization check, JWT validation, userinfo lookups, token caching, and event publishing — so each stage is visible in the trace tree.

(permission-service-observability-distributed-tracing)=
### Distributed tracing

Distributed tracing **is supported**. The service configures the **W3C Trace Context** propagator as the global OpenTelemetry text-map propagator, which means:

- When an incoming request arrives (either REST or gRPC), the service extracts the parent trace context from the standard `traceparent` / `tracestate` HTTP headers and makes it the parent of the server-side span. If no valid context is present, a new root trace is started.
- Trace IDs flow unchanged across the REST and gRPC surfaces, so a single client request that spans multiple services shows up as one end-to-end trace in the collector.

Callers that initiate traces upstream (for example, an API gateway, a client SDK, or another Omniverse service) only need to propagate the `traceparent` header for their traces to stitch together with Permission Service spans.

Sampling is head-based and the decision is derived from the trace id, so it is stable for a given trace. The sampling ratio is taken from `observability.tracing.samplingRatio` (a value between `0.0` and `1.0`); `1.0` samples every trace, `0.0` samples none.

(permission-service-observability-logging)=
## Logging

Logs are written to **stdout**. The log level comes from the Helm value `observability.logging.level`, which the chart passes to the service as the `RUST_LOG` environment variable. It accepts filter directives that set the level globally or per target — for example:

- `info` — enable `info` and above for every target.
- `permission_service=debug,hyper=warn` — debug logs for the service itself, warnings only for `hyper`.

Two output formats are supported and are chosen at startup:

- **Pretty** (default) — human-readable, colorized, multi-line output for interactive and development use.
- **Structured** — single-line JSON records suitable for ingestion by log aggregators. Enabled by setting `observability.logging.structured.enabled` to `true`.

Log records produced by the third-party components bundled in the service go through the same pipeline, so they honor the configured level and output format as well.

(permission-service-observability-access-logs)=
### Access logs

In addition to the operational logs above, the service can emit **API access logs**: one structured record per business REST or gRPC call, capturing the operation, the selected request parameters, and the response status and outcome. Access logs are **disabled by default**. Enable them with `observability.logging.access.enabled` / `ACCESS_LOG_ENABLED`.

Access logs are ordinary `info` events on the tracing target `access`, so `RUST_LOG` must also allow that target for them to appear (the default `observability.logging.level` of `info` is sufficient). To keep application logs quiet while still emitting access logs, set the level to something like `error,access=info`. They use the **same formatter as every other log**: pretty when `observability.logging.structured.enabled` is `false`, and nested JSON when it is `true`.

See {ref}`API access logs <permission-service-access-logs-api-access-logs>` for the full record contract, the per-endpoint parameter tables, and the integration pattern.

(permission-service-observability-log-levels)=
### Log levels

The service uses five log levels with the following conventions:

- **`error`** — Unrecoverable or unexpected failures. Examples: failure to initialize authentication on startup, unexpected errors while verifying a bearer token (other than the token simply being rejected), failure to fetch or parse JWKS or userinfo responses, failure to refresh the service-identity access token, invalid access-token files, and malformed JWT headers or claims. These messages indicate a real problem to investigate.
- **`warn`** — Degraded but non-fatal conditions. Examples: authentication disabled via `ENABLE_AUTH=false` (Helm: `openId.enabled: false`), non-`Bearer` `Authorization` headers, non-2xx responses from the identity provider's JWKS or userinfo endpoints, missing `jwks_uri` on the discovery document, failure to connect to the Event Aggregation Service, and failures to publish policy-change events.
- **`info`** — Normal lifecycle events and operator-visible state changes. Examples: the effective service configuration at startup, "Listening HTTP on …" and "Listening gRPC on …" bind confirmations, successful JWKS pulls, assignment of system policies and system metadata, successful connection to the Event Aggregation Service, access-token file reads, and scheduled token refreshes. Incoming requests also produce info-level spans.
- **`debug`** — Verbose per-request diagnostics useful during troubleshooting. Examples: absence of an `Authorization` header, bearer tokens rejected as invalid or forbidden, number of policies matched for an action, request payloads for outgoing notification events, successful publication of policy-change events, and successful refreshes of client-credentials tokens.
- **`trace`** — Reserved for very fine-grained diagnostics. The service itself does not currently emit trace-level records, but the level is accepted and can be enabled to surface trace-level output from the bundled third-party components.

Unless a specific subsystem needs a deeper view, `info` is the recommended level for steady-state operation and `permission_service=debug` is the recommended level for focused troubleshooting of the service itself.

(permission-service-observability-configuring-observability-with-the-helm-chart)=
## Configuring Observability with the Helm Chart

All observability settings live under the top-level `observability` key in the Permission Service Helm chart values. The chart translates these values into environment variables on the deployment (`RUST_LOG`, `STRUCTURED_LOGGING`, `ACCESS_LOG_ENABLED`, `ACCESS_LOG_HEADERS`, `TRACES_ENDPOINT`, `TRACE_SAMPLING_RATIO`, `METRICS_ENDPOINT`, `SERVICE_NAME`, `SERVICE_SCOPE`).

(permission-service-observability-helm-values-reference)=
### Helm values reference

| Value                                           | Default                | Description                                                                                                                                                                                                                        |
|-------------------------------------------------|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `observability.serviceName`                     | `"permission-service"` | `service.name` resource attribute attached to all exported metrics and traces, and the instrumentation scope name of exported metrics. The instrumentation scope name of exported traces is always `permission_service`, regardless of this value. |
| `observability.serviceScope`                    | `"permission_service"` | Passed to the service as `SERVICE_SCOPE`, but currently has no effect on exported metrics or traces.                                                                                                                              |
| `observability.logging.level`                   | `"info"`               | Log level filter, passed to the service as the `RUST_LOG` environment variable. Accepts a global level or per-target directives, for example `info` or `permission_service=debug,hyper=warn`.                                      |
| `observability.logging.structured.enabled`      | `false`                | Enables single-line JSON log output. Leave disabled for human-readable pretty logs.                                                                                                                                               |
| `observability.tracing.enabled`                 | `false`                | Enables trace export. When `false`, no tracing endpoint is injected and the service runs without span export.                                                                                                                     |
| `observability.logging.access.enabled`          | `false`                | Enables API access logs (one record per business REST/gRPC call). Emitted as `info` events on target `access`, so `RUST_LOG` must allow that target. See {ref}`API access logs <permission-service-access-logs-api-access-logs>`.                                      |
| `observability.logging.access.headers`          | `{}`                   | Map of access field name to request header name; each present header is copied as a top-level field on every access record (for example `ip`, `ua`). `Authorization` is never copied. Ignored when access logs are disabled.        |
| `observability.tracing.endpoint.grpc`           | `""`                   | OTLP/gRPC endpoint that receives trace spans, for example `http://otel-collector:4317`. Only used when `observability.tracing.enabled` is `true`.                                                                                  |
| `observability.tracing.samplingRatio`           | `"1.0"`                | Head-based sampling ratio between `0.0` and `1.0`. `1.0` samples every trace; lower values proportionally reduce the exported volume.                                                                                              |
| `observability.metrics.enabled`                 | `false`                | Enables metrics export. When `false`, no metrics endpoint is injected and the service runs without metric export.                                                                                                                 |
| `observability.metrics.endpoint.grpc`           | `""`                   | OTLP/gRPC endpoint that receives metrics, for example `http://otel-collector:4317`. Only used when `observability.metrics.enabled` is `true`.                                                                                      |

(permission-service-observability-example-export-metrics-traces-and-structured-logs)=
### Example: export metrics, traces, and structured logs

The following values block points both exporters at an OpenTelemetry Collector reachable inside the cluster, raises the service log level to `debug`, and switches log output to JSON so that a log pipeline can parse it:

```yaml
observability:
  serviceName: "permission-service"
  serviceScope: "permission_service"

  logging:
    level: "permission_service=debug,info"
    structured:
      enabled: true

  tracing:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
    samplingRatio: "0.1"

  metrics:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
```

(permission-service-observability-additional-environment-variables)=
### Additional environment variables

The service also honors the [standard OpenTelemetry SDK environment variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) (for example `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_RESOURCE_ATTRIBUTES`, or `OTEL_SDK_DISABLED`). These can be injected through the chart's `extraVars` map when a deployment needs to authenticate to a managed collector or add extra resource attributes:

```yaml
extraVars:
  OTEL_EXPORTER_OTLP_HEADERS: "authorization=Bearer <token>"
  OTEL_RESOURCE_ATTRIBUTES: "deployment.environment=production"
```

(permission-service-observability-see-next)=
## See next

Continue with {ref}`Permission Web UI <permission-service-web-ui-permission-web-ui>` for the browser UI that shares the same OpenID provider and REST API as the service.
