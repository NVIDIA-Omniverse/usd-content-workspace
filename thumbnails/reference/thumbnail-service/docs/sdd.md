# Thumbnails API — Software Design

> **Scope note.** This SDD covers the whole Thumbnails API. It now lives in two sibling repos under
> `api-thumbnails/`: **`thumbnail-service`** (Rust — the API and the conversion worker) and
> **`thumbnail-dev-utils`** (Bash). **`thumbnail-task-queue`** (Python) was retired when conversion
> moved into `crates/task-worker`; see `docs/cutover-runbook.md`. All quorum specs/plans/bees live
> in `thumbnail-service`.

## Tech stack

**thumbnail-service** (Rust, edition 2021, toolchain 1.94, Cargo workspace)
- Async runtime: `tokio`; HTTP: `axum` 0.7 + `utoipa`/Swagger UI; gRPC: `tonic` 0.14 (+ `tonic-web`,
  reflection, `prost`); HTTP client: `reqwest` 0.12.
- Config/CLI: `clap` 4 (env-driven); observability: `tracing` with OpenTelemetry export via
  `otel-tools` / `tonic-observability` / `axum-observability`; errors: `thiserror`; auth: internal
  `permission-auth` crate. Tests: `proptest`, `wiremock`, `serial_test`.

**task-worker** (same Rust workspace, its own binary and its own Deployment)
- Consumes `omni.thumbnail.generate` from a durable Notification queue; converts with the `image`
  crate via `crates/thumbnail-convert`; reads and writes blobs over storage `FileObjectService`;
  associates by calling thumbnail-service `AssociateThumbnail` over gRPC. Links no HTTP framework —
  its probe surface is a plain `TcpListener` — so image codecs and the API stack stay in separate
  deployables.

  It replaced Python/Celery/RabbitMQ/Postgres wholesale. There is no task store: outcome counters
  on `/metrics` and the associations themselves are the record of what happened.

**thumbnail-dev-utils** (Bash)
- Shell orchestration driving `cargo`, `poetry`, `uvicorn`, `celery`/`flower`, `alembic`, `kubectl`,
  `helm`, `k3d`, `docker`. Dev-quality: `pre-commit` (active); `shellcheck` runs in CI only and is
  non-blocking (the shellcheck pre-commit hook is disabled).

## Architecture overview

The Thumbnails API is two deployables from one Rust workspace, plus a dev-only orchestration repo.

**`server`** (`crates/service`) is the front door and the metadata authority. It exposes the public
API over two protocols (HTTP REST on :3000 and gRPC on :50051) backed by shared business logic in
the `thumbnail-common` crate. It owns thumbnail↔source associations (persisted via the storage
service), resolves each output's exact destination from `THUMBNAIL_PATH_TEMPLATE`, and publishes one
`omni.thumbnail.generate` event per thumbnail to be produced. It can also consume storage events, so
a qualifying file landing in storage triggers generation without an API call.

**`task-worker`** (`crates/task-worker`) is the async compute tier: its own binary, its own
Deployment, and no API surface. It consumes `omni.thumbnail.generate` from its own durable queue,
converts with the `image` crate via `crates/thumbnail-convert`, uploads the result over storage
`FileObjectService`, and calls back into `server` over gRPC to record the association. It does
**not** own association metadata — that authority stays in `server` — and it holds no task store:
outcome counters on `/metrics` and the associations themselves are the record of what happened.

**`thumbnail-dev-utils`** is a Bash meta-repo that stands up the whole stack locally (bare-metal
processes or a local k3d cluster), wiring both components to remote dev infrastructure (storage,
event-consumer, event-aggregation) via `kubectl port-forward`. It ships no runtime artifact. The
repos are expected as developer-cloned sibling working copies (not git submodules).

Two producers publish generate work — the storage-CREATE consumer and the `CreateThumbnails` API —
and they differ only in how a partial failure is reported. A CREATE delivery has an acknowledgement
to decide and no caller to tell, so any failure fails the batch; the API reports per-specification
outcomes. There are two durable Notification queues with two distinct ids and two acknowledgement
domains: ACKing StorageEventQueue means the work was durably *published*, not that a thumbnail
exists.

