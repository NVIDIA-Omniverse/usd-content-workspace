# Thumbnail Service

Rust workspace that exposes thumbnail APIs over HTTP (Axum) and gRPC (Tonic)
from a unified service crate, plus a standalone conversion worker. Shared business logic lives in
`crates/common` for auth, storage integration, work-event dispatch, and notifications.

## Service Surfaces

### HTTP API (`crates/service/src/http`)

- `GET /health/live`
- `GET /health`
- `GET /v1alpha/thumbnails`
- `POST /v1alpha/thumbnails`
- `POST /v1alpha/associations`
- `DELETE /v1alpha/associations`

The task-polling surface (`POST`/`GET`/`DELETE /v1alpha/tasks[/{task_id}]`) was removed with the
Python task-queue retirement; `POST /v1alpha/thumbnails` triggers generation directly and reports
per-specification failures synchronously in the response — there's no task to poll. OpenAPI output
is generated to `thumbnail.openapi.yaml`.

### gRPC API (`crates/service/src/grpc/mod.rs`)

`nvidia.omniverse.thumbnail.v1alpha.ThumbnailService` methods:

- `CreateThumbnails`
- `AssociateThumbnail`
- `DisassociateThumbnail`

`GetThumbnailTask` and `CancelThumbnailTask` were removed along with the HTTP task-polling surface.
There is no gRPC equivalent of `GET /v1alpha/thumbnails` — that query stays REST-only.

