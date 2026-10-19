(userinfo-service-observability-observability)=
# Observability

User Info Service produces three kinds of signal an operator can use to monitor it and diagnose
problems: **structured logs**, **distributed traces**, and **metrics**. Logging is always on;
tracing and metrics are off by default and are turned on by pointing the service at an
OpenTelemetry collector. This page describes each signal, what it contains, how trace context
flows through a request, and how to configure the three signals in the Helm chart.

Both roles emit all three signals. Readers report on the requests they serve; the writer reports
on directory synchronization and its calls to the identity provider. For the readiness and health
probes referenced below, see {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`; for how the two
roles relate, see {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`.

(userinfo-service-observability-logging)=
## Logging

The service writes logs to standard output, so a Kubernetes log collector picks them up without
extra configuration. Two aspects are configurable: the log level and the output format.

- **Level.** The log level accepts a single level such as `info`, or a comma-separated list of
  `target=level` directives that set the level per log target. The service's own records use the
  log target `microsoft_graph_userinfo_service`, so `info,microsoft_graph_userinfo_service=debug`
  raises verbosity for the service's own logs while keeping the rest at `info`. The default is
  `info`.
- **Format.** By default logs are human-readable text, which is convenient when reading them
  directly. Enabling structured logging emits one JSON object per line instead, suitable for
  ingestion by a log aggregator such as Loki or Elasticsearch.

Log records emitted while a request or synchronization cycle is being handled carry the
identifiers of the active trace, so a log line can be correlated with the trace it belongs to when
tracing is enabled.

Separately from these operational logs, the service can emit a structured **access log** — a record
of REST and gRPC service access covering both processed business calls and requests rejected before
the requested operation runs (authentication failures, unknown routes and methods, malformed
requests), for audit and request-level troubleshooting. Access logging is off by default and is described in
{ref}`API Access Logs <userinfo-service-access-logs-api-access-logs>`.

(userinfo-service-observability-tracing)=
## Distributed Tracing

When tracing is enabled, the service exports spans over OTLP/gRPC to the collector endpoint an
operator configures. Each incoming REST or gRPC request becomes a server span, and the outgoing
calls made while serving it — to Microsoft Graph and, when authorization is enabled, to the
Permission Service — become child spans, so a single trace shows the whole path of a request.

(userinfo-service-observability-trace-propagation)=
### Trace Context Propagation

The service participates in a caller's existing trace using the W3C Trace Context standard. It
reads the incoming trace context from the `traceparent` request header (REST) or the equivalent
request metadata (gRPC), continues that trace rather than starting a new one, and injects the
current trace context into the requests it makes to Microsoft Graph. Graph calls additionally
carry a client request identifier derived from the active trace, so a single request can be
followed from the caller, through the service, to the identity provider.

(userinfo-service-observability-metrics)=
## Metrics

When metrics are enabled, the service exports them over OTLP/gRPC to the configured collector
endpoint on a periodic interval. Metrics are pushed to the collector; the service does not expose
a scrape endpoint of its own. Each metric carries a set of attributes (labels) that partition its
values, and each falls into one of three OpenTelemetry instrument types:

- **Counter** — a monotonically increasing total, reset only when the process restarts. Use rates
  derived from a counter (per second, per minute) rather than its raw value.
- **Histogram** — a distribution of recorded values, used here for durations in seconds. Query it
  for percentiles (p50, p95, p99), averages, and counts.
- **Gauge** — a current value that can rise and fall. The snapshot sizes are reported when they
  change; the snapshot age is an *observable* gauge, sampled by the exporter at collection time.

The subsections below list every metric, grouped by the part of the service it measures. Which
role emits a given metric depends on what that role does: readers emit the request, authorization,
and snapshot metrics while serving; the writer emits the synchronization and commit metrics; and
the Microsoft Graph metrics come from whichever role calls Graph — the writer during
synchronization, or a reader in live mode.

(userinfo-service-observability-metrics-requests)=
### Request Metrics

Request metrics follow the RED pattern (rate, errors, duration) and are the basis for
service-level dashboards and alerts. REST and gRPC have parallel instruments.

| Metric | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `http.server.request.duration` | Histogram | s | `method`, `route`, `status` | Duration of REST request handling. |
| `http.server.request.count` | Counter | — | `method`, `route`, `status` | Total REST requests received. |
| `http.server.request.error.count` | Counter | — | `method`, `route`, `status` | REST requests that resulted in a 4xx or 5xx status. |
| `rpc.server.duration` | Histogram | s | `rpc.service`, `rpc.method`, `rpc.grpc.status_code` | Duration of gRPC request handling. |
| `rpc.server.request.count` | Counter | — | `rpc.service`, `rpc.method`, `rpc.grpc.status_code` | Total gRPC requests received. |
| `rpc.server.request.error.count` | Counter | — | `rpc.service`, `rpc.method`, `rpc.grpc.status_code` | gRPC requests that resulted in a non-OK status code. |

(userinfo-service-observability-metrics-graph)=
### Microsoft Graph Client Metrics

These measure the outgoing calls the service makes to Microsoft Graph, including retries against
the identity provider's rate limits.

| Metric | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `graph.client.request.duration` | Histogram | s | `operation`, `status` | Duration of outgoing Microsoft Graph API calls. |
| `graph.client.request.count` | Counter | — | `operation`, `status` | Total Graph API calls, including retries. |
| `graph.client.retry.count` | Counter | — | `operation`, `reason` | Graph call retries, labeled with the reason for the retry. |

(userinfo-service-observability-metrics-authorization)=
### Authorization Metrics

These are emitted only when Permission Service integration is enabled. They cover the outgoing
authorization calls and the in-memory decision cache described in
{ref}`Authorization with the Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`.

| Metric | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `permission.client.request.duration` | Histogram | s | `status` | Duration of outgoing Permission Service calls. |
| `permission.client.request.count` | Counter | — | `status` | Total Permission Service calls. |
| `permission.cache.hit_count` | Counter | — | — | Permission decision cache hits. |
| `permission.cache.miss_count` | Counter | — | — | Permission decision cache misses. |

(userinfo-service-observability-metrics-synchronization)=
### Directory Synchronization Metrics

These describe the writer's synchronization cycles and the freshness of the directory each
instance holds. The snapshot age is the primary freshness signal — see
{ref}`Data Freshness and Eventual Consistency <userinfo-service-directory-synchronization-data-freshness>`.

| Metric | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `directory.sync.duration` | Histogram | s | `outcome` | Duration of each synchronization cycle. |
| `directory.sync.count` | Counter | — | `outcome` | Total synchronization cycles completed. |
| `directory.snapshot.users` | Gauge | — | — | Number of users in the current in-memory snapshot. |
| `directory.snapshot.groups` | Gauge | — | — | Number of groups in the current in-memory snapshot. |
| `directory.snapshot.memberships` | Gauge | — | — | Number of membership edges in the current snapshot. |
| `directory.snapshot.age_seconds` | Observable gauge | s | — | Seconds since the last successful synchronization cycle completed. |

(userinfo-service-observability-metrics-persistence)=
### PostgreSQL Persistence Metrics

These measure the shared store operations: loading the snapshot into memory at startup, and (for
the writer) committing it after a synchronization cycle.

| Metric | Type | Unit | Attributes | Description |
|---|---|---|---|---|
| `snapshot.postgres.load.duration` | Histogram | s | — | Duration of loading the snapshot from PostgreSQL at startup. |
| `snapshot.postgres.commit.duration` | Histogram | s | — | Duration of committing the snapshot after a synchronization cycle. |
| `snapshot.postgres.pool.active_connections` | Observable gauge | — | — | Number of active connections in the instance's database pool. |

(userinfo-service-observability-header-forwarding)=
## Request Header Forwarding

When the service handles a REST or gRPC request and, while serving it, calls another Omniverse
service — currently the Permission Service, when
{ref}`authorization <userinfo-service-authorization-authorization-with-the-permission-service>` is
enabled — it can forward selected headers from the incoming request onto that outgoing call. An
operator lists the header names to forward; the service reads that list at startup and, for every
request it serves, copies each listed header that is present onto the calls it makes downstream.
This carries request-scoped context — such as a gateway correlation identifier — through the call
chain so a single request can be followed across services.

Header names are matched case-insensitively. Forwarding a header has no effect on the authorization
decision itself, which is determined by the caller's token, the action, and the resource; the
service's decision cache is unaffected. The list is empty by default, so no headers are forwarded
unless configured. This is distinct from trace-context propagation, which the service always
performs when tracing is enabled.

(userinfo-service-observability-readiness)=
## Readiness and Health Signals

Alongside the exported telemetry, the service exposes two HTTP probes on the REST port that are
worth monitoring directly. A liveness probe at `/healthz` reports that the process is running; a
readiness probe at `/readyz` returns a 2xx status once the instance reports ready. Both roles
serve these probes, but readiness means something different for each:

- **Reader.** A reader reports ready as soon as it has started and loaded whatever the shared store
  holds. That can be a partial directory while the writer's first synchronization is still in
  progress, or an empty directory when the writer has not yet saved anything; the reader serves
  it until a later reload picks up more data. A reader in live mode is ready immediately.
- **Writer.** The writer reports ready once it holds a complete directory — at startup when the
  store already holds one from an earlier synchronization, otherwise after its first full
  synchronization completes.

Because a ready reader may still be serving an empty or partial directory, reader readiness alone
does not show that data has arrived. To confirm that it has, check that the writer reports ready,
and that the readers' snapshot size metrics (`directory.snapshot.users`,
`directory.snapshot.groups`) show the expected counts; each reader also logs how many users and
groups it loaded and whether the loaded directory is complete. See
{ref}`Memory and CPU Sizing <userinfo-service-scaling-sizing>` for how long the writer's first
synchronization can take on a large directory.