```text
omni.storage.created                          Client (HTTP or gRPC)
        │                                              │
        ▼ StorageEventQueue                            ▼ CreateThumbnails → storage Stat
┌──────────────────────────── server (crates/service) ───────────────────────────┐
│  event consumer ─┐        HTTP :3000 / gRPC :50051                             │
│                  ├─────► thumbnail-common: eligibility, path template,         │
│  association authority (via storage service)          size / spec fan-out      │
└───────┬────────────────────────────────────────────────────────────┬───────────┘
        │ publish omni.thumbnail.generate (one per output)            │ AssociateThumbnail (gRPC)
        ▼ WorkerEventQueue                                           ▲   → server then publishes
┌──────────────── task-worker (crates/task-worker) ──────────┐       │     omni.thumbnail.associated
│  convert (crates/thumbnail-convert) → upload → associate   │───────┘
│  reads source + writes thumbnail to blob via storage       │
└──────────────────┬─────────────────────────────────────────┘
                   ▼
        Storage service + Azure Blob (thumbnail output, at destination_resource_address
                                       resolved by server from THUMBNAIL_PATH_TEMPLATE)
```

**One thumbnail is one trace, from this service onward.** Every hop drawn above that
originates here carries W3C trace context: the generate message across the queue, the
conversion, the storage reads and writes, and the association call back into `server`. The
queue is the hop that matters — nearly all the elapsed time is on the far side of it, and
before context crossed it the expensive work was unattributable to whatever asked for it.

The one hop that does not carry it is the first: `omni.storage.created` arrives without
trace context, because the storage service does not stamp its events yet. A thumbnail
generated from a storage event therefore starts its trace at the delivery span this service
opens rather than at whatever wrote the source object. An API-initiated thumbnail has no
such gap — its trace starts at the inbound request. See
`helm/thumbnail-service/README.md#tracing`.

Context is continued rather than forwarded. Each component parents its own span to the
context it received and publishes its own span's context onward, so no hop is skipped over
in the resulting tree. A message with absent or malformed context is processed normally
and the consumer roots a new trace — trace context is telemetry and never a precondition
for work. Operational consequences of the joined trace, including how sampling now reaches
worker spans, are in `helm/thumbnail-service/README.md#tracing`.

## Key components

**thumbnail-service (crates)**
- **`crates/common` (`thumbnail-common`)** — core shared logic: `auth`, `config`, `converters`,
  `errors`, `notifications` (event consumer: connection, streaming, event processing, publisher),
  `outbound`, `resource`, `runtime`, `spec_enums`, `storage` (association + metadata clients),
  `task_queue` (client/conversion), `thumbnails`, `thumbnail_sizes`, `types`, `utils`.
- **`crates/services/http` (`thumbnail-http`)** — Axum REST API; binaries `http_server`,
  `generate_openapi`. Routes: `GET /health`, `GET /v1alpha/thumbnails`, `POST/GET /v1alpha/tasks`,
  `GET/DELETE /v1alpha/tasks/{task_id}`, `POST/DELETE /v1alpha/associations`.
- **`crates/services/grpc` (`thumbnail-grpc`)** — Tonic service `ThumbnailService` (CreateThumbnails,
  GetThumbnailTask, CancelThumbnailTask, AssociateThumbnail, DisassociateThumbnail, GetThumbnails).
- **`crates/services/thumbnail-smoke-test`** — standalone CLI smoke test (Helm test hook).
- **`crates/auth`** — wraps `permission-auth`; runtime-selected `AuthInterceptor`.
- **`crates/proto/{thumbnail,storage,notifications}`** — generated tonic stubs (thumbnail = server;
  storage + notifications = client).

**thumbnail-task-queue** (application code under `app/`; `main.py` and `workers/` at repo root)
- **`main.py`** (repo root) — FastAPI app. Endpoints: `POST /image-to-image`, `GET /tasks`,
  `GET /tasks/{id}`, `DELETE /tasks/{id}`, `GET /ping`, `GET /health`, `GET /health/detailed`,
  `GET /celery/stats` (+ `/docs`, `/redoc`, `/openapi.json`). `POST /image-to-image` accepts a single
  object **or an array**, requires both `resource_identity` and `resource_address` on each source
  (400 otherwise), and fans out into `len(sources) × len(specifications)` tasks.
- **`app/celery_app.py`** — Celery config (broker/backend, routing `app.tasks.image_to_image →
  thumbnail_queue`, `task_time_limit=300` / soft `240`, `task_acks_late`).
