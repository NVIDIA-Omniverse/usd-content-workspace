# Thumbnail API

> **Applies to:** Thumbnail API `v1alpha` as served by the NVIDIA Thumbnail Service · **Released:** Thumbnail Service 0.1.0 (Developer Guide 1.3.0); the separate Thumbnail API specification document is internal and not in the released NGC API-resource inventory · **This repo's snapshot:** proto package `nvidia.omniverse.thumbnail.v1alpha`; OpenAPI `info.version: 0.1.0` · **Sources:** [`proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto`](../../../../thumbnails/reference/thumbnail-service/proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto), [`thumbnail.openapi.yaml`](../../../../thumbnails/reference/thumbnail-service/thumbnail.openapi.yaml), [`docs/api/`](../../../../thumbnails/reference/thumbnail-service/docs/api/), [service README](../../../../thumbnails/reference/thumbnail-service/README.md); adapted from the [source skill package](../../NOTICE.md)

The Thumbnail Service is released under Apache-2.0. Build generated clients from the `.proto` and OpenAPI files that match the deployed service version; a running service also serves its OpenAPI document at `/api-docs/openapi.json` and Swagger UI at `/swagger-ui`. Confirm the deployed version first ([capture-a-deployment](../versions/capture-a-deployment.md)); API version rules are in [versions/api-contracts.md](../versions/api-contracts.md).

Deployment: [deploy/thumbnails.md](../deploy/thumbnails.md). Building a different implementation: [develop/custom-thumbnail-implementation.md](../develop/custom-thumbnail-implementation.md).

## Model: generation and associations are separate

| Half | REST | gRPC | Purpose |
| --- | --- | --- | --- |
| Generation | `POST /v1alpha/thumbnails` | `CreateThumbnails` | Ask for thumbnails of a source file |
| Read associations | `GET /v1alpha/thumbnails` | `GetThumbnails` | List thumbnails linked to a file or folder |
| Associate | `POST /v1alpha/associations` | `AssociateThumbnail` | Record that an existing thumbnail belongs to a target |
| Disassociate | `DELETE /v1alpha/associations` | `DisassociateThumbnail` | Remove one association |

A client learns that a thumbnail exists only by reading associations. There is no task identifier, status endpoint, or polling resource; the task endpoints and `GetThumbnailTask`/`CancelThumbnailTask` were removed. Storage CREATE events also trigger generation with no client request, and reading associations is the only way to find those results.

The service README says gRPC has no equivalent of `GET /v1alpha/thumbnails`; the proto in the same snapshot defines `GetThumbnails` and the conformance and service code use it. Follow the proto.

## Completion mode

The Thumbnail API design (source skill package, which is not published in this repository) allows an implementation to complete generation synchronously (`200`, thumbnails readable at response time) or asynchronously (`202`). Callers must treat both as success and must not assume either.

The NVIDIA service in this snapshot is asynchronous only:

- REST answers `202 Accepted` with `{"failed": [...]}`. There is no `succeeded` array in this snapshot's OpenAPI or proto, although the API design describes one. Do not code against `succeeded` for this implementation.
- gRPC answers `OK` with `failed` populated only on partial dispatch failure.
- `failed` lists fully resolved specifications that could not be dispatched. Never compare `failed.length` with the request's `specifications.length`: an absent or empty list fans out over the configured `THUMBNAIL_SIZES`.
- A `502` (REST) or `INTERNAL` (gRPC) means nothing was dispatched or the source `Stat` failed. A publish that timed out may still have reached the broker, so read associations before retrying.

## Resources and targets

| Field | Rule |
| --- | --- |
| `resource_address` | Required, non-blank, `maxLength: 2048` |
| `resource_identity` | Opaque. Required for files, omitted for folders. Never used for lookup |
| `target_type` | `FILE` (default) or `FOLDER`; associations are keyed on the target address for both |

`CreateThumbnails` takes only `resource_address`; the service resolves the identity with a storage `Stat` (missing source → `404` / `NOT_FOUND`).

## Specifications

Processor vocabulary (`ProcessorIdentity`): `IMAGE_TO_IMAGE`, `VIDEO_TO_IMAGE`, `VIDEO_TO_VIDEO`, `USD_TO_IMAGE`, `USD_TO_VIDEO`. The `ThumbnailSpecification` oneof carries only `image_to_image` (field 1) and `manual` (field 6); fields 2–5 are reserved and must not be reused. The service generates `IMAGE_TO_IMAGE` only; other processors are rejected with `400` / `INVALID_ARGUMENT`. Video and USD processing are not available in this release; the source package describes them as deferred, not removed from the vocabulary.