(userinfo-service-observability-configuration)=
## Configuration

All three signals are configured through the Helm chart's `observability` section. The defaults
enable text logging at `info` and leave tracing and metrics off.

| Setting | Helm value (`observability.*`) | Default | Description |
|---|---|---|---|
| Log level | `logging.level` | `info` | Log level or comma-separated `target=level` directives; the service's own log target is `microsoft_graph_userinfo_service` (e.g. `info,microsoft_graph_userinfo_service=debug`). |
| Structured logs | `logging.structured.enabled` | `false` | Emit logs as one JSON object per line instead of human-readable text. |
| Enable access logs | `logging.access.enabled` | `false` | Emit access records for REST/gRPC service access, including requests rejected before the requested operation runs. See {ref}`API Access Logs <userinfo-service-access-logs-api-access-logs>`. |
| Access-log header map | `logging.access.headers` | `{}` | Request headers copied onto each access record as top-level fields. |
| Forwarded headers | `headers.forward` | `[]` | Incoming request headers forwarded to other Omniverse services (currently the Permission Service). |
| Enable tracing | `tracing.enabled` | `false` | Export traces over OTLP/gRPC. |
| Trace endpoint | `tracing.endpoint.grpc` | `""` | OTLP/gRPC collector address (e.g. `http://collector:4317`); required when tracing is enabled. |
| Enable metrics | `metrics.enabled` | `false` | Export metrics over OTLP/gRPC. |
| Metrics endpoint | `metrics.endpoint.grpc` | `""` | OTLP/gRPC collector address (e.g. `http://collector:4317`); required when metrics are enabled. |
| Service name | `serviceName` | `microsoft-graph-userinfo-service` | Service name reported on exported traces and metrics. |