- **`app/tasks.py`** — the `image_to_image` task (token fetch → validate → process → upload →
  associate → status update: PENDING/PROCESSING/RETRY/SUCCESS/FAILURE). `max_retries=2` with a 60s
  countdown; terminal `FAILURE` only after retries are exhausted; association is skipped for failed
  tasks. All storage interactions use a **service-account token**, not the caller's bearer token.
- **`app/thumbnail_core.py` / `thumbnail_storage.py` / `thumbnail_metadata.py`** — processing
  orchestration, storage read/write via gRPC FileObject service, and association delegation to
  `thumbnail-service`. _(Note: `thumbnail_metadata.py`'s `_generate_and_update_metadata_*` names are
  legacy — actual behavior is association delegation, not direct metadata writes.)_
- **`app/core/image_processor.py`** — Pillow transforms (SQUARE, SCALE_BY_WIDTH/HEIGHT, CROP, FIT,
  SCALE_AND_CROP).
- **`app/core/grpc_client.py`, `app/grpc_pool.py`, `app/grpc_auth_interceptor.py`,
  `app/token_manager.py`** — gRPC channel/pool/auth interceptor, OAuth2 service-account token
  acquisition. _(`app/core/event_publisher.py` implements an `omni.thumbnail.created` publisher but is
  currently **unwired** — nothing calls it; events are published by thumbnail-service on associate.)_
- **`models.py` / `schemas.py` / `database.py`** — `Task` ORM + Celery meta tables, Pydantic schemas,
  DB session/engine. Tables are created at runtime via SQLAlchemy `create_all` at startup (see
  Deployment re: Alembic).

**thumbnail-dev-utils**
- **`local-scripts/`** — `start-local.sh` (builds Rust binaries, runs uvicorn API + Celery worker +
  Flower, sets up port-forwards, runs smoke/health suite), `stop-local.sh`, `restart-local.sh`,
  `setup-git-hooks.sh`.
- **`k3d/k3d.sh`** — unified CLI (`create|destroy|ports|deploy|build|restart|logs|status|undeploy`)
  for running the stack in a local k3d cluster via Helm.

## External dependencies

- **Storage service (gRPC)** — file/thumbnail metadata plus source-image read and thumbnail write
  (FileObject service). Both `server` and `task-worker` reach it through `STORAGE_GRPC_ENDPOINT`.
- **thumbnail-service (gRPC)** — `task-worker` calls `AssociateThumbnail` after every successful
  upload, via `THUMBNAIL_SERVICE_GRPC_ENDPOINT`. Association is the service's to own; the worker
  never writes association metadata itself.