Protobuf sources for this API live in `proto/nvidia/omniverse/thumbnail/v1alpha/`. The
schemas of the services this one calls are not copied into this repository; they are
generated from their published buf schema-registry modules. See
[Regenerating protobuf code](#regenerating-protobuf-code).

The service also implements
`nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices`, which
the discovery operator calls to learn which APIs this backend serves. It answers
unauthenticated — the operator dials the gRPC port with a plaintext, credential-less
channel — and advertises the `ThumbnailService` gRPC surface plus the `v1alpha` REST
surface. The operator only asks pods whose Kubernetes Service is annotated
`discovery.omniverse.nvidia.com/enabled: "true"`, which the chart sets by default.

### `resource_identity` resolution

`CreateThumbnails` (gRPC) and `POST /v1alpha/thumbnails` (HTTP) take only `resource_address` —
there is no caller-supplied `resource_identity` field on either request. The service resolves
`resource_identity` itself via a storage `Stat` call before dispatching generation work, so the
event-driven path (storage CREATE) and both API paths all dispatch with a resolved, non-empty
identity; a missing/unstatable resource fails with `NOT_FOUND` instead.

`resource_identity` is still an explicit, caller-supplied field on `AssociateThumbnail` (required
for a `FILE` thumbnail/target, omitted for a `FOLDER` target) since that call registers an
already-existing thumbnail rather than resolving one from storage.

## Core Configuration

The service fails fast at startup if required config is missing or malformed.

### Required runtime variables

- `STORAGE_GRPC_ENDPOINT`
- `THUMBNAIL_PATH_TEMPLATE` — template used to resolve one exact output address per generated
  thumbnail (replaces the legacy `THUMBNAIL_ADDRESS`); see **Thumbnail destination path template**
  below
- `THUMBNAIL_SIZES` (comma-separated `WxH`, for example `256x256,512x512`)
- `THUMBNAIL_ENCODING_FORMAT`
- `THUMBNAIL_TRANSFORM_TYPE`
- `THUMBNAIL_CROP_STRATEGY`
- `THUMBNAIL_QUALITY`
- `THUMBNAIL_PRESERVE_ASPECT_RATIO`

These five `THUMBNAIL_*` processing vars are the defaults injected whenever a request or event
specification omits that field (`crates/common/src/generation.rs`); `THUMBNAIL_SIZES` instead
drives fan-out — when a request/event omits `specifications` entirely, the service generates one
thumbnail per configured size, not a single default size.

Optional: `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` may be set to the deployed thumbnail writer
(`task-worker`) principal recorded on storage events, for the storage-event recursion guard.
Currently inert in every environment — storage does not yet propagate the writer principal onto
CREATE events — so template-output shape matching carries the whole guard regardless of whether
this is set; see **Generated-output recursion prevention** below and
`docs/adr/0001-thumbnail-storage.md`. Not exposed as a Helm value (see ADR 0001); inject via
`thumbnailService.extraVars` if an environment needs it.

### Auth mode (exactly one mode must be enabled)

#### Client credentials mode

- `ENABLE_CLIENT_CREDENTIALS=true`
- `CLIENT_ID`
- `CLIENT_SECRET` (or `CLIENT_SECRET_FILE`, a path to a mounted secret file)
- `CLIENT_CREDENTIALS_OPENID_CONFIGURATION`
- Optional: `CLIENT_SCOPE`, `CLIENT_CREDENTIALS_EXTRA_HEADERS`, `CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS`

#### Access-token-file mode

- `ENABLE_ACCESS_TOKEN_FILE=true`
- `ACCESS_TOKEN_FILE_PATH`
- Optional: `ACCESS_TOKEN_FILE_POLL_INTERVAL` (humantime, for example `15s`)

Do not enable both auth modes at once.

### Listener ports

- HTTP server: `PORT` (default `3000`)
- gRPC server: `GRPC_PORT` (default `50051`)

### Optional event-consumer settings

- `DISABLE_EVENT_CONSUMPTION`
- `EVENT_CONSUMER_SERVER` (`HTTP`, `GRPC`, or `NONE`)
- `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT`
- `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` — comma-separated list of storage-container URL
  prefixes; required for both the assigned server consumer and `task-worker`;
  StorageEventQueue's and WorkerEventQueue's ids are each derived from this set by their own
  binary; see **Durable queue self-provisioning** below. Neither binary reads
  `NOTIFICATIONS_QUEUE_ID` — that variable no longer exists.
- `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT`
- `EVENT_PROCESSING_MAX_CONCURRENT`
- `STREAM_TIMEOUT_SECS`
- `MAX_RECONNECT_BACKOFF_SECONDS`
- `NOTIFICATIONS_PROVISIONING_MAX_RETRIES` (default `4`; transient retry attempts for durable-queue startup provisioning, `0` disables)
- `EVENT_TASK_SUBMISSION_MAX_RETRY_AGE`

### Durable queue self-provisioning

The assigned event consumer always creates its durable notifications queue on startup
(ensure-exists), so `scripts-notifications/create-durable-queue.sh` is no longer required before
deploy. This is not optional: whenever event consumption is enabled,
`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` is required, and a missing value (no non-empty prefix)
is a hard startup failure.

Two durable queues, two acknowledgement domains, two distinct ids:

- **StorageEventQueue** (`server`, storage-created/deleted events) — its id is not configured
  directly; `server` derives it at startup as `thumbnail-service-storage-<sha256 hex of
  "<sorted, deduplicated resourcePrefixes>">`.
- **WorkerEventQueue** (`task-worker`, `omni.thumbnail.generate` work) — its id is likewise not
  configured directly; `task-worker` derives it at startup as `thumbnail-service-worker-<sha256
  hex of "<sorted, deduplicated resourcePrefixes>">`. `NOTIFICATIONS_QUEUE_ID`
  no longer exists as a configuration variable for either binary.

Both ids share the `thumbnail-service` prefix (a hardcoded literal, not configurable) so they read
as belonging to this service in a Notification queue listing, and the `storage`/`worker` segment
keeps the two apart — that segment, not the hash, is what makes them distinct.

Because neither hash carries a queue-kind discriminator, **the two ids share one digest whenever
both components read the same `resourcePrefixes`**. That is the intended way to check them: in a
queue listing, a matching digest across the `-storage-` and `-worker-` pair means the two agree,
and differing digests mean one component's configuration is stale. A prefix mismatch is otherwise
silent — WorkerEventQueue filters `omni.thumbnail.generate` on the same source-address prefixes the
server watches, so a worker configured with a different set binds filters matching nothing and sits
idle, which looks exactly like no eligible files arriving.

**Kubernetes (normal path):** set `resourcePrefixes` under `notifications.eventConsumer` in your
environment values file (shared by both queues' filters and the input to both queues' derived
ids). See
[`helm/thumbnail-service/README.md`](helm/thumbnail-service/README.md#durable-queue-self-provisioning)
for the full guide (required fields, `resourcePrefixes` vs `thumbnailAddress`, startup behavior, and
permissions).

**Local dev / bare binary:** set `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` for both `server` and
`task-worker`. See `.env.example`.

`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES` is a comma-separated list of storage-container URL prefixes
that scopes which storage resources the queue subscribes to (`omni.storage.created` / `deleted`
under each prefix). Each prefix becomes one greedy-prefix filter on the single durable queue, so one
deployment can subscribe across multiple buckets/prefixes. Use the storage container root, not
`THUMBNAIL_PATH_TEMPLATE`. Blank delimited entries (`a,,b`) are rejected at parse time; Helm's YAML
list path instead trims and drops blank entries before joining — see
[`helm/thumbnail-service/README.md`](helm/thumbnail-service/README.md#durable-queue-self-provisioning).

## Thumbnail destination path template

`THUMBNAIL_PATH_TEMPLATE` replaces the legacy fixed `THUMBNAIL_ADDRESS` prefix: the service resolves
one exact, absolute output address per generated thumbnail from this template and sends it as
`destination_resource_address` on every generation request (event-driven, HTTP, and gRPC all go
through the same dispatch path in `crates/common/src/work_events/dispatch.rs`). `task-worker` writes
the actual blob but no longer derives or falls back to any address of its own — it writes exactly
where it's told.

Supported variables:

| Variable | Meaning |
|----------|---------|
| `{resource_path}` | Source directory path, with **no** trailing slash, e.g. `/assets/photos`. Write your own separator: `{resource_path}/.thumbs/` |
| `{resource_filename}` | Source filename with extension, e.g. `image.jpg` |
| `{resource_address_md5}` | MD5 hex digest of the full source `resource_address` |
| `{resource_identity_md5}` | MD5 hex digest of the source `resource_identity` |
| `{width}` / `{height}` | Requested thumbnail dimensions |
| `{output_encoding}` | Lowercased output encoding, e.g. `png` |
| `{quality}` | Requested encode quality |

The template must identify its source via either `{resource_address_md5}` alone, or both
`{resource_path}` and `{resource_filename}` together, and must render an absolute storage address —
the service fails fast at startup if the configured template can't satisfy these.

### Examples

**Centralized, content-hash-keyed** (dev's actual template) — every thumbnail lands under one
shared `thumbnails/` tree, keyed by a hash of the source address plus a hash of its identity, so
collisions across sources are avoided without touching the source container:

```text
https://examplestorage.blob.core.windows.net/example/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}
```

**Centralized, flat** — same idea, one level simpler (no `{resource_identity_md5}` segment;
`{resource_address_md5}` alone still uniquely identifies the source, matching `.env.example`):

```text
https://your-storage-account.blob.core.windows.net/your-container/thumbnails/{resource_address_md5}/{width}x{height}/{resource_filename}.auto.{output_encoding}
```

**Source-relative** — a sibling `_thumbnails` directory next to each source file, instead of one
shared tree, using `{resource_path}` + `{resource_filename}` (together) to identify the source
instead of an address hash:

```text
{resource_path}/_thumbnails/{width}x{height}/{resource_filename}.{output_encoding}
```

Templates are concatenated verbatim: every separator in the rendered address is one you
wrote. No variable contributes a slash of its own, so `{resource_path}/_thumbnails/`
renders one separator, `{resource_path}//_thumbnails/` renders the empty path segment you
asked for, and `{resource_path}_thumbnails/` renders a sibling directory whose name ends
in `_thumbnails` rather than a child directory.

**Invalid — no source identifier.** Rejected at startup with
`thumbnail path template must identify its source with '{resource_address_md5}' or both
'{resource_path}' and '{resource_filename}'`: every source of a given size would render to the
same path, silently overwriting each other's thumbnail.

```text
https://your-storage-account.blob.core.windows.net/your-container/{width}x{height}/thumbnail.{output_encoding}
```

**Invalid — not absolute.** Rejected at startup with `thumbnail path template must render an
absolute storage address`: a bare container-relative path (missing the `https://<account>/`
prefix) doesn't resolve to a real storage address.

```text
your-container/{resource_address_md5}/{width}x{height}.{output_encoding}
```
### Generated-output recursion prevention

A thumbnail write is itself a storage CREATE event; without safeguards it would be re-submitted for
generation, appending another `.auto.<ext>` on every pass. Two checks run in order, independent of
which layout is configured:

1. **Event markers / writer principal** — when `GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY` is set, it
   matches events carrying that principal, which after the Rust cutover is `task-worker`'s.
   Currently inert in dev: storage does not yet propagate the writer's principal onto CREATE
   events, so this check never fires and check 2 below is carrying the whole guard.
2. **Template-output shape matching** — skips CREATE events whose address matches
   `THUMBNAIL_PATH_TEMPLATE`'s rendered shape.

See `docs/adr/0001-thumbnail-storage.md` for full rationale.

## Local Development

### Prerequisites

- Rust toolchain (`cargo`, `rustfmt`, `clippy`)
- Docker (optional, for image validation)
- `helm` and `yamllint` (for chart/yaml checks)
- `buf` and `protoc` — only to regenerate protobuf code, which is committed. A normal
  `cargo build` needs neither, nor any network access. See
  [Regenerating protobuf code](#regenerating-protobuf-code).

### Setup

```bash
cp .env.example .env
```

Fill `.env` with environment-specific credentials and endpoints.

### Run services

Run directly:

```bash
cargo run --bin server
```

Or use helper scripts:

```bash
./start-native-services.sh start
./start-native-services.sh status
./start-native-services.sh stop
```

### Regenerating protobuf code

Generated protobuf code is committed, so a normal build compiles it as ordinary source:
`cargo build` invokes no `protoc` and contacts no schema registry, and a clean checkout
builds offline. Regeneration is a separate step you run on purpose when a schema changes.

```bash
# once
cargo install protoc-gen-prost@0.5.0 protoc-gen-prost-crate@0.5.0 protoc-gen-tonic@0.5.0

./scripts/buf-gen.sh                 # all four crates, then cargo fmt --all
./scripts/buf-gen.sh storage         # or just one
```

The plugin versions are pinned so anyone regenerating gets byte-identical output, which
makes checking for drift against upstream a diff rather than an investigation.

Each crate under `crates/proto/` has two hand-maintained files — `buf.gen.yaml`, which
says where the schema comes from, and `Cargo.toml.template`, which carries the package
metadata, dependencies, and lint allowances. Everything else in those crates is generator
output: `Cargo.toml`, `lib.rs`, and all of `src/**` are overwritten on every run, so
edits there do not survive. That is also why code outside these crates imports
fully-qualified `nvidia::omniverse::…` paths instead of shorter crate-root aliases, which
a regeneration would delete.

The thumbnail schema is generated from the in-repo `proto/` directory. The storage,
notifications, and discovery schemas are generated from their protos published in
this repository (`storage/api/proto`, `notifications/api/protos`, `discovery/api/proto`).

## Validation Commands

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
make yaml-check
make helm-lint
./scripts/pre-commit-check.sh
```

## Helm Deployment

Primary chart: `helm/thumbnail-service`.

Useful commands:

```bash
helm lint helm/thumbnail-service
helm template thumbnail-service helm/thumbnail-service
```

For deployment architecture and values guidance, see:

- `helm/DEPLOYMENT_ARCHITECTURE.md`
- `helm/thumbnail-service/README.md`

## Repo Map

- `crates/common`: shared domain logic — auth, storage client, work-event dispatch, notifications, error types
- `crates/service`: unified HTTP + gRPC service crate (`server` binary)
- `crates/task-worker`: the conversion worker; consumes generate work, converts, uploads, associates
- `crates/thumbnail-convert`: source bytes + resolved spec → thumbnail bytes (no transport dependencies)
- `crates/auth`: shared auth interceptor / OIDC discovery helpers
- `crates/proto/*`: generated protobuf/tonic crates (thumbnail, storage, notifications, discovery)
- `proto/`: protobuf source definitions
- `helm/thumbnail-service/`: Helm chart for both Deployments
- `docs/api/`, `docs/service/`: the publishable documentation sets — the public Thumbnail API specification and
  the operator guide for this reference implementation — as Markdown sources under each set's `docs/`, readable
  as-is. NVIDIA builds the rendered documentation site from them internally (same layout as `permission-service/docs/`)
- `docs/`: PRD, SDD, ADRs, and engineering/test/doc guides
- `openspec/`: capability specs and in-flight change proposals
- `scripts/`: dev and CI utility scripts
- `scripts-notifications/`: helper scripts for notification service integration