The endpoint values are collector addresses in `host:port` form and are used verbatim, with no
URL path appended, because OTLP/gRPC routes by service method rather than by path. The default
network port for OTLP/gRPC is `4317`. Set the corresponding endpoint whenever you enable tracing
or metrics; leaving it empty while the signal is enabled leaves the exporter without a
destination.

These settings render as environment variables in the reader and writer pod specifications, so
changing any of them and upgrading the release rolls the affected pods automatically. Each value
takes effect when the pod starts.

(userinfo-service-observability-helm)=
### Configuring Observability in the Helm Chart

Add the `observability` section to `userinfo-values.yaml`. The example below emits structured logs
and exports both traces and metrics to an in-cluster collector:

```yaml
observability:
  logging:
    level: "info"
    structured:
      enabled: true
  tracing:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
  metrics:
    enabled: true
    endpoint:
      grpc: "http://otel-collector.observability.svc.cluster.local:4317"
  serviceName: "microsoft-graph-userinfo-service"
```

Install or upgrade the release with the same commands used for the synchronized directory
deployment in {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-observability-references)=
## References

- [OpenTelemetry Protocol (OTLP) specification][otlp] — the export protocol used for traces and
  metrics; its OTLP/gRPC default network port is `4317`.
- [W3C Trace Context][tracecontext] — the standard for the `traceparent` header the service reads
  and propagates.

[otlp]: https://opentelemetry.io/docs/specs/otlp/
[tracecontext]: https://www.w3.org/TR/trace-context/

(userinfo-service-observability-see-next)=
## See next

- {ref}`Troubleshooting and Error Behavior <userinfo-service-troubleshooting-troubleshooting-and-error-behavior>`