`MANUAL` (width, height, encoding format) registers a caller-produced thumbnail and is accepted only by the associate operation.

### Two REST wire shapes

`POST /v1alpha/associations` takes a flat object tagged by `type`:

```json
{
  "thumbnail": {"resource_address": "https://acct.blob.core.windows.net/c/thumbs/a.png", "resource_identity": "<identity>"},
  "association_target": {"resource_address": "https://acct.blob.core.windows.net/c/photos/a.jpg", "resource_identity": "<identity>"},
  "target_type": "FILE",
  "specification": {"type": "IMAGE_TO_IMAGE", "width": 256, "height": 256, "encoding_format": "PNG",
                    "transform_type": "FIT", "crop_strategy": "CENTER", "quality": 85, "preserve_aspect_ratio": true}
}
```

`POST /v1alpha/thumbnails` takes an outer-keyed object with no `type`:

```json
{
  "resource_address": "https://acct.blob.core.windows.net/c/photos/a.jpg",
  "specifications": [
    {"image_to_image": {"width": 256, "height": 256, "encoding_format": "PNG", "transform_type": "FIT", "crop_strategy": "CENTER"}}
  ]
}
```

Over gRPC both use the `ThumbnailSpecification` oneof. `ImageToImageSpecification.processor_identity` (field 1) is required over gRPC (`UNSPECIFIED` is `INVALID_ARGUMENT`) and has no REST counterpart; set it to `PROCESSOR_IDENTITY_IMAGE_TO_IMAGE`.

### Enumerations

| Enum | Values |
| --- | --- |
| Image encoding | `PNG`, `JPEG`, `GIF`, `WEBP` |
| Video encoding (defined, carried by no message) | `MP4`, `ANIMATED_GIF`, `H264`, `H265`, `VP9`, `AV1` |
| `transform_type` | `SQUARE`, `SCALE_BY_WIDTH`, `SCALE_BY_HEIGHT`, `CROP`, `FIT`, `SCALE_AND_CROP`, `THUMBNAIL`, `RESIZE` |
| `crop_strategy` | `CENTER`, `TOP`, `BOTTOM`, `LEFT`, `RIGHT`, `TOP_LEFT`, `TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, `SMART` |
| `FrameSelectStrategy` (defined, unused) | `AUTO`, `TIMECODE` |
| `target_type` | `FILE`, `FOLDER` |

REST uses the bare names; gRPC uses prefixed enum constants (for example `TRANSFORM_TYPE_FIT`). `THUMBNAIL`, `RESIZE` and `FIT` are distinct transforms.

### Validation

| Rule | Violation |
| --- | --- |
| `width`, `height` in `1..=8192` | `400` / `INVALID_ARGUMENT`, before any storage call |
| `quality` in `1..=100` for `JPEG` and `WEBP`; not range-checked for `PNG`, `GIF` | `400` / `INVALID_ARGUMENT` |
| At most 1000 specifications per generation request | `400` / `INVALID_ARGUMENT` |
| Blank `resource_address` | `400` / `INVALID_ARGUMENT` |
| Non-`image_to_image` entry (including `manual`) in a generation request | `400` / `INVALID_ARGUMENT` |

One invalid specification rejects the whole request. Omitted `quality` and `preserve_aspect_ratio` take the deployment defaults (`THUMBNAIL_QUALITY`, `THUMBNAIL_PRESERVE_ASPECT_RATIO`) before anything is recorded, so the stored specification shows the values actually used. An explicit `false` is honored.

## REST endpoints

All `/v1alpha` routes require `Authorization: Bearer <token>`. `GET /health` (readiness; `503` until gRPC serves) and `GET /health/live` (liveness) are unauthenticated.

| Route | Success | Errors |
| --- | --- | --- |
| `POST /v1alpha/thumbnails` | `202` `{failed: []}` | `400`, `401`, `403` (cannot read source), `404` (source missing), `502` (Stat fault or nothing dispatched) |
| `GET /v1alpha/thumbnails` | `200` `{thumbnails: [...]}`, empty when none | `400` (unknown or unsupported parameter), `401`, `403`, `502` (metadata query failed) |
| `POST /v1alpha/associations` | `204`, no body | `400`, `401`, `500` |
| `DELETE /v1alpha/associations` | `200` `{succeeded, failed}` | `400`, `401`, `502` (removal failed or target not enumerable; body has the arrays) |

`GET` query parameters: `association_target_resource_address` (required) plus optional exact-match filters `specification` (`IMAGE_TO_IMAGE` only), `width`, `height`, `encoding_format`, `transform_type`, `crop_strategy`. Results are ordered by thumbnail address and capped at 1000 with no continuation token; a response of exactly 1000 may be incomplete, so narrow with filters. Unreadable stored entries are skipped. Each entry has `target_type`, `association_target`, `thumbnail`, `specification` and `create_time`.

`DELETE` query parameters: `association_target_resource_address` and `thumbnail_resource_address` (required), `target_type` (default `FILE`), and identity parameters that are accepted and ignored. Deleting something not associated returns `200` with both arrays empty. Disassociation removes metadata only; the thumbnail bytes remain.

Associating the same thumbnail, target and specification again replaces the entry. Treat any `2xx` as success.

Error body: `{"source": "...", "error_message": "...", "details": ...}`; `source` and `error_message` are required, messages are capped at 4096 characters, and clients must not parse `error_message`.

```bash
curl -sS -X POST -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/json' \
  -d '{"resource_address":"https://acct.blob.core.windows.net/c/photos/a.jpg"}' \
  https://thumbnails.example.com/v1alpha/thumbnails          # 202, default sizes
