# Thumbnail Service Helm Chart

Deploys the thumbnail pipeline's two components as one Helm release: the
orchestrator (a single `thumbnail-service` container exposing REST on port
3000 and gRPC on port 50051) and the task-worker (a separate `thumbnail-worker`
Deployment that consumes generation work and converts it — see **Task-worker**
below). `helm/thumbnail-task-worker` was folded into this chart; see
`openspec/changes/archive/2026-08-13-fold-task-worker-into-service-chart` for why.

Values are split into three groups (no `common:` wrapper):

| Group | Scope |
|-------|--------|
| Root (unwrapped) | Runtime/application configuration and shared deployment settings: dependency endpoints (`storage.endpoint`, `notifications.eventConsumer.endpoint`, `notifications.eventAggregation.endpoint`), queue configuration under `notifications.eventConsumer`, thumbnail policy, worker runtime variables, image metadata, pod/security settings, identity, ingress, and Service |
| `thumbnailService:` | Orchestrator workload deployment only: repository, names, replicas/resources, HPA, probes, extra env |
| `thumbnailWorker:` | Worker workload deployment only: enabled, repository, names, replicas/resources, HPA, probes, grace period, extra env |

Both components share one `serviceIdentity` values block but still
render separate ServiceAccount objects (the worker's
ServiceAccount name is fixed for an Azure AD federated-credential binding
outside this repo). Client-credentials mode mounts an existing Secret via
`clientSecretRef.name`; the chart does not create Secrets.

## Install

```bash
# In-process client-credentials mode (suitable for any cluster)
helm install thumbnail-service . \
  --set thumbnailService.image.repository=your-registry/thumbnail-service \
  --set thumbnailSizes="256x256" \
  --set storage.endpoint=storage-service:8011 \
  --set serviceIdentity.clientCredentials.enabled=true \
  --set serviceIdentity.clientCredentials.clientId=<azure-app-id> \
  --set serviceIdentity.clientCredentials.openIdConfigurationUri=<https://.../.well-known/openid-configuration> \
  --set serviceIdentity.clientCredentials.clientSecretRef.name=<existing-secret>

# Sidecar-driven access-token-file mode (Azure Workload Identity clusters)
# Operator must populate `serviceIdentity.accessTokenFile.provisionerSidecar.container`
# in their environment values file with the `client-credentials-sidecar` image
# and per-service OAUTH2_* envs.
helm install thumbnail-service . \
  --set thumbnailService.image.repository=your-registry/thumbnail-service \
  --set thumbnailSizes="256x256" \
  --set storage.endpoint=storage-service:8011 \
  --set serviceIdentity.clientCredentials.enabled=false \
  --set serviceIdentity.accessTokenFile.enabled=true \
  -f environments/<env>/thumbnail-service/values.yaml
```

Both examples above configure root runtime values plus the orchestrator repository. A real
install also needs `thumbnailWorker.image.repository` — see **Task-worker**
below. Storage is always `storage.endpoint` (not a per-worker override). Both durable
queue ids (StorageEventQueue and WorkerEventQueue) are hashed from
`notifications.eventConsumer.resourcePrefixes`, each by its own binary. The
worker associates against this release's in-cluster Service; there is no
`thumbnailServiceGrpcEndpoint` values key.

## Authentication modes

Both Deployments authenticate outbound calls (storage, event-consumer,
event-aggregation) with one of two mutually exclusive flows, configured once
at the chart root:

| Mode | Values | Token source |
|------|--------|--------------|
| `clientCredentials` | `serviceIdentity.clientCredentials.enabled=true` | The service exchanges `CLIENT_ID` + `CLIENT_SECRET_FILE` for an access token at startup and refreshes it itself (no sidecar). |
| `accessTokenFile` | `serviceIdentity.accessTokenFile.enabled=true` and `provisionerSidecar.container` populated | A `client-credentials-sidecar` container provisions the token and writes it to a shared `emptyDir` volume; the main containers consume it via `ACCESS_TOKEN_FILE_PATH`. |

The chart **does** enforce mutual exclusivity at template time
(`service.validateServiceIdentity`), and the binary also fails fast at
startup if both or neither mode is enabled.

### Migration from legacy env vars

The dual-mode work renamed `thumbnail-service`'s env vars to match the
canonical names used by `client-credentials-sidecar` and `permission-service`:

| Old (removed) | New (canonical) |
|---------------|-----------------|
| `SERVICE_ACCOUNT_CLIENT_ID` | `CLIENT_ID` |
| `SERVICE_ACCOUNT_CLIENT_SECRET` | `CLIENT_SECRET` |
| `SERVICE_ACCOUNT_CLIENT_SECRET_FILE` | `CLIENT_SECRET_FILE` |
| `TENANT_ID`, `TOKEN_URI` | (removed) — use `CLIENT_CREDENTIALS_OPENID_CONFIGURATION` instead, which resolves the token endpoint via OIDC discovery. |
| `TOKEN_REFRESH_INTERVAL_SECONDS`, `TOKEN_EXPIRY_BUFFER_SECONDS`, `TOKEN_ACQUISITION_TIMEOUT_SECONDS`, `TOKEN_WAIT_POLL_MS` | (removed) — refresh interval and expiry buffer are now constants inside `thumbnail-auth`. |

There is **no fallback**: charts and `.env` files that still set the old
names will fail clap parsing at startup.

## Values migration (post-!182 → current)

Breaking change from the post-!182 shape (flat orchestrator + `worker:`).
Migrate environment values files before upgrading this chart version:

