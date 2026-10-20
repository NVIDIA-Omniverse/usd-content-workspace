(thumbnail-service-index-omniverse-storage-thumbnail-service)=
# Omniverse Storage Thumbnail Service

(thumbnail-service-index-overview)=
## Overview

This service is the **reference implementation** of the Omniverse Storage Thumbnail API — the runnable service that implements the published REST and gRPC contracts so other teams can validate integrations and ship compatible alternatives.

Thumbnail Service generates preview images for files stored in the Omniverse Storage service and records the
**association** between each source file and its thumbnails. It ships as two cooperating deployables from one
Helm chart:

- **Thumbnail Service** (the orchestrator) exposes the REST and gRPC APIs, consumes storage lifecycle events,
  decides *what* to generate and *where* each thumbnail is written, and is the single authority for association
  metadata.
- **Thumbnail task-worker** performs the image conversion: it consumes generation work from its own durable
  queue, downloads the source, converts it, uploads the thumbnail, and asks the orchestrator to record the
  association.

Both components authenticate outbound calls with one shared **service identity** issued by an OpenID Connect
identity provider, and both exchange work exclusively through durable queues on the Omniverse **Notification
Service**.

(thumbnail-service-index-table-of-contents)=
## Table of Contents

- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>`
- {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>`
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>`
- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>`
- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>`
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>`
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>`
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>`
- {ref}`Observability <thumbnail-service-observability-observability>`
- {ref}`Conformance and smoke tests <thumbnail-service-conformance-and-smoke-tests-conformance-and-smoke-tests>`
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>`

(thumbnail-service-index-use-cases)=
## Use Cases

- **Automatic previews** — a durable subscription to `omni.storage.created` events turns every eligible image
  landing in a watched storage container into one thumbnail per configured size, with no client call.
- **On-demand generation** — clients call `POST /v1alpha/thumbnails` or `CreateThumbnails` to request
  thumbnails for a specific source with their own specifications, and receive per-specification acceptance.
- **Registering pre-generated thumbnails** — a `MANUAL` specification lets a caller associate a thumbnail it
  produced itself, without any conversion.
- **Discovering previews** — `GET /v1alpha/thumbnails` and `GetThumbnails` list the thumbnails associated
  with a file or folder, filtered by size, format, or transform, so clients render previews without deriving
  storage paths.
- **Lifecycle hygiene** — when a source is deleted from storage, every association it carried is removed and
  the service-generated blobs are deleted; caller-owned thumbnails are left alone.

(thumbnail-service-index-architecture)=
## Architecture

The diagram below shows the two Deployments rendered by the Helm chart, the two durable Notification queues that
connect them, and the external services each one calls.

```{image} images/architecture.svg
:alt: Thumbnail Service and task-worker with the Storage and Notification services
```

(thumbnail-service-index-request-flow)=
### Request Flow

1. A file is created in a watched storage container and the Storage service publishes `omni.storage.created`.
   **Thumbnail Service** receives it from **StorageEventQueue** and checks eligibility: the address must name a
   file with a supported image extension, and it must not be a thumbnail the service itself generated.
2. Alternatively, a client calls `POST /v1alpha/thumbnails` (or gRPC `CreateThumbnails`) with a
   `resource_address`. The service validates the request, then resolves the source's `resource_identity` with a
   storage `Stat` using the **caller's bearer token**.
3. For every requested size (the caller's specifications, or one per configured `THUMBNAIL_SIZES` entry) the
   service resolves the exact output address from `THUMBNAIL_PATH_TEMPLATE` and publishes one
   `omni.thumbnail.generate` event. Acknowledging the storage event means the work was durably **published**,
   not that a thumbnail exists.
4. **Thumbnail task-worker** consumes the event from **WorkerEventQueue**, downloads the source, converts it
   inside configured memory bounds, uploads the thumbnail to exactly the address it was given, and calls
   `AssociateThumbnail` on the orchestrator over gRPC.
5. The orchestrator writes the association metadata to the Storage service — one key on the source, one on the
   thumbnail — and publishes `omni.thumbnail.associated` so downstream consumers can react.
6. Clients read associations back with `GET /v1alpha/thumbnails`; a later `omni.storage.deleted` event sweeps
   them away again.

For the eligibility rules and acknowledgement semantics of step 1 see
{ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>`; for steps 4–5 see
{ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` and {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>`.

(thumbnail-service-index-how-other-services-interact-with-thumbnail-service)=
### How Other Services Interact with Thumbnail Service

Calling services integrate through either API:

- **REST / JSON** — `POST` and `GET` on `/v1alpha/thumbnails` for generation and lookup, `POST` and `DELETE`
  on `/v1alpha/associations` for registering and removing associations. Interactive API documentation is
  bundled with the service at `/swagger-ui`.
- **gRPC** — `CreateThumbnails`, `GetThumbnails`, `AssociateThumbnail`, and `DisassociateThumbnail` on
  `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService`; reflection is available for clients that discover
  services dynamically.

Both APIs accept the same logical inputs and share one validation path, so a request that is legal on one
transport is legal on the other. Callers present an `Authorization: Bearer <token>` header (gRPC metadata for
the RPCs); the service checks that a token is present and forwards it to the Storage service, whose permission
check is the authorization decision.

(thumbnail-service-index-what-thumbnail-service-interacts-with)=
### What Thumbnail Service Interacts With

| External System | Role |
|---|---|
| **Storage service** | Source of truth for files and metadata. The orchestrator calls `Stat` and the metadata API (read, write, delete) to resolve identities and store associations; the worker reads source bytes and writes thumbnail blobs through the file-object API; the deletion sweep removes service-generated blobs. |
| **Notification Service — Event Consumer API** | Hosts the two durable queues. Each component provisions its own queue at startup (ensure-exists) and consumes with explicit acknowledgements; retryable failures are negatively acknowledged and redelivered. |
| **Notification Service — Event Aggregation API** | Receives every event the orchestrator publishes: `omni.thumbnail.generate` (consumed by the worker), `omni.thumbnail.associated`, and `omni.thumbnail.disassociated`. |
| **OpenID Connect Identity Provider** | Issues the bearer token both components present on outbound calls, either exchanged in-process with client credentials or provisioned into a shared file by a sidecar. See {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>`. |
| **Discovery operator** | Learns which APIs this deployment serves by calling the unauthenticated `CapabilitiesService/ListServices` RPC on the Service annotated `discovery.omniverse.nvidia.com/enabled: "true"`. |
| **OpenTelemetry** | Optional. Metrics, traces, and structured logs can be exported to your observability stack. See {ref}`Observability <thumbnail-service-observability-observability>`. |

(thumbnail-service-index-two-durable-queues-two-acknowledgement-domains)=
### Two durable queues, two acknowledgement domains

Thumbnail work crosses the Notification Service twice, on two separate durable queues with two distinct ids.
This is the single most important thing to understand before operating the service.

```{image} images/two-queues.svg
:alt: StorageEventQueue feeding the orchestrator and WorkerEventQueue feeding the task-worker
```

| | StorageEventQueue | WorkerEventQueue |
|---|---|---|
| Owner | orchestrator | task-worker |
| Event types | `omni.storage.created`, `omni.storage.deleted` | `omni.thumbnail.generate` |
| Queue id | `thumbnail-service-storage-<sha256 hex>` | `thumbnail-service-worker-<sha256 hex>` |
| Filters | one greedy prefix per watched container | one greedy prefix per watched container |
| An acknowledgement means | the generate work was durably **published** | the thumbnail was converted, uploaded, and **associated** |

Both ids hash the same sorted set of `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`, so for one configuration they end
in the **same digest** and differ only in the `-storage-` / `-worker-` label. Matching digests in a queue
listing mean the two components agree on their prefixes; differing digests mean one of them is stale. See
{ref}`Durable queue self-provisioning <thumbnail-service-event-consumption-durable-queue-self-provisioning>`.

(thumbnail-service-index-technologies)=
## Technologies

| Area | Overview |
|---|---|
| Implementation | Two cooperating processes (`task-worker` and the orchestrator), built for container deployment on a distroless base as a non-root user |
| APIs | HTTP REST with JSON (OpenAPI 3.1 and Swagger UI bundled), gRPC with reflection and the standard health service |
| Work distribution | Durable Notification Service queues with explicit acknowledgements and negative acknowledgements; no broker or database of its own |
| Image processing | Built-in decoders and encoders for JPEG, PNG, GIF, BMP, TIFF, ICO, and WebP input and JPEG, PNG, GIF, and WebP output, with memory bounds enforced before allocation |
| Durable state | None owned by the service — associations live as metadata in the Storage service, thumbnails as blobs |
| Observability | OpenTelemetry metrics and traces over OTLP, W3C trace context end to end, structured JSON logs; a Prometheus text endpoint on the worker |

```{toctree}
:maxdepth: 1
:caption: Contents:
deployment
configuration
authentication
destination-path-template
event-consumption
generation-pipeline
associations
notifications
observability
conformance-and-smoke-tests
troubleshooting
```

(thumbnail-service-index-see-also)=
## See also

The **Omniverse Storage Thumbnail API** specification (REST and gRPC) is documented separately; this service tracks that contract as its reference implementation.
