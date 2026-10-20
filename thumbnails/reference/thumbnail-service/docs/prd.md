# Thumbnails API — Product Requirements

> **Scope note.** The Thumbnails API is built from two sibling repositories under
> `api-thumbnails/`: **`thumbnail-service`** (Rust — the dual-protocol API plus the `task-worker`
> conversion binary) and **`thumbnail-dev-utils`** (Bash local-stack orchestration).
> **`thumbnail-task-queue`** (Python/Celery) was retired when conversion moved into the Rust
> workspace. This PRD describes the API as a whole. All quorum specs, plans, and bee tickets live
> in `thumbnail-service`.

## Existing scope

The Thumbnails API automatically generates preview thumbnails for files added to the Omniverse
storage service and records the association between each source file and its generated thumbnail(s),
so that developers integrating with the storage-service stack can display previews of user content.

It is a **developer-facing, proprietary API** that ships as part of the larger USD storage-service
stack. NVIDIA does not host it as a SaaS product; instead the team ships container images, Helm
charts, and documentation to customers who deploy the stack into their own clusters.

Primary capabilities today:
- **Automatic thumbnail generation** — a durable subscription to storage events (`omni.storage.*`)
  triggers thumbnail generation when qualifying files are added, with no explicit client call
  required.
- **On-demand thumbnail generation** — clients may submit thumbnail-generation tasks explicitly over
  HTTP (REST) or gRPC and poll for status.
- **Thumbnail associations** — the API is the authority for the metadata link between a source file
  and its generated thumbnail(s); clients can query, associate, and disassociate thumbnails.
- **Raster image processing** — the storage-event filter accepts a broad set of image extensions
  (jpg, jpeg, png, gif, bmp, tiff/tif, webp, svg, ico, heic); generation is raster image-to-image via
  Pillow, applying a processing specification (square, scale-by-width/height, crop, fit,
  scale-and-crop) and producing **one or more configured sizes** per source, uploaded to blob storage.

## Why

Users of the storage service want to see visual previews of the files they add. Rendering those
previews on demand in every client is wasteful and inconsistent; instead the Thumbnails API
generates previews once, asynchronously, as content lands in storage, and makes the resulting
thumbnail associations queryable through a stable API. This offloads image processing from clients
and keeps thumbnail generation consistent across every consumer of the storage-service stack.

## Audience

Developers integrating with the Omniverse storage-service stack. The API surface (REST + gRPC) is
the product; end users experience it indirectly through whatever client renders the previews.

## Out of scope

- **USD and video thumbnailing** — the service's type system anticipates UsdToImage, UsdToVideo,
  VideoToImage, and VideoToVideo specifications, but the current generation pipeline is raster
  image-to-image only. USD/video thumbnail generation is deliberately out of scope for now.
- **Hosting as a SaaS** — the API is shipped to customers to self-deploy, not operated as a hosted
  service by NVIDIA.
- **Client-side rendering / preview UI** — the API produces and associates thumbnails; presenting
  them is the consumer's responsibility.

## Acceptance criteria (project-level)

The Thumbnails API is considered working correctly when:
- The full test suites and linters across both repos run green (`cargo build --workspace`,
  `cargo test --workspace`, and clippy/fmt for `thumbnail-service`; shellcheck/`bash -n`
  for `thumbnail-dev-utils`).
- The integration tests pass against the dev cluster, whose deployment is defined in the
  `usd-storage-api-deployment` repository (`environments/dev/`).
- A file added to the storage service results in an automatically generated thumbnail and a queryable
  source-to-thumbnail association.

## Maturity

Pre-production, approaching first production release. Actively developed; not yet shipped to
customers in the storage-service stack.

## Per-feature scope

_Each `/quo-plan` invocation that produces docs adds a `### Feature: <title>` subsection here._

### Feature: Durable event queue ensure-exists on startup

The service self-provisions its durable event-consumer queue on startup. The assigned consumer
ensures its durable notifications queue exists before attaching, so operators no longer need to
pre-create the queue out-of-band (`create-durable-queue.sh`) before deploying.

- **Always on for the assigned consumer.** Self-provisioning is not optional: whenever event
  consumption is enabled, the consumer ensures its queue exists on startup. Operators configure it by
  setting `queueId` and `resourcePrefixes` in their environment values file. See
  `helm/thumbnail-service/README.md`.
- **Multi-prefix scoping.** Operators can scope the durable queue to one or more
  storage-container URL prefixes so a single deployment auto-generates thumbnails for storage
  events across multiple buckets/prefixes. At least one prefix is required whenever event
  consumption is enabled; absence is a hard failure.
- **Error-handling expectations.** Insufficient permission to create the queue degrades gracefully —
  the service warns and then attaches to the queue rather than failing. An invalid request is a hard
  startup failure (fail fast) rather than a degraded-but-running state.

Scope is limited to ensure-exists on startup: the service does not delete queues or otherwise manage
their lifecycle beyond ensuring they exist.