| Old (post-!182) | New |
|-----------------|-----|
| Top-level orchestrator keys (`resources`, `replicaCount`, probes, `app.*`, …) | `thumbnailService.*` |
| `worker.*` | `thumbnailWorker.*` (except keys that move to root below) |
| `podSecurityContext`, `securityContext` | unchanged at root |
| Per-component image tag / pull policy / pull secrets | root `image.{tag,pullPolicy,pullSecrets}`; only each component's `image.repository` remains namespaced |
| `thumbnailService.service`, `.httpProxy`, `.smokeTest` | root `service`, `httpProxy`, `smokeTest` |
| Per-component `serviceAccount.{create,automount,annotations}` | root `serviceAccount.{create,automount,annotations}`; each component keeps only `serviceAccount.name`, because two independently named objects still render |
| Per-component `podLabels` / `podAnnotations` | root `podLabels` / `podAnnotations`, applied to both pod templates |
| `serviceIdentity` (and former `worker.serviceIdentity` / worker secret shape) | root `serviceIdentity` — one values block; two ServiceAccounts still rendered. OAuth client secret is an existing Secret named by `clientSecretRef.name`; the chart does not create Secrets |
| `observability.logging.structured.enabled` / `worker.observability.logging.structured.enabled` | removed — JSON logging is now always on; the toggle has been deleted |
| `app.storage.endpoint` / `worker.app.storageGrpcEndpoint` | root `storage.endpoint` |
| `app.streaming.*` (and any manual worker stream env overrides) | root `streaming.*` (`maxConcurrent`, `streamTimeoutSecs`, `maxReconnectBackoffSecs`) |
| `app.eventConsumer.endpoint` / `worker.app.eventConsumer.endpoint` / `notifications.endpoint` | `notifications.eventConsumer.endpoint` — one shared consumer endpoint |
| `app.eventConsumer.*` / `worker.app.eventConsumer.*` / `notifications.resourcePrefixes` | `notifications.eventConsumer.resourcePrefixes`, `.storageEventQueue.enabled`. Both the orchestrator and the worker hash their own queue's id from `resourcePrefixes` at startup; neither is a values key |
| `app.eventAggregation.*` / `thumbnailService.app.eventAggregation` / root `eventAggregation` | `notifications.eventAggregation.{enabled,endpoint}` |
| `app.thumbnailPathTemplate`, `.thumbnailSizes`, `.thumbnailConfig` | root `thumbnailPathTemplate`, `thumbnailSizes`, `thumbnailConfig` |
| Worker `app.*` runtime variables | root `workerRuntime.*`; `THUMBNAIL_SERVICE_GRPC_ENDPOINT` is derived from this release's Service (`service.nameOverride`:`service.grpcPort`) |
| `app.generatedThumbnailPrincipalIdentity` | **removed** — Rust still accepts `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` via `thumbnailService.extraVars`; chart support returns once storage propagates writer principal ([ADR 0001](../../docs/adr/0001-thumbnail-storage.md)) |
| Per-component log level (`thumbnailService.observability` / `thumbnailWorker.app.logLevel`) | root `observability.logging.level` (`RUST_LOG` on both) |
| `nodeSelector` / `tolerations` / `affinity` / `volumes` / `volumeMounts` | root keys, applied to both Deployments |

Worker decode parallelism is `workerRuntime.maxConcurrentConversions`
(not under the deployment-only `thumbnailWorker` block).

## Key Configuration

| Parameter | Description | Default |
|-----------|-------------|---------|
| `storage.endpoint` | Storage gRPC endpoint for both components | `storage-service:8011` |
| `notifications.eventConsumer.endpoint` | Notifications event-consumer gRPC endpoint for both StorageEventQueue and WorkerEventQueue (`NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT`). Required when either consumer is enabled. | `event-consumer-service:50051` |
| `notifications.eventAggregation.endpoint` | Event-aggregation gRPC endpoint (`NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT`). Injected only when `notifications.eventAggregation.enabled` is true **and** this endpoint is non-empty; enabling aggregation with an empty endpoint renders no variable and generate publishing then fails retryably. | `event-aggregation-service:50051` |
| `notifications.eventConsumer.resourcePrefixes` | List of storage URL prefixes for both consumers' queue filters (joined into `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`; one greedy-prefix filter per entry). Required (non-empty) when either consumer is enabled. Also the input to derived durable queue ids. | `[]` |
| `streaming.maxConcurrent` | Notifications stream concurrency (StorageEventQueue prefetch) | `100` |
| `streaming.streamTimeoutSecs` | Stream helper timeout (`STREAM_TIMEOUT_SECS`) for both consumers | `90` |
| `streaming.maxReconnectBackoffSecs` | Max reconnect backoff (`MAX_RECONNECT_BACKOFF_SECONDS`) for both consumers | `60` |
| `image.tag` | Container image tag for both Deployments | Chart appVersion |
| `image.pullPolicy` | Pull policy for both component images | `IfNotPresent` |
| `image.pullSecrets` | Registry pull secrets applied to both pods | `[]` |
| `thumbnailService.image.repository` | Orchestrator container image | Required |
| `thumbnailWorker.image.repository` | Worker container image | Required |
| `thumbnailService.replicaCount` | Orchestrator pod replicas | `1` |
| `thumbnailService.resources` | CPU/memory for the `thumbnail-service` container (not the token sidecar). Sized for API + StorageEventQueue filtering, not image decode. | `250m`/`64Mi` request, `1`/`256Mi` limit |
| `thumbnailSizes` | Comma-separated `WxH` thumbnail dimensions to generate per source asset. Must be non-empty/valid. | `256x256,512x512` |
| `notifications.eventConsumer.storageEventQueue.enabled` | Enable event consumption (StorageEventQueue) | `true` |
| `thumbnailPathTemplate` | Template resolved into one exact, absolute output address per generated thumbnail. | Required, `""` |
| `serviceIdentity.clientCredentials.enabled` | Enable in-process client-credentials flow | `false` |
| `serviceIdentity.clientCredentials.clientId` | OAuth2 client (application) ID | Required when `enabled=true` |
| `serviceIdentity.clientCredentials.clientScope` | OAuth2 scope. Optional: rendered as `CLIENT_SCOPE` only when set, and not required by the chart's render-time validation. | `""` |
| `serviceIdentity.clientCredentials.openIdConfigurationUri` | OpenID Connect discovery endpoint | Required when `enabled=true` |
| `serviceIdentity.clientCredentials.clientSecretRef.name` | Existing Kubernetes Secret holding the client secret | Required when `enabled=true` |
| `serviceIdentity.accessTokenFile.enabled` | Enable sidecar-driven file-based auth | `false` |
| `serviceIdentity.accessTokenFile.path` | Path the main containers read for the Bearer token | `/etc/secrets/access-token/token` |
| `serviceIdentity.accessTokenFile.pollInterval` | Re-stat interval (humantime) | `15s` |
| `serviceIdentity.accessTokenFile.provisionerSidecar.container` | Operator-supplied sidecar spec (image / env / volumeMounts) | `{}` |
| `observability.logging.level` | Injected as `RUST_LOG` on both components | `info` |
| `observability.tracing.enabled` | Export trace spans. Takes effect only when `observability.tracing.endpoint.grpc` is also set; otherwise injects `OTEL_TRACES_EXPORTER=none`. | `true` |
| `observability.tracing.endpoint.grpc` | OTLP/gRPC collector endpoint for trace spans. Injected as the per-signal `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`. | `""` |
| `observability.tracing.samplingRatio` | Sampling ratio for `parentbased_traceidratio` sampling (0.0–1.0) | `0.1` |
| `observability.metrics.enabled` | Export metrics. Takes effect only when `observability.metrics.endpoint.grpc` is also set; otherwise injects `OTEL_METRICS_EXPORTER=none`. | `true` |
| `observability.metrics.endpoint.grpc` | OTLP/gRPC collector endpoint for metrics. Injected as the per-signal `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`, independent of the tracing endpoint. | `""` |
| `observability.resourceAttributes` | Additional key=value pairs appended to `OTEL_RESOURCE_ATTRIBUTES` | `""` |