- **Notifications / event-consumer + event-aggregation (gRPC)** — `thumbnail-service` holds a durable
  subscription to `omni.storage.*` (the auto-generation trigger, via
  `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` plus resource prefixes; StorageEventQueue's id is derived from those prefixes) and, on associate/disassociate,
  publishes `omni.thumbnail.associated` / `omni.thumbnail.disassociated` to the aggregation channel
  (`NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT`). There is no `omni.thumbnail.created` event in practice.
- **Azure Blob storage** — `thumbnail-service` resolves one exact, absolute output address per
  generated thumbnail from `THUMBNAIL_PATH_TEMPLATE` and sends it as `destination_resource_address`
  on every generation request; **`task-worker`** writes the thumbnail blob to exactly that address
  and no longer derives or falls back to any address of its own. This replaced the earlier scheme
  where the Python task queue derived its own destination under a fixed `THUMBNAIL_ADDRESS` base URL.
  Generated-output recursion (a thumbnail write re-triggering generation) is prevented by two
  checks: event-marker/writer-principal matching (optionally configured with
  `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY`, naming `task-worker` as the writer of generated output,
  and currently inert in dev) and template-output shape matching, which is therefore the only active
  guard — see `thumbnail-service/docs/adr/0001-thumbnail-storage.md`.
- **OAuth2 / OpenID (M2M)** — service-account client-credentials tokens; `thumbnail-service` also
  supports an access-token-file mode (Azure Workload Identity), delivered in the Helm deployment via a
  `token-provisioner` sidecar/init container writing a shared token file. `DEVELOPMENT_MODE=true` is a
  local-dev flag — in `thumbnail-service` it relaxes validation and returns mock thumbnail data for
  read queries rather than disabling an auth middleware.

No longer dependencies: the Python `thumbnail-task-queue`, its RabbitMQ broker, and its PostgreSQL
task store were all retired with the `task-worker` cutover. Generation work crosses Notification as
`omni.thumbnail.generate` events rather than an HTTP enqueue.

## Deployment

- Both components are containerized (multi-stage Dockerfiles; NVIDIA distroless runtime images, non-root
  user) and shipped from one Helm chart (`helm/thumbnail-service/`), which renders both as independent
  Deployments in a single release (see `openspec/changes/fold-task-worker-into-service-chart`). The
  storage-service stack is deployed by customers into their own clusters.
- **thumbnail-service** runs one Rust `service-server` container for HTTP, gRPC, and the storage-event
  consumer; it exposes ports 3000 / 50051 and publishes `omni.thumbnail.generate` work to WorkerEventQueue.
- **thumbnail-task-worker** is a separate Rust Deployment, rendered by the same chart but scaled and
  configured independently. It consumes WorkerEventQueue, converts and uploads thumbnails, then calls
  thumbnail-service gRPC to associate them. It scales independently from the API and exposes only its
  configurable probe/metrics port.
- CI in both service repos runs the stages `init → test → build → compliance → scans → publish`,
  publishing container images and Helm charts to the release registries; versions propagate to Helm charts.
  `thumbnail-dev-utils` CI is validation-only (shellcheck + `bash -n`).
- Version source of truth for `thumbnail-service` is workspace `Cargo.toml`
  `workspace.package.version`.
- The dev cluster used for integration testing is defined in the
  `usd-storage-api-deployment` repository (`environments/dev/`).

## Conventions

- **thumbnail-service:** shared logic lives in `thumbnail-common`; transport crates only map
  request/response types. Config is entirely env/clap-driven with fail-fast startup validation;
  durations are humantime strings (`15s`), not integer `*_MS`. Typed `thiserror` errors for control
  flow. **Enum values are UPPER_SNAKE_CASE across proto, Rust, and HTTP** consistently. Generated code
  (OpenAPI, protobuf) is regenerated via scripts on interface change only. `THUMBNAIL_SIZES` is a
  required config that drives **multi-size output** (1–10 entries, each dimension bounded 1..=4096) —
  a source can yield several thumbnails.
- **thumbnail-task-worker:** shared Rust generate-message validation; bounded conversion concurrency;
  gRPC storage and association calls use the shared auth interceptor and token refresh path.

## Per-feature design

_Each `/quo-plan` invocation that produces docs adds a `### Feature: <title>` subsection here._

### Feature: Durable event queue ensure-exists on startup

The assigned event consumer ensures its durable notifications queue exists before it attaches,
so operators no longer have to pre-create the queue out-of-band as a required deployment step.
This sits in front of the existing durable subscription to `omni.storage.*` described in
`## External dependencies` (`NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` + derived StorageEventQueue id);
it adds a startup-time provisioning step and does not change how the consumer streams once
attached.

**Owning module.** Provisioning lives in the `thumbnail-common` notifications area
(`crates/common/src/notifications/provisioning.rs`). It exposes `ensure_durable_queue` — the thin
async gRPC wrapper that issues a single one-shot `CreateDurableQueue` (ensure-exists semantics: the
server creates the queue or returns the existing one) — and the pure helper
`build_storage_filter_groups`, which emits the storage filter groups the ensured queue subscribes
to. The wrapper builds its own one-shot `EventConsumerServiceClient` by cloning the pre-connected
channel and shared service-account interceptor, so it never disturbs the long-lived streaming
client.

**Multiple resource prefixes.** `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` is a comma-separated list
of storage-container URL prefixes. Clap parses it into `Option<Vec<String>>` via `value_delimiter`
and per-entry `parse_non_empty` (blank entries and wholly blank values fail at parse time).
Helm’s YAML list path is softer: it trims and drops blank list entries before joining into the
env var (`["a","","b"]` → `a,b`), so the same malformed input can succeed via Helm and fail via
raw env — intentional layer difference, not a contract bug.
`EndpointArgs::resolve_queue_provisioning` then de-duplicates while preserving first-occurrence
order and fails fast (naming the variable) if unset / empty, resolving into
`QueueProvisioning.resource_prefixes: Vec<String>`.
`build_storage_filter_groups(resource_prefixes: &[String])` then emits the `omni.storage.created`
and `omni.storage.deleted` filter groups, each carrying **one greedy-prefix `ResourceFilter` per
configured prefix**. The result is a single durable queue (one `queue_id`) that subscribes across
all listed buckets/prefixes, letting one deployment auto-generate thumbnails for storage events
under multiple containers.

**Startup placement.** The step is wired into `spawn_event_consumer_supervisor` in the unified
`server` binary. It runs when this process is the assigned consumer (`EVENT_CONSUMER_SERVER`
matches the parsed role) and event consumption is not disabled. Ensure-exists runs synchronously at
that call site — before the consume loop is spawned and before the durable subscription attaches —
so the queue is guaranteed to exist (or startup has already decided how to proceed) by the time
streaming begins. It is deliberately not inside the spawned loop nor the per-reconnect
`connect_grpc` path.

**Configuration.** The assigned event consumer always ensures its durable queue exists on startup;
there is no opt-out toggle. One design-level knob governs the filter scope, alongside the
consumer-endpoint config. `NOTIFICATIONS_QUEUE_ID` is not among them: neither binary reads it, and
both queue ids are derived (see **Operator configuration** below):

- `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` — a comma-separated list of storage-container URL prefixes
  that scopes the ensured queue's resource-path filters. Clap splits on `,` and trims each entry
  (`value_delimiter` + `parse_non_empty`); blank delimited entries fail at parse time.
  `resolve_queue_provisioning` de-duplicates then sorts lexicographically. The wrapper
  builds exactly two filter groups from the result, one for `omni.storage.created` and one for
  `omni.storage.deleted`, each carrying **one greedy-prefix `ResourceFilter` per configured prefix**
  so every resource at or below any listed prefix matches. A single durable queue (one `queue_id`)
  thus subscribes across all listed buckets/prefixes. Use the storage **container root** for each
  entry, not `THUMBNAIL_PATH_TEMPLATE` (thumbnail output path). StorageEventQueue's `queue_id` is
  `"thumbnail-service-storage-" + sha256(comma-joined canonical prefixes)`.

`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` is required for the assigned server consumer; a missing value
(no non-empty prefix surviving the split) is a hard fail-fast at startup, surfaced as an `Err` naming
the specific missing variable at the supervisor call site. Helm also fails at render time when the event consumer is
enabled without a non-empty `resourcePrefixes` list; the render-time guard filters
blank/whitespace-only entries before checking, so an all-blank list (e.g. `["", " "]`) is rejected
too rather than passing render and failing later at startup.

**Operator configuration (Kubernetes).** Set `notifications.eventConsumer.endpoint` and
`notifications.eventConsumer.resourcePrefixes` (a YAML list). Both durable queue ids are derived
from that prefix set: `"thumbnail-service-storage-"`/`"thumbnail-service-worker-"` followed by
`sha256("<sorted prefixes>")`. The digest carries no queue-kind discriminator, so the two ids
share it whenever both binaries read the same prefix set — matching digests in a queue listing are
the signal that the two components agree, and the label alone keeps the ids distinct. There is no
values key for either id. See
`helm/thumbnail-service/README.md#durable-queue-self-provisioning`.

**Error semantics.** The ensure call's `tonic::Status` code drives startup behavior:

| gRPC status | Behavior |
|---|---|
| `PermissionDenied` | Warn and attach — the service continues, assuming an out-of-band queue already exists (backward-compat with the consume-only-identity path). |
| `InvalidArgument` | Fail fast at startup (permanent misconfiguration). |
| `Unimplemented` | Fail fast at startup. The deployed event-consumer service is expected to support the ensure-exists capability, so `Unimplemented` is treated as an unexpected/permanent condition — not a backward-compat fallback. (If graceful degradation against a pre-capability server were ever required, this would move to the `PermissionDenied` warn-and-attach path.) |
| `Unauthenticated` | Refresh the service-account token once and retry the call once; a persistent auth failure fails startup. |
| Transient (`Unavailable` / `DeadlineExceeded` / connection-style) | Bounded retry with the reconnect-style exponential backoff, then fail fast when the retry budget is exhausted. A shutdown signal during backoff aborts cleanly without starting the consumer. |

**Relationship to the manual provisioning script.** The manual
`scripts-notifications/create-durable-queue.sh` path is now optional: the assigned consumer
self-provisions on startup. The script remains useful for update/drain/delete and test workflows.
The `## External dependencies` notifications description is unchanged: the queue identity and
subscription topics are the same; only who creates the queue changes (the service, not an operator
script).

Scope is limited to ensuring the queue exists on startup. The service does not update, delete, or
drain queues or otherwise manage their lifecycle.