```

## gRPC service `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService`

Send `authorization: Bearer <token>` metadata on every call.

| RPC | Outcomes |
| --- | --- |
| `CreateThumbnails` | `OK` (partial failures in `failed`); `INVALID_ARGUMENT`; `NOT_FOUND`; `UNAUTHENTICATED`; `PERMISSION_DENIED`; `INTERNAL` (Stat fault or every dispatch failed) |
| `GetThumbnails` | `OK` (empty list when none); `INVALID_ARGUMENT`; `UNAUTHENTICATED`; `PERMISSION_DENIED`; `UNAVAILABLE` (retry) |
| `AssociateThumbnail` | `OK` with `success`; post-validation failures in-band as `success = false`, `error_message`, `failure_kind` (`RETRYABLE` / `PERMANENT`; absent or `UNSPECIFIED` means retryable); `UNAUTHENTICATED` |
| `DisassociateThumbnail` | `OK` with one entry in `succeeded`, or one in `failed`, or both empty (not associated); `UNAVAILABLE` when the target cannot be enumerated; `UNAUTHENTICATED` |

`AssociateThumbnailResponse.thumbnail` and `.association_target` are deprecated and never set. `GetThumbnailsThumbnailEntry.specification` is a `google.protobuf.Struct`, so numbers arrive as doubles (`256.0`); REST returns integers. `create_time` is a string as stored. Retrying `UNAUTHENTICATED` needs a new token.

## Authentication and authorization

The service rejects calls without a bearer token before parsing the body. Token validation and access decisions are delegated to Storage: the caller's token is used for the source `Stat` and metadata reads, so `401` means Storage rejected the token and `403` means Storage denied access (OpenAPI `bearer_auth` description). The associate and disassociate routes document no `403`; how a Storage denial during their metadata write surfaces (`500`/`502` over REST, in-band over gRPC) is unverified in this snapshot, so test it against the deployment. Do not forward caller tokens to other hosts.

Source deletion: on `omni.storage.deleted` the orchestrator removes every association of the deleted source, publishes `omni.thumbnail.disassociated`, and deletes only thumbnails it generated (`IMAGE_TO_IMAGE` or template-shaped). Caller-registered (`MANUAL`) bytes survive, so a caller cannot use association metadata to delete objects it does not own (`crates/common/tests/disassociate_all_caller_owned_blob.rs`).

## Discovery and change notifications

`nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` is served unauthenticated on the gRPC port and advertises type `thumbnail`, the gRPC service above, and REST endpoint `thumbnail` version `v1alpha` path `/v1alpha/` ([apis/discovery.md](discovery.md)).

The service publishes `omni.thumbnail.associated` and `omni.thumbnail.disassociated` through Event Aggregation after the change is readable. Delivery is best-effort and does not affect the response; the read API is authoritative ([apis/notifications.md](notifications.md)).
