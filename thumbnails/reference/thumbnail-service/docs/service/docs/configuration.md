(thumbnail-service-configuration-environment-configuration-reference)=
# Environment configuration reference

Both binaries are configured entirely through environment variables (every variable also has a `--kebab-case`
command-line flag). This page lists every variable the **orchestrator** and the **task-worker**
read, its default, and the Helm value that sets it. For the deployment walkthrough see
{ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>`; for the identity variables in context see
{ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>`.

(thumbnail-service-configuration-how-configuration-is-supplied)=
## How configuration is supplied

- **Fail-fast startup.** A required variable that is missing or malformed stops the process before any listener
  is bound, with an error naming the variable. Conditionally required variables (the notification settings
  when consumption is enabled) fail the same way.
- **Durations** accept strings like `250ms`, `2s`, or `1h`, or a bare integer of milliseconds, except
  `STREAM_TIMEOUT_SECS`, which is a whole number of seconds as its name says.
  **Booleans** accept `true`/`false`/`1`/`0`, case-insensitively.
- **Endpoints are plaintext.** Every gRPC dependency endpoint is reached over plaintext HTTP/2 inside the
  cluster; a value beginning `https://` is refused at startup with an explanation. Supply a scheme-less or
  `http://` address. TLS terminates at the ingress.
- **Helm mapping.** The chart injects these variables from the values listed in the *Helm value* column;
  anything not exposed as a value can be added through `thumbnailService.extraVars` or
  `thumbnailWorker.extraVars`.

(thumbnail-service-configuration-server-environment-variables)=
## Server environment variables

(thumbnail-service-configuration-listeners-and-request-handling)=
### Listeners and request handling

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `PORT` | `3000` | `service.restPort` | REST listener port. |
| `GRPC_PORT` | `0.0.0.0:50051` | `service.grpcPort` (rendered as `0.0.0.0:<port>`) | gRPC bind address, host and port together. |
| `OUTBOUND_REQUEST_TIMEOUT` | `30s` | — | Per-call deadline on the service's own unary outbound calls: the generate publish and the association-event publish. Deliberately per call rather than on the channel, because the same channel carries the long-lived consume stream. |
| `ENABLE_CORS` | `false` | — | Answer cross-origin REST requests. |
| `CORS_ALLOWED_ORIGINS` | `""` | — | Comma-separated origins permitted when CORS is enabled. Empty permits **none**; only `GET`, `POST`, `DELETE`, `OPTIONS` and the `Authorization`, `Content-Type`, `Accept` headers are ever allowed. |
| `DEVELOPMENT_MODE` | `false` | — | **Never set in a deployed environment.** Makes `GET /v1alpha/thumbnails` return fabricated data without calling storage, which skips the only authorization check the read path has. |

(thumbnail-service-configuration-dependency-endpoints)=
### Dependency endpoints

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `STORAGE_GRPC_ENDPOINT` | `localhost:50001` | `storage.endpoint` (default `storage-service:8011`) | Storage service gRPC endpoint for `Stat`, metadata, and blob deletion. |
| `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` | — | `notifications.eventConsumer.endpoint` (default `event-consumer-service:50051`) | Notification Event Consumer API. **Required** when this process is the assigned, enabled storage-event consumer. |
| `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT` | — | `notifications.eventAggregation.endpoint` (default `event-aggregation-service:50051`, injected when `notifications.eventAggregation.enabled`) | Notification Event Aggregation API where `omni.thumbnail.generate`, `omni.thumbnail.associated`, and `omni.thumbnail.disassociated` are published. Unset disables association events and makes every generate dispatch fail as retryable. |
| `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` | — | `notifications.eventConsumer.resourcePrefixes` (list, joined with `,`) | Comma-separated storage-container URL prefixes. Each entry becomes one greedy-prefix filter on StorageEventQueue and is an input to the derived queue id. **Required** for the consumer; blank entries (`a,,b`) are rejected. |

(thumbnail-service-configuration-event-consumer)=
### Event consumer

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `EVENT_CONSUMER_SERVER` | `HTTP` | — | Which role consumes StorageEventQueue: `HTTP`, `GRPC`, or `NONE`. The unified binary owns both roles, so `HTTP` and `GRPC` both enable consumption; `NONE` disables it. |
| `DISABLE_EVENT_CONSUMPTION` | `false` | injected as `true` when `notifications.eventConsumer.storageEventQueue.enabled: false` | Hard off-switch for storage-event consumption, checked before the role. |
| `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` | — | none — use `thumbnailService.extraVars` | Optional principal identity of the thumbnail writer (the task-worker). When set, `omni.storage.created` events carrying that principal are skipped. Currently inert in every environment because storage does not yet stamp the writer principal; template-shape matching carries the recursion guard. |

(thumbnail-service-configuration-stream-and-redelivery-tuning)=
### Stream and redelivery tuning

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `EVENT_PROCESSING_MAX_CONCURRENT` | `100` | `streaming.maxConcurrent` | Deliveries processed at once. A permit is reserved *before* a delivery is read, so the loop keeps observing shutdown and token expiry at saturation. |
| `STREAM_TIMEOUT_SECS` | `90` | `streaming.streamTimeoutSecs` | Per-attempt deadline for durable-queue provisioning calls. Not an idle teardown — a quiet, healthy stream is never rebuilt for silence. |
| `MAX_RECONNECT_BACKOFF_SECONDS` | `60` | `streaming.maxReconnectBackoffSecs` | Ceiling of the reconnect backoff. |
| `RECONNECT_BASE_BACKOFF_SECONDS` | `5` | — | First reconnect backoff step; doubled per attempt (≥ 1). |
| `RECONNECT_BACKOFF_EXPONENT_CAP` | `4` | — | Doublings before the backoff stops growing (`0..=16`). Backoff is jittered over the upper half of the computed interval so replicas do not retry in lock-step. |
| `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` | `4` | — | Transient retries (`UNAVAILABLE`, `DEADLINE_EXCEEDED`, connection failures) for `CreateDurableQueue` at startup before failing fast; `0` disables retries. |
| `HEALTH_WARNING_INTERVAL` | `10` | — | Reconnect attempts between repeated health warnings. |
| `TASK_CLEANUP_TIMEOUT_SECS` | `30` | — | Grace period for in-flight event tasks when a consume session ends (≥ 1). |
| `NACK_REDELIVERY_DELAY_SECS` | `60` | — | How long a retryably-failed delivery's negative acknowledgement is held before it is sent. The broker requeues immediately, so this is the spacing between attempts at the same work. Keep it far inside the broker's acknowledgement timeout (1800 s observed). |
| `NACK_DEFERRAL_CONCURRENCY_LIMIT` | `10` | — | Most negative acknowledgements held at once. Past the bound a failure is negatively acknowledged immediately. |
| `NACK_EARLY_REDELIVERY_LIMIT` | `5` | — | Consecutive early redeliveries of one delivery key before the consume session is recycled (circuit breaker). |
| `EVENT_TASK_SUBMISSION_MAX_RETRY_AGE` | `1h` | — | Parsed and logged at startup; **not enforced** by the current release. |

See {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` for how these values interact.

(thumbnail-service-configuration-storage-channel-and-retries)=
### Storage channel and retries

| Variable | Default | Purpose |
|---|---|---|
| `STORAGE_GRPC_CONNECT_TIMEOUT` | `10s` | Connect deadline for the storage channel. |
| `STORAGE_GRPC_REQUEST_TIMEOUT` | `30s` | Per-request deadline on the storage channel; also bounds the `CreateThumbnails` source `Stat` and blob deletion. |
| `STORAGE_GRPC_HTTP2_KEEPALIVE_INTERVAL` | `30s` | HTTP/2 keepalive ping interval. Keepalive notices a peer that has gone away; it does not bound a peer that answers and then goes quiet — the request timeout does. |
| `STORAGE_GRPC_KEEPALIVE_TIMEOUT` | `10s` | How long to wait for a keepalive acknowledgement. |
| `STORAGE_GRPC_TCP_KEEPALIVE` | `60s` | TCP keepalive on the storage channel. |
| `STORAGE_METADATA_MAX_RETRIES` | `3` | Attempts for storage metadata reads and writes on `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED`, `ABORTED` (≥ 1). |
| `STORAGE_METADATA_INITIAL_DELAY` | `200ms` | First retry delay, doubled per attempt. |
| `STORAGE_METADATA_MAX_DELAY` | `2s` | Retry delay ceiling. |

None of these are Helm values; set them through `thumbnailService.extraVars` if an environment needs them.

(thumbnail-service-configuration-thumbnail-processing-defaults)=
### Thumbnail processing defaults

All seven are **required** and have no default; the process refuses to start if any is missing or malformed.
The five `THUMBNAIL_*` processing values are injected whenever a request or event specification omits that field;
`THUMBNAIL_SIZES` drives fan-out; `THUMBNAIL_PATH_TEMPLATE` resolves destinations.

| Variable | Helm value | Purpose |
|---|---|---|
| `THUMBNAIL_PATH_TEMPLATE` | `thumbnailPathTemplate` | Template resolved into one exact, absolute output address per generated thumbnail. See {ref}`Supported variables <thumbnail-service-destination-path-template-supported-variables>` and {ref}`Validation at startup <thumbnail-service-destination-path-template-validation-at-startup>`. |
| `THUMBNAIL_SIZES` | `thumbnailSizes` (default `256x256,512x512`) | Comma-separated `WxH` pairs; each dimension `1..=4096`, one to ten entries, no duplicates, no blank entries. Tighter than the API's `8192` on purpose: every storage event pays for every size. |
| `THUMBNAIL_ENCODING_FORMAT` | `thumbnailConfig.encodingFormat` (default `PNG`) | One of `PNG`, `JPEG` (alias `JPG`), `GIF`, `WEBP`; case-insensitive. |
| `THUMBNAIL_TRANSFORM_TYPE` | `thumbnailConfig.transformType` (default `FIT`) | One of `SQUARE`, `SCALE_BY_WIDTH`, `SCALE_BY_HEIGHT`, `CROP`, `FIT`, `SCALE_AND_CROP`, `THUMBNAIL`, `RESIZE`. |
| `THUMBNAIL_CROP_STRATEGY` | `thumbnailConfig.cropStrategy` (default `SMART`) | One of `CENTER`, `TOP`, `BOTTOM`, `LEFT`, `RIGHT`, `TOP_LEFT`, `TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, `SMART`. Applies only to `CROP` and `SCALE_AND_CROP`. |
| `THUMBNAIL_QUALITY` | `thumbnailConfig.quality` (default `85`) | Integer `1..=100`; the default encode quality. Honored by the JPEG encoder only. |
| `THUMBNAIL_PRESERVE_ASPECT_RATIO` | `thumbnailConfig.preserveAspectRatio` (default `true`) | Explicit boolean value (`true`/`1`/`false`/`0`). |

(thumbnail-service-configuration-outbound-authentication)=
### Outbound authentication

Exactly one mode must be enabled; the process fails at startup if both or neither is. The variables are shared
by both binaries. See {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` for the
flows.

| Variable | Mode | Helm value | Purpose |
|---|---|---|---|
| `ENABLE_CLIENT_CREDENTIALS` | client credentials | `serviceIdentity.clientCredentials.enabled` | Enable the in-process OAuth 2.0 client-credentials exchange. |
| `CLIENT_ID` | client credentials | `serviceIdentity.clientCredentials.clientId` | OAuth 2.0 client (application) id. Required in this mode. |
| `CLIENT_SECRET` | client credentials | — | Client secret as a literal. Exactly one of `CLIENT_SECRET` / `CLIENT_SECRET_FILE`. |
| `CLIENT_SECRET_FILE` | client credentials | `serviceIdentity.clientCredentials.clientSecretRef.{name,key}` → `/etc/secrets/<key>` | Path to a file holding the client secret; how the chart mounts a Kubernetes `Secret`. |
| `CLIENT_SCOPE` | client credentials | `serviceIdentity.clientCredentials.clientScope` | OAuth 2.0 scope sent with the exchange, typically `<client-id>/.default`. Optional: the chart renders this variable only when set and does not require it. |
| `CLIENT_CREDENTIALS_OPENID_CONFIGURATION` | client credentials | `serviceIdentity.clientCredentials.openIdConfigurationUri` | OIDC discovery document URL; `token_endpoint` is resolved from it at startup. Required in this mode. |
| `CLIENT_CREDENTIALS_EXTRA_HEADERS` | client credentials | `serviceIdentity.clientCredentials.extraHeaders` | Optional extra HTTP headers on the token request, comma-separated `key=value` pairs. |
| `CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS` | client credentials | `serviceIdentity.clientCredentials.extraBodyParams` | Optional extra form parameters on the token request, comma-separated `key=value` pairs. |
| `ENABLE_ACCESS_TOKEN_FILE` | access-token file | `serviceIdentity.accessTokenFile.enabled` | Enable the sidecar-provisioned token file. |
| `ACCESS_TOKEN_FILE_PATH` | access-token file | `serviceIdentity.accessTokenFile.path` (default `/etc/secrets/access-token/token`) | Path of the JSON token file. Required in this mode. |
| `ACCESS_TOKEN_FILE_POLL_INTERVAL` | access-token file | `serviceIdentity.accessTokenFile.pollInterval` (default `15s`) | How often the file is re-read looking for a rotation. |

(thumbnail-service-configuration-observability-variables)=
### Observability

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `RUST_LOG` | `info` | `observability.logging.level` | Log filter: a level (`info`, `debug`), or comma-separated `<component>=<level>` pairs to set the level per component. An unparseable value falls back to `info` rather than failing startup. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | — | `observability.tracing.endpoint.grpc` | OTLP/gRPC collector for trace spans. Injected only when `observability.tracing.enabled` is true and the endpoint is set; otherwise the chart injects `OTEL_TRACES_EXPORTER=none`. |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | — | `observability.metrics.endpoint.grpc` | OTLP/gRPC collector for metrics, independent of the tracing endpoint. Injected only when `observability.metrics.enabled` is true and the endpoint is set; otherwise `OTEL_METRICS_EXPORTER=none`. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | `parentbased_traceidratio`, `0.1` | (fixed), `observability.tracing.samplingRatio` | Head-based sampling; child spans — including the worker's conversion spans — inherit the producer's decision. |
| `OTEL_RESOURCE_ATTRIBUTES` | unset; `service.instance.id=<pod name>` once export is on | `observability.resourceAttributes` (appended) | Resource attributes on every span and metric. Injected, together with `POD_NAME`, only when tracing or metrics export is active; with both endpoints empty (the chart default) neither variable is set. |
| `OTEL_LOGS_EXPORTER` | `console` | injected unconditionally | Logs go to stdout as JSON for the cluster's log collector. |
| `OTEL_SDK_DISABLED` | — | — | Full kill switch: disables traces and metrics **and** suppresses stdout logging. To disable OTLP export only, use the per-signal `observability.tracing.enabled` / `observability.metrics.enabled` flags instead. |

Logs are always structured JSON; there is no plain-text toggle, and no logs-only way to suppress them short of
`OTEL_SDK_DISABLED`, which also disables traces and metrics. See
{ref}`Configuring Observability with the Helm Chart <thumbnail-service-observability-configuring-observability-with-the-helm-chart>`.

(thumbnail-service-configuration-worker-environment-variables)=
## Worker environment variables

The task-worker reads **no** `THUMBNAIL_*` specification settings and no path template: every processing field
arrives on the `omni.thumbnail.generate` message, so a local default could only drift from what the publisher
resolved. It configures where to talk, how hard to work, and how much decode it will accept. The outbound
authentication variables above apply unchanged.

| Variable | Default | Helm value | Purpose |
|---|---|---|---|
| `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` | — (**required**) | `notifications.eventConsumer.endpoint` | Event Consumer API hosting WorkerEventQueue. |
| `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` | — (**required**) | `notifications.eventConsumer.resourcePrefixes` | Same prefix set as the orchestrator; filters `omni.thumbnail.generate` on the **source** address and derives `thumbnail-service-worker-<sha256>`. An empty set binds a queue that receives nothing, so it is refused. |
| `STORAGE_GRPC_ENDPOINT` | — (**required**) | `storage.endpoint` | Storage service for source reads and thumbnail writes. |
| `THUMBNAIL_SERVICE_GRPC_ENDPOINT` | — (**required**) | derived: `<service name>:<service.grpcPort>` — not a values key | Where the worker calls `AssociateThumbnail`. Connected lazily so the two Deployments cannot deadlock at startup. |
| `MAX_CONCURRENT_CONVERSIONS` | `2` | `workerRuntime.maxConcurrentConversions` | Conversions in flight per replica (≥ 1). Each additional slot costs `MAX_TRANSFORM_PEAK_BYTES` of memory ceiling. |
| `REDELIVERY_RECYCLE_INTERVAL` | `60s` | `workerRuntime.redeliveryRecycleInterval` | Delay before a retryably-failed delivery's negative acknowledgement is sent; the spacing between attempts at the same work. Values below `5s` are refused. |
| `NACK_DEFERRAL_CONCURRENCY_LIMIT` | `10` | — | Most negative acknowledgements held at once (≥ 1). |
| `NACK_EARLY_REDELIVERY_LIMIT` | `5` | — | Consecutive early redeliveries of one delivery key before the stream is recycled (≥ 1). |
| `OUTBOUND_REQUEST_TIMEOUT` | `30s` | `workerRuntime.outboundRequestTimeout` | Deadline on every outbound call — storage read, storage write, and `AssociateThumbnail` — so a hung dependency releases its conversion slot. |
| `SHUTDOWN_DRAIN_TIMEOUT` | `30s` | `workerRuntime.shutdownDrainTimeout` | Grace period for in-flight conversions after `SIGTERM`; must stay below `thumbnailWorker.terminationGracePeriodSeconds` (default `45`). Unfinished work stays unacknowledged and is redelivered. |
| `STREAM_TIMEOUT_SECS` | `90` | `streaming.streamTimeoutSecs` | Whole seconds to wait for the consume stream to be accepted; also the provisioning deadline. |
| `RECONNECT_BASE_BACKOFF_SECONDS` | `5` | — | First reconnect backoff step (≥ 1). |
| `MAX_RECONNECT_BACKOFF_SECONDS` | `60` | `streaming.maxReconnectBackoffSecs` | Reconnect backoff ceiling (≥ 1). |
| `MAX_SOURCE_BYTES` | `67108864` (64 MiB) | `workerRuntime.maxSourceBytes` | Largest source the worker downloads and decodes. |
| `MAX_DECODED_PIXELS` | `64000000` (64 MP) | `workerRuntime.maxDecodedPixels` | Largest decoded image the worker allocates for. |
| `MAX_TRANSFORM_PEAK_BYTES` | `1073741824` (1 GiB) | `workerRuntime.maxTransformPeakBytes` | Largest peak allocation one resample may reach; the figure the pod's memory limit is sized against. See {ref}`Conversion bounds <thumbnail-service-generation-pipeline-conversion-bounds>`. |
| `HEALTH_PORT` | `8080` | `workerRuntime.healthPort` | Probe and `/metrics` listener; the worker's only port. |
| `DEVELOPMENT_MODE` | `false` | — | Parsed for parity with the orchestrator; the worker does not act on it. |

(thumbnail-service-configuration-helm-values-to-environment-variables)=
## Helm values to environment variables

The table below is the reverse index: which Deployment each root value reaches.

| Helm value | Environment variable | orchestrator | worker |
|---|---|---|---|
| `storage.endpoint` | `STORAGE_GRPC_ENDPOINT` | yes | yes |
| `notifications.eventConsumer.endpoint` | `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` | when consumer enabled | yes |
| `notifications.eventConsumer.resourcePrefixes` | `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` | when consumer enabled | yes |
| `notifications.eventConsumer.storageEventQueue.enabled: false` | `DISABLE_EVENT_CONSUMPTION=true` | yes | — |
| `notifications.eventAggregation.endpoint` | `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT` | when enabled | — |
| `streaming.maxConcurrent` | `EVENT_PROCESSING_MAX_CONCURRENT` | yes | — |
| `streaming.streamTimeoutSecs` | `STREAM_TIMEOUT_SECS` | yes | yes |
| `streaming.maxReconnectBackoffSecs` | `MAX_RECONNECT_BACKOFF_SECONDS` | yes | yes |
| `thumbnailPathTemplate` | `THUMBNAIL_PATH_TEMPLATE` | yes | — |
| `thumbnailSizes` | `THUMBNAIL_SIZES` | yes | — |
| `thumbnailConfig.*` | `THUMBNAIL_ENCODING_FORMAT`, `THUMBNAIL_TRANSFORM_TYPE`, `THUMBNAIL_CROP_STRATEGY`, `THUMBNAIL_QUALITY`, `THUMBNAIL_PRESERVE_ASPECT_RATIO` | yes | — |
| `workerRuntime.*` | `HEALTH_PORT`, `MAX_CONCURRENT_CONVERSIONS`, `REDELIVERY_RECYCLE_INTERVAL`, `OUTBOUND_REQUEST_TIMEOUT`, `SHUTDOWN_DRAIN_TIMEOUT`, `MAX_SOURCE_BYTES`, `MAX_DECODED_PIXELS`, `MAX_TRANSFORM_PEAK_BYTES` | — | yes |
| `service.grpcPort` (with the Service name) | `THUMBNAIL_SERVICE_GRPC_ENDPOINT` | — | yes |
| `serviceIdentity.*` | the outbound authentication variables | yes | yes |
| `observability.logging.level` | `RUST_LOG` | yes | yes |
| `observability.tracing.*`, `observability.metrics.*`, `observability.resourceAttributes` | `OTEL_*` | yes | yes |
| `thumbnailService.extraVars` / `thumbnailWorker.extraVars` | any | yes | yes |

`thumbnailWorker.enabled` (default `true`) is a Deployment-level toggle, not an environment variable, so it has
no row above: setting it `false` skips rendering the worker Deployment (and its HPA, ServiceAccount, and
Secret) entirely rather than changing anything either binary reads. See
{ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` for the
orchestrator-only install path.

(thumbnail-service-configuration-see-also)=
## See also

- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — the Helm values these variables are rendered from, and the render-time checks.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — the two outbound authentication modes in detail.
- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>` — what `THUMBNAIL_PATH_TEMPLATE` accepts and how it is validated.
- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — how the stream and redelivery settings behave at runtime.
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — what the worker's bounds and concurrency settings control.