Note: `observability.logging.structured.enabled` / `STRUCTURED_LOGGING` do not exist. JSON log output is unconditional.
The pre-`tracing`/`metrics` `observability.otel.*` block is rejected at render time with a migration message rather
than silently ignored — see [Observability](#observability).

`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` is **not** a chart value. The Rust
binaries still support the env var; inject via `thumbnailService.extraVars` if
needed before chart support returns (see [ADR 0001](../../docs/adr/0001-thumbnail-storage.md)).

`thumbnailService.resources` is the `thumbnail-service` container only (API +
StorageEventQueue filtering, not image decode). Defaults were measured in
sapi-rc during a ~500 MB/s CREATE ingest (~146m CPU, ~11Mi RSS). The
token-provisioner sidecar carries no default resources of its own; both HPAs
use the `ContainerResource` metric type targeting the main container by name,
so the sidecar's resources (or lack of them) don't affect autoscaling. Worker
decode sizing is under **Task-worker** below.

## Durable queue self-provisioning

The assigned event consumer always creates its own durable notifications queue on startup
(ensure-exists) instead of requiring operators to pre-create it with
`scripts-notifications/create-durable-queue.sh`. This is not optional: when
`notifications.eventConsumer.storageEventQueue.enabled: true`, the shared
`notifications.eventConsumer.endpoint` and a non-empty `notifications.eventConsumer.resourcePrefixes` list are required.
Durable queue ids are derived from that prefix set.

### Required configuration (Kubernetes)

Set the shared consumer endpoint and source-prefix filters under
root `notifications` (for example
`usd-storage-api-deployment/environments/<env>/thumbnail-service/values.yaml`):

```yaml
notifications:
  eventConsumer:
    endpoint: "event-consumer-service:50051"    # required — shared by StorageEventQueue and WorkerEventQueue
    resourcePrefixes:                           # required — at least one entry; hashes into both queues' ids
      - "https://<account>.blob.core.windows.net/<container>/"
      - "https://<account>.blob.core.windows.net/<another-container>/"
```

Then deploy as usual (`helm upgrade` / your GitOps flow).

**Queue ids** — Each binary derives its own queue's id at startup from `resourcePrefixes`; Helm
injects `NOTIFICATIONS_QUEUE_ID` on neither Deployment:
- StorageEventQueue (`server`): `thumbnail-service-storage-<sha256 hex of "<sorted prefixes>">`
- WorkerEventQueue (`task-worker`): `thumbnail-service-worker-<sha256 hex of "<sorted prefixes>">`

The `thumbnail-service-` prefix is a hardcoded literal (not a values key) so both ids read as
belonging to this service, and the same for every deployment of this chart, Helm or bare binary.
The `storage`/`worker` segment is what keeps the two ids distinct — neither digest carries a
queue-kind discriminator.

**Checking that both Deployments agree** — because the digest is a function of the prefix set
alone, the two ids carry the *same* digest whenever both components read the same
`resourcePrefixes`. In a queue listing, compare the pair:

```text
thumbnail-service-storage-dc9ff58a…  ┐ same digest → both agree on resourcePrefixes
thumbnail-service-worker-dc9ff58a…   ┘

thumbnail-service-storage-dc9ff58a…  ┐ different digest → one Deployment's config is stale
thumbnail-service-worker-053670e3…   ┘
```

This is worth checking when thumbnails stop being produced with no errors anywhere. WorkerEventQueue
filters `omni.thumbnail.generate` on the same source-address prefixes the orchestrator watches, so a
worker whose prefix set has drifted binds filters that match nothing: its queue sits at zero
deliveries while StorageEventQueue runs normally, which is indistinguishable from no eligible files
arriving.

Changing `resourcePrefixes` therefore rotates both queues to freshly-provisioned ones carrying the
new filters — there is no separate values key to override either id.

**`notifications.eventConsumer.resourcePrefixes`** — list of storage container URL prefixes used to build queue filters for
`omni.storage.created` and `omni.storage.deleted`, shared by both consumers. The chart joins the list with `,` into
`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`, and each entry becomes one greedy-prefix filter on the
single durable queue, so one deployment can subscribe across multiple buckets/prefixes. Use the
**container root** for each entry, not a subdirectory like `.../thumbnails`. This is the same scope
as `RESOURCE_PATH` in `scripts-notifications/.env.example`, not
`thumbnailPathTemplate` (which only controls where thumbnails are written —
see **Thumbnail destination path template** below).

Helm validates at **render time**: an enabled event consumer with
an empty or all-blank `resourcePrefixes` list (every entry empty/whitespace-only) fails
`helm template` / `helm upgrade` before anything reaches the cluster. Blank entries inside an
otherwise non-empty list are trimmed and dropped at render
(`["a", "", "b"]` → env value `a,b`); the raw-env path is stricter — a comma-separated
`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` value with a blank slot (`a,,b`) is rejected at parse
time. Same malformed input, two behaviors by layer; prefer well-formed lists in both paths.

### What happens on startup

1. Before its consume loop starts, each binary calls `CreateDurableQueue` with its own derived
   queue id and filter groups carrying one greedy-prefix filter per entry in `resourcePrefixes` —
   `server` for `omni.storage.created` / `omni.storage.deleted` on StorageEventQueue,
   `task-worker` for `omni.thumbnail.generate` on WorkerEventQueue.
2. Queue does not exist → created (`[PROVISION] Durable queue created` in logs).
3. Queue already exists (for example pre-created by script) → idempotent no-op
   (`[PROVISION] Durable queue already existed`).
4. If the create response `queue_id` does not equal the configured id, provisioning fails and
   **startup aborts** — the process will not attach consume to a different returned id.
5. Consumer attaches and streams events as before.

You no longer need to run `create-durable-queue.sh` before every deploy. The script remains useful
for update/drain/delete workflows.

### Prerequisites

- `notifications.eventConsumer.storageEventQueue.enabled: true` with non-empty `notifications.eventConsumer.resourcePrefixes`.
- Event-consumer service supports client-specified `queue_id` on `CreateDurableQueue` (ensure-exists
  API).
- Service identity should have **create** permission on durable queues so provisioning can actually
  create. If the identity is consume-only, `CreateDurableQueue` returns `PermissionDenied`: the
  service logs a warning and **attaches to the presumed-existing queue** (backward-compatible). That
  path only works if the queue was already created out-of-band.

See also: `thumbnail-service/README.md` (env-var reference), `.env.example` (local dev), and
`scripts-notifications/README.md` (manual queue scripts).

## Thumbnail destination path template

`thumbnailPathTemplate` (env: `THUMBNAIL_PATH_TEMPLATE`) has no chart
default — every environment must set it. thumbnail-service resolves this template into one exact,
absolute address per generated thumbnail and sends it as `destination_resource_address` on every
generation request (event-driven, HTTP, gRPC). `task-worker` consumes that work, converts and
uploads the thumbnail at exactly that address, then asks thumbnail-service to associate it back to
storage.

Supported variables: `{resource_path}`, `{resource_filename}`, `{resource_address_md5}`,
`{resource_identity_md5}`, `{width}`, `{height}`, `{output_encoding}`, `{quality}`. The template
must identify its source via `{resource_address_md5}` alone, or both `{resource_path}` and
`{resource_filename}` together, and must render an absolute address — `helm template`/`helm install`
won't catch a bad template (it's a plain required string at render time), but the service fails fast
at startup if it can't be parsed or doesn't satisfy those constraints.

Example (dev's centralized, collision-resistant-by-hash layout):

```yaml
thumbnailService:
  app:
    thumbnailPathTemplate: "https://examplestorage.blob.core.windows.net/example/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}"
```

A source-relative alternative (sibling `_thumbnails` directory next to the source) is also
supported: `{resource_path}_thumbnails/{width}x{height}/{resource_filename}.{output_encoding}`.

See `thumbnail-service/README.md#thumbnail-destination-path-template` and
`docs/adr/0001-thumbnail-storage.md` for the full variable reference and the
generated-output recursion-prevention design (event markers and, when configured, writer principal
via `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY`, plus template-output shape matching). Chart support
for that env var was removed while the check is inert; re-add via `extraVars` or wait for Helm
exposure once storage propagates writer principal.

## Multi-size fan-out notes

- Event-driven mode NACKs the source event when any per-size submission fails.
- Broker redelivery re-fans-out all sizes; because downstream output identity is task-id-based, previously successful sizes can leave orphaned outputs after retries.
- Mitigations are outside this chart: task-queue idempotency keyed by `(resource_identity, width, height)` and/or periodic cleanup jobs.
- Thumbnail-service controls the requested `(W, H)` set and, via root `thumbnailPathTemplate`, the destination layout/addressing.
- `destination_resource_address` is always set by thumbnail-service on every generation request, resolved from the template; task-queue writes exactly there and performs no derivation of its own.

## Secrets

The chart does **not** create Kubernetes Secrets. In client-credentials mode,
set `serviceIdentity.clientCredentials.clientSecretRef.name` to an existing
Secret (for example one provisioned by Azure Key Vault CSI). Both Deployments
mount that Secret at `/etc/secrets` and point `CLIENT_SECRET_FILE` at
`clientSecretRef.key` (default `CLIENT_SECRET`).

Sidecar (`accessTokenFile`) mode has no client secret to mount.

## Outbound TLS posture

The service and worker reach storage, notifications and each other over
**plaintext HTTP/2**. This is deliberate and now enforced: an endpoint beginning
`https://` is refused at startup with an explanation, rather than failing later
with an opaque connect error.

The reasoning: this build links `tonic` without a TLS feature, so it cannot
complete a handshake; TLS terminates at the Contour HTTPProxy below; the
endpoints are cluster-internal Service addresses; and the chart configures no
service mesh or mTLS sidecar. The consequence worth stating plainly is that
service-account bearer tokens travel in gRPC metadata in the clear between pods,
and the trust boundary is the cluster network.

Changing that is a real change, not a configuration flip. It needs a `tls-*`
feature on `tonic`, endpoints that honor the scheme, chart values for the trust
bundle, and a certificate lifecycle. Setting an `https://` endpoint without those
never worked; before this it produced either an unexplained transport failure or
the address `http://https://host:443`.

## External Access (HTTPProxy)

Enable Contour HTTPProxy for external ingress:

```yaml
thumbnailService:
  httpProxy:
    enabled: true
    fqdn:
      host: "thumbnail"
      domain: "example.com"
    tls:
      enabled: true
```

Creates endpoints:
- REST: `thumbnail.example.com`
- gRPC: `grpc.thumbnail.example.com`

## Task-worker

The conversion half of the thumbnail pipeline, rendered by this same chart as
a separate Deployment (`deployment-service.yaml` / `deployment-worker.yaml`) under the
`thumbnailWorker:` values key. It consumes `omni.thumbnail.generate` work from its own durable
Notification queue, converts and uploads the thumbnail, and asks the
orchestrator to record the association.

It serves no API. `workerRuntime.healthPort` carries `/healthz`, `/readyz`,
and `/metrics`, and nothing else; its default is `8080`.

Shared root values feed both containers: `storage.endpoint` →
`STORAGE_GRPC_ENDPOINT`; `streaming.streamTimeoutSecs` /
`streaming.maxReconnectBackoffSecs` → `STREAM_TIMEOUT_SECS` /
`MAX_RECONNECT_BACKOFF_SECONDS` on the worker as well as the orchestrator.
There is no per-component storage endpoint.

### Two queues, two acknowledgement domains

Thumbnail work crosses Notification twice, on two separate durable queues.
This is the single most important thing to understand before operating this
chart.

```text
omni.storage.created ──► StorageEventQueue ──► orchestrator
                                      │  eligibility, path template, size fan-out
                                      └─► PublishEvent(omni.thumbnail.generate), one per size
                                                        │
omni.thumbnail.generate ─────────────► WorkerEventQueue ──► task-worker
                                                        │  convert, upload
                                                        └─► gRPC AssociateThumbnail ──► orchestrator
```

| | StorageEventQueue | WorkerEventQueue |
|---|---|---|
| Owner | orchestrator (`thumbnail-service` container) | task-worker (`thumbnail-worker` container) |
| Event types | `omni.storage.created`, `omni.storage.deleted` | `omni.thumbnail.generate` |
| Queue id | `thumbnail-service-storage-<sha256 hex>` | `thumbnail-service-worker-<sha256 hex>` |
| Filters | one greedy prefix per watched container (required) | one greedy prefix per watched container (required) |
| An ACK means | the work was durably **published** | the thumbnail was converted, uploaded, and associated |

Both ids share the `thumbnail-service-` prefix (a hardcoded literal, so any deployment of this
chart or binary produces recognizably-named queues) and then a `storage`/`worker` segment, followed
by a sha256 hex digest of the sorted prefix set. The digest carries no queue-kind discriminator, so
for one prefix set both ids end in the **same** digest and differ only in that segment. The two
queue ids **must differ**, and do so by construction: `-storage-` and `-worker-` are different
literals at a fixed position ahead of a fixed-length digest, so no digest value can make the ids
equal — the guarantee does not depend on SHA-256 collision resistance. The shared digest is also
the check that both Deployments agree on `resourcePrefixes` (see **Queue ids** above). Changing the
prefix set rotates **both** queues to a fresh id. A create response that returns a different id
aborts startup; the consumer does not attach to the returned UUID while config still names
something else.

Acknowledging a StorageEventQueue delivery says the work was enqueued, **not** that any
thumbnail exists. Conversion failures happen later, on WorkerEventQueue, and never
reopen the original CREATE. There is deliberately no "2 of 3 sizes converted"
result anywhere: partial outcomes are observed through per-thumbnail
association events, the worker metrics below, and WorkerEventQueue's dead-letter queue.

### Prerequisites before the first rollout

Provision these in order. The ordering is not a nicety: `PublishEvent`
succeeds whether or not any queue is bound to the event type, so an
orchestrator that dispatches generate events into an environment without
WorkerEventQueue acknowledges every CREATE for work that is silently dropped.

1. **Notification deployment — permissions YAML.** Durable consume of a
   *non-storage* event type goes through the fine-grained `event_permissions`
   check, not the storage `consume-all-*` actions. Add an entry for
   `omni.thumbnail.generate` to the file named by
   `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH`, plus the matching policy on
   `EventResourceId`. This is a change to the Notification deployment, not a
   grant that can be made from this side.
2. **IAM grants** on the shared service-account principal:
   - `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate`
   - `event-consumer-service:create-durable-queues`
   - `event-consumer-service:consume-durable-queues`
3. **WorkerEventQueue**, with an `omni.thumbnail.generate` event-type filter. The worker
   self-provisions it on startup, so deploying this release is normally
   enough — but confirm it exists and is consuming before rolling out an
   orchestrator image that publishes generate events.
4. Recursion prevention: thumbnails land back in watched storage and their
   CREATEs return to StorageEventQueue. Template-output shape matching against
   `THUMBNAIL_PATH_TEMPLATE` is the active guard today. The optional
   writer-principal check (`GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY`) remains
   available on the binary via `extraVars` but is not chart-configurable while
   storage does not propagate that identity — see
   [ADR 0001](../../docs/adr/0001-thumbnail-storage.md).

### Redelivery, retries, and the dead-letter queue

`ConsumeDurableEventsWithAcksRequest` now carries `nack_delivery_tags` alongside
`ack_delivery_tags`, so a retryable failure is returned to its queue by an
explicit negative acknowledgement rather than by withholding the acknowledgement
and waiting for a reconnect. Both durable queues use it: WorkerEventQueue in the
task-worker, and StorageEventQueue in whichever component runs the storage-event
consumer.

The broker requeues a negatively acknowledged delivery **immediately**, so all
spacing between successive attempts at the same work is consumer-side. Three
values set it, and they apply to both queues:

| Setting | Default | Purpose |
|---|---|---|
| `NACK_REDELIVERY_DELAY_SECS` (worker: `REDELIVERY_RECYCLE_INTERVAL`) | 60s | How long a failed delivery's negative acknowledgement is held before being sent |
| `NACK_DEFERRAL_CONCURRENCY_LIMIT` | 10 | Most negative acknowledgements held at once |
| `NACK_EARLY_REDELIVERY_LIMIT` | 5 | Consecutive early redeliveries of one delivery key before the session is recycled |

Two bounds matter operationally. The delay must stay far inside the broker's
acknowledgement timeout (1800s in dev), because a held delivery is still in
flight and a consumer that exceeds that timeout is cancelled off a queue that
still has work — the 2026-08-25 stall. And the number held at once has to be
capped, because held deliveries occupy the prefetch window; past the cap a
failure is negatively acknowledged immediately and the anti-spin gate spaces the
redelivery instead. Deferrals are held by each consume loop itself and flushed
when a session ends, so a reconnect never abandons one.

Attempts are spaced per **delivery key** — the logical work identity, which
survives a redelivery where the delivery tag does not. StorageEventQueue keys on
the storage event's `resource_identity` (falling back to `resource_address`); the
worker keys on the generate message's `destination_resource_address`.

The recycle survives in two narrow roles rather than as the retry path: as the
fallback when a negative acknowledgement cannot be delivered at all (broken ack
stream, or a panicked conversion that leaves no tag), and as the circuit breaker
when one key is redelivered early past `NACK_EARLY_REDELIVERY_LIMIT`, which only
happens if the broker requeues without honouring the delay. A healthy consumer
never recycles.

Each redelivery advances the platform's delivery counter, so Notification's own
bounded redelivery and dead-lettering apply unchanged. Inspect and replay
dead-lettered generate messages with the platform's `ManageDeadLetteredEvents`
RPC.

#### Quiet streams stay open

A healthy stream with nothing outstanding is left alone. The worker does **not**
rebuild solely because the queue has been silent — matching the Notification
consumer already approved in RC (permission-service). Reconnects happen on
stream end, transport error, `UNAUTHENTICATED` after token refresh, token near
expiry (refresh then reopen), or one of the two recycle triggers above. Neither
trigger is elapsed silence, so a quiet healthy stream is left alone. Transport
keepalive remains configured on the channel so a genuinely dead peer is noticed;
it does not count as activity to an L4 idle reaper, and that is accepted.

Readiness follows the stream: a replica that has no established stream is not
Ready. Operators watch `thumbnail_worker_stream_recycles_total` and
`thumbnail_worker_stream_token_reconnects_total` together with WorkerEventQueue depth for
path health — not a silence-based idle rebuild counter.

#### Upgrade note

`OutboundClients::build` now fails process startup if no usable service-identity
bearer arrives within 60 seconds (`TOKEN_READY_TIMEOUT`). A slow IdP or a
late-starting access-token sidecar becomes CrashLoopBackOff instead of starting
and serving 401s. Prefer a native sidecar (`restartPolicy: Always` on the init
container) so the token file is present before the app container starts; 60s is
generous for a cold file but will look alarming if the sidecar is a plain
parallel container that lags.

Both Deployments ship a `startupProbe` with a ~125s budget so liveness does not
SIGKILL the pod while it is still waiting for that first bearer (the worker also
binds `/healthz` before `OutboundClients::build`). Without the startup probe,
chart liveness (~50s) would kill the process before the gate's own timeout could
report.

What the worker does **not** do is retry in process. There is no Celery-style
`max_retries` countdown: one attempt per delivery, then classify and either
acknowledge or negatively acknowledge it. Retrying in place would hold a
conversion slot open while the dependency is down, which is the opposite of
useful under stress.

Failure handling in full:

| Outcome | Acknowledgement | Where it shows up |
|---|---|---|
| Converted, uploaded, associated | ACK | `omni.thumbnail.associated`, success metric |
| Validation failure, unsupported processor or format, source missing, decode bound exceeded | **ACK** | error log naming the destination and class, plus a failure metric |
| Storage or associate unavailable, timeouts | **NACK** after the redelivery delay | retryable metric, `thumbnail_worker_nacks_total`, then redelivery; eventually the platform DLQ |

Permanent failures are acknowledged deliberately. A poison message that kept
its delivery would tight-loop forever and never reach the dead-letter queue
either, because the delivery counter only advances on an actual redelivery.
That is why every permanent rejection is logged at error level with its
destination before the ACK — it is the only durable trace such a message
leaves.

### Metrics

`GET :<workerRuntime.healthPort>/metrics` (port `8080` by default) returns a text
exposition of:

- `thumbnail_worker_outcomes_total{outcome="success"|"permanent_failure"|"retryable_failure"}`
- `thumbnail_worker_failures_total{class="validation"|"unsupported_processor"|"unsupported_format"|"source_missing"|"decode"|"upload"|"associate"}`
- `thumbnail_worker_conversion_milliseconds_total`
- `thumbnail_worker_unacknowledged_deliveries` — watch this; a rising value
  means retryables are accumulating faster than they retire, and the queue is
  heading for its maximum length. Both an acknowledgement and a negative
  acknowledgement release it, so it reflects work actually in flight
- `thumbnail_worker_nacks_total{reason="retryable_failure"|"early_redelivery"}` —
  deliveries returned to the queue, by cause. Graph it as a rate, not a raw
  value: like every counter here it is per-pod and resets on restart, so
  `sum by (reason) (rate(thumbnail_worker_nacks_total[5m]))` is the useful form.
  `retryable_failure` is the retry rate — a steady climb means a dependency is
  persistently failing. `early_redelivery` means the broker is redelivering ahead
  of the configured delay and the consumer is rejecting it without reprocessing;
  sustained, that is what trips the circuit breaker below.
  Watch it against `outcomes_total{outcome="retryable_failure"}`: failures
  climbing while NACKs flatline means settlements are not reaching the broker
- `thumbnail_worker_stream_recycles_total` — now only the fallback and
  circuit-breaker roles, so any sustained rise means negative acknowledgements
  are not getting through, or the broker is ignoring the redelivery delay
- `thumbnail_worker_stream_token_reconnects_total` — near-expiry refresh that
  produced a usable bearer and reopened the stream; a 1 Hz climb means refresh
  is not clearing expiry (sick IdP or sidecar file still inside its skew window)

These replace the retired Python task table: `GET /tasks/{id}`,
`DELETE /tasks/{id}`, Flower, and `/celery/stats` are all gone with it.
Per-item status is no longer queryable. Use these counters for outcome rates,
WorkerEventQueue depth for backlog, and `ManageDeadLetteredEvents` for individual
poison messages.

### Sizing

`thumbnailWorker.replicaCount` is the "number of workers" knob — the worker is
single-process and scales horizontally, so one pod is one blast radius for
image decode. `workerRuntime.maxConcurrentConversions` controls how much
each replica does at once.

Memory is bounded by `workerRuntime.maxTransformPeakBytes`, one budget per
in-flight conversion:

```text
limits.memory  ≥  maxConcurrentConversions × maxTransformPeakBytes + ~200 MiB idle
                  2 × 1 GiB + 200 MiB ≈ 2.2 GiB  →  2304Mi
```

The bound is set by the **resample**, not the decode, which is why it is a byte
budget rather than a pixel count. `imageops::resize` runs two passes, and the
first allocates a working buffer of `source_width × new_height` at 16 B/px
(`Rgba32F`) while both the source and the result are resident. That width comes
from the source, so it does not shrink with the request: a 1,000,000×64 image is
exactly 64 MP and passes every decode check, yet a 100×100 thumbnail of it
allocates 1.6 GB. No transform is safe merely because its output dimensions are
small.

The largest conversion the API contract permits — a 64 MP source at 8000×8000
resampled to the 8192×8192 `MAX_DIMENSION` ceiling — costs 488 MiB of source,
1000 MiB of working buffer and 256 MiB of result, or 1744 MiB for one
conversion. The chart does not size for that: the configured `thumbnailSizes`
are 256×256 and 512×512, whose realistic worst case is a 64 MP source with a
flatten at ~553 MiB. Anything over the budget is a permanent conversion failure
with a clear reason, not an OOM kill, so the trade is rejected extremes for a
pod less than half the size. Raise `maxTransformPeakBytes` and `limits.memory`
together if you need to serve the ceiling.

The 750Mi request is the typical JPEG load-test working set rather than that
worst case. The gap is left on purpose: packing HPA `maxReplicas: 20` on the
request (15 GiB) rather than the limit (45 GiB) is the scheduler behaviour we
want until a pathological input shows up in production. The chart fails to
render if `limits.memory` cannot cover the budget above, so raising concurrency
or `maxTransformPeakBytes` without raising the limit is an install error rather
than a silent OOM.

CPU is sized the same way — against concurrency — but from measurement rather
than a bound. A worker drew 2.3–2.7 cores under backlog at
`maxConcurrentConversions: 4` without reaching its limit, so each slot averaged
~0.6 of a core across its download, convert, upload and associate phases. Two
slots draw ~1.2–1.4 cores, which is where `requests.cpu: 1300m` comes from; it
tracks real draw rather than a utilization target, keeping the pod above the
HPA's 70% target under backlog. `limits.cpu: 2500m` is the ceiling the pod can
usefully draw — conversion is single-threaded, so two slots cannot exceed two
cores of conversion work, and the remainder covers acknowledgements and probes
while both slots are in their CPU phase.

One coupling to know before lowering `limits.cpu` further: it is the CPU
*limit*, not the node, that sizes the async runtime.
`available_parallelism()` reads the cgroup quota and rounds down — a 2.5-core
quota reports 2 — and the worker's `#[tokio::main]` takes its worker-thread
count from that. Two async threads suffice only because conversion runs on
`spawn_blocking`, keeping the CPU-bound work off the async threads so the
consume loop stays responsive.

Two earlier fixes still apply, with a narrower claim than they first had:

- **`drop-source-bytes-after-decode`** — the encoded source buffer is freed
  immediately after decode, so it is not in the flatten peak.
- **`flatten-alpha-in-place`** — PNG 16-bit gray+alpha plus JPEG output is
  the 8 B/px path: `into_rgba8()` keeps the decoded buffer and a 4 B/px
  RGBA8 copy. 8-bit RGBA JPEG blends in place (decoded size only). PNG
  output skips flatten. `Rgba16` peaks at decoded + 4 B/px (12 B/px) but
  `max_alloc` caps it at 32 MP, so it loses to La16 at 64 MP in absolute
  bytes.

### Format support

The worker decodes JPEG, PNG, GIF, BMP, TIFF/TIF, ICO, and WebP, and encodes
JPEG, PNG, GIF, and WebP. SVG, HEIC/HEIF, and AVIF are **not** supported and
are not on the orchestrator's eligibility allowlist — each needs a decoder the
`image` crate does not ship. They produced failed tasks under the Python
worker too, so nothing regresses; they are now skipped before any work is
published.

**One deliberate divergence from the Python worker:** WebP output is encoded
losslessly and `quality` is ignored for it, because the pure-Rust encoder has
no lossy mode. A WebP-configured deployment will see noticeably larger
thumbnails than before. Every other transform, crop direction, and encoding
reproduces the Pillow behavior exactly.

### Running your own event distribution service

A customer may run their own event distribution service in place of
Notification, provided it conforms to the Notification Event Aggregation API.
The worker talks only to the published API at its configured endpoint, so
pointing it at a conformant implementation is a configuration change with no
code change, alternate build, or feature flag.

Conformance to the API is necessary but **not sufficient**. The worker's
failure behavior depends on semantics the proto does not express, and an
implementation can satisfy every RPC signature while breaking it. A
conformant service must also provide:

- **Acknowledgement-only completion.** There is no negative acknowledgement;
  completion is expressed by sending a delivery tag, and its absence is the
  only failure signal.
- **Redelivery of unacknowledged messages on reconnect.** The worker's
  recycle mechanism above depends on a reconnect redelivering what was not
  acknowledged. A service that drops unacknowledged messages on disconnect
  loses work.
- **Bounded redelivery leading to dead-lettering.** A message that fails
  repeatedly must eventually stop being redelivered and become inspectable.
  Without a bound, a poison retryable circulates forever.

Delivery limits, message TTL, and maximum queue length are deployment
configuration on the Notification side and are documented in prose rather
than in the API surface. Getting them wrong changes the worker's behavior
without changing any call it makes.

### Logging

| Parameter | Description | Default |
|-----------|-------------|---------|
| `observability.logging.level` | Injected as `RUST_LOG` on both components | `info` |

JSON log output is unconditional on the worker (same as the orchestrator). There is no `structured.enabled` toggle on either component.
OTLP trace and metric export use the same `observability.tracing.*` / `observability.metrics.*` values as the orchestrator.

### Tracing

One thumbnail is one trace. It starts at the inbound `CreateThumbnails` request or
at the `omni.storage.created` delivery, crosses the `omni.thumbnail.generate` queue
on the message itself, and continues through the worker's conversion and its
`AssociateThumbnail` call back into the orchestrator.

Three consequences of joining that trace are worth knowing before tuning anything:

**Traces span queue dwell.** A trace stays open from publish until the worker
finishes, so its wall-clock duration includes however long the message waited. A
backlogged worker produces long traces where it previously produced two short,
disconnected ones. Watch for traces whose total duration is minutes while the sum
of their span durations is seconds — that is queue depth, not slow work, and it
makes any latency dashboard built on trace duration misleading.

**Worker spans inherit the producer's sampling decision.** The sampled flag travels
in the trace context, and the sampler is always `parentbased_traceidratio`. So
lowering `observability.tracing.samplingRatio` does not thin worker telemetry evenly — it drops the
conversion spans for every request that was not sampled, which is the expensive
half of the trace and usually the half you wanted. Sample for the volume of
*conversions* you want to see, not the volume of API calls.

**Association-endpoint latency reads lower than it used to, and the old number was
wrong.** `AssociateThumbnail` and the disassociate paths publish
`omni.thumbnail.associated` / `disassociated` off the critical path, and those
publishes briefly shared the request's span. Because a span's exported end time is
its last exit, the request's reported duration became the publish's: a 20 ms
associate against a slow notifications service exported as several seconds, with no
child span accounting for the gap. The publish now gets a span of its own —
`thumbnail.association_event`, a child of the request in the same trace — so the
request span ends when the request does. If you have an alert threshold tuned to
the old figure, it is measuring something that no longer happens.

While you are in that neighborhood: the per-attempt `[RETRY]` line from the storage
metadata client is now DEBUG rather than WARN. Since outbound calls gained a client
trace layer, every failed attempt already emits `rpc.client.call.exception` at WARN
with the error chain, and that builder re-creates its channel per attempt — so a
storage outage was producing two WARN lines per attempt at exactly the moment
someone is reading the logs. What `[RETRY]` adds over the layer's line is the
attempt number and the backoff; the failure of the operation as a whole is still
reported by its caller. Turn the level up to `debug` to get the per-attempt detail
back.

One gap remains, and it is not ours to close: the storage service does not yet
stamp `omni.storage.created` with trace context. Until it does, a thumbnail
generated from a storage event starts its trace at this service rather than at
whatever wrote the source object. Nothing breaks in the meantime; the trace is
simply shorter at the front. When storage does start sending one, both halves of
the W3C context are honored on that hop — `traceparent` and `tracestate` — so a
sampling decision made upstream is not silently overridden here.

## Full Values Reference

See `values.yaml` for all configuration options with documentation — shared
root keys at the top, then `thumbnailService:` and `thumbnailWorker:`.
