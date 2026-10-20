(thumbnail-service-associations-association-metadata-and-queries)=
# Association metadata and queries

Thumbnail Service is the **authority for the link between a source resource and its thumbnails**. It keeps that link as user metadata in the storage service — there is no database of its own — and exposes it through `AssociateThumbnail`, `DisassociateThumbnail`, and `GetThumbnails` on both transports. This page describes what is written, how the metadata key is derived, how the two writes are kept consistent, what each operation validates and returns, how the storage-deleted sweep differs from the public disassociate, which blobs the service deletes, and how the read path filters and caps its results.

For the request and response shapes on the wire, see the Omniverse Storage Thumbnail API specification (documented separately). For the events each write publishes, see {ref}`Service notifications <thumbnail-service-notifications-service-notifications>`.

(thumbnail-service-associations-metadata-model)=
## Metadata model

An association is **two metadata writes carrying the same JSON value**, both keyed on a **resource address** (never on an identity):

| Side | Written on | Key | Purpose |
|---|---|---|---|
| Association-target side | `association_target.resource_address` | `thumbnail_<W>x<H>_<FORMAT>_<suffix>` | Enumerated by `GetThumbnails` and by disassociation to rediscover the thumbnail |
| Thumbnail side | `thumbnail.resource_address` | `thumbnail_metadata` | Records that the thumbnail resource is associated |

The value, identical on both sides:

```json
{
  "thumbnail": {
    "resource_identity": "fileobject:thumb-abc123:v7",
    "resource_address": "https://acct.blob.core.windows.net/thumbnails/475673a3…/256x256/image.jpg.auto.png"
  },
  "association_target": {
    "resource_address": "https://acct.blob.core.windows.net/assets/photos/image.jpg",
    "resource_identity": "fileobject:src-def456:v3"
  },
  "target_type": "FILE",
  "specification": {
    "type": "IMAGE_TO_IMAGE",
    "width": 256,
    "height": 256,
    "encoding_format": "PNG",
    "transform_type": "FIT",
    "crop_strategy": "SMART",
    "quality": 85.0,
    "preserve_aspect_ratio": true
  },
  "create_time": "2026-09-11T12:00:00Z"
}
```

| Field | Notes |
|---|---|
| `thumbnail.resource_identity` | Always present; thumbnails are files. Empty string when unknown. |
| `association_target.resource_identity` | Present for `FILE` targets; **omitted entirely** (not `null`) for `FOLDER` targets, which have no identity. |
| `target_type` | `FILE` or `FOLDER`. Stored entries lacking it are read as the operation's own kind, defaulting to `FILE`. |
| `specification` | Flat, `type`-tagged, **normalized**: an omitted `quality` or `preserve_aspect_ratio` is filled from `THUMBNAIL_QUALITY` / `THUMBNAIL_PRESERVE_ASPECT_RATIO` once, in the shared store path, so HTTP and gRPC produce byte-identical keys, metadata, and events. A `MANUAL` specification carries only `type`, `width`, `height`, and `encoding_format`. |
| `create_time` | RFC 3339, the service clock at write time. |

Enum values are UPPER_SNAKE_CASE throughout. Numbers are stored as protobuf doubles and rendered back as JSON integers on read when they are whole (`256`, not `256.0`); `quality` keeps its numeric form. Every resource address is **canonicalized** before any storage call — parsed as a URL and re-serialized, which percent-encodes raw spaces and Unicode in the path, with `[`, `]`, `^`, `|`, and `\` additionally encoded — so raw inputs from events and callers converge on one key.

(thumbnail-service-associations-deterministic-key)=
## Deterministic key

`<FORMAT>` in the target-side key is the encoding's display name (`PNG`, `JPEG`, `GIF`, `WEBP`). `<suffix>` is the first 12 bytes (24 lowercase hex characters) of a SHA-256 over, in order: the target address, the thumbnail `resource_identity`, `width`, `height`, the format, the specification type (`IMAGE_TO_IMAGE` or `MANUAL`, so the two kinds never collide), and — for `IMAGE_TO_IMAGE` — the transform, crop strategy, resolved quality, and resolved `preserve_aspect_ratio`.

The key is therefore a pure function of the association: re-associating the same thumbnail with the same specification **overwrites** the same key rather than creating a duplicate, which is what lets the generation worker's redeliveries and concurrent replicas converge on one consistent association without a lock. An integral quality written as `85` or `85.0` hashes identically.

(thumbnail-service-associations-two-phase-write-and-rollback)=
## Two-phase write and rollback

Storage cannot make the two writes atomic, and reads are not symmetric — `GetThumbnails` resolves association state from the target-side key alone — so a failure between the writes would otherwise be visible as a successful association with no thumbnail-side metadata behind it. `AssociateThumbnail` proceeds as follows:

1. Canonicalize both addresses, validate the request, normalize the specification, and resolve the identities the message needs.
2. Write the **association-target-side** key first. If it fails, nothing else is written and the failure is returned.
3. Write the thumbnail-side `thumbnail_metadata`. If it fails, the service **rolls back** the target-side key it just wrote, then returns the **original** write failure so the caller's retry decision is driven by the cause that matters. A rollback that finds the key already absent counts as a successful rollback.
4. On success, spawn one `omni.thumbnail.associated` publish off the request path.

Two gaps are accepted knowingly. Between the first write landing and the rollback completing, a concurrent read can still observe the association. And if the rollback itself fails, both failures are logged at `error` with the resource address and key needed to reconcile by hand, and the returned error names both while leading with the write failure (`… (and the association-target-side key '<key>' on <address> could not be rolled back: …)`).

The write order is deliberate: writing the thumbnail side first would remove the false-visible association but leave a `thumbnail_metadata` blob that neither the targeted disassociate nor the storage-deleted sweep could ever find, since both discover thumbnails by enumerating `thumbnail_*` keys on the target.

(thumbnail-service-associations-validation-rules)=
## Validation rules

Both transports validate the same canonical request before any storage call; the store path validates again as defense in depth. Order: thumbnail resource, target resource, specification presence, dimensions, quality. Each rule rejects with `400 Bad Request` / `INVALID_ARGUMENT` and a distinct, matchable message; the full catalog is in the REST API and gRPC specifications' error conditions for this endpoint.

`target_type` defaults to `FILE` when omitted on either transport. Over gRPC an `UNSPECIFIED` or out-of-range enum is `INVALID_ARGUMENT` naming the field (for example `specification.image_to_image.encoding_format must not be UNSPECIFIED`); over HTTP a JSON syntax or typed deserialization failure is `400` with the documented `ErrorResponse`, never `422`.

On success this returns `204 No Content` over HTTP / `success: true` over gRPC. A storage failure after
validation is reported through the standard status/error-code contract for each transport, documented in the
REST API and gRPC specifications. A failed rollback (see Two-phase write and rollback above) is reported the
same way as any other post-validation storage failure — the caller cannot distinguish the two.

The `failure_kind` field exists so the generation worker can decide acknowledgements without matching on prose. Every HTTP error body is `{"source": "thumbnail-service", "error_message": "…", "details": null}`.

(thumbnail-service-associations-targeted-disassociation)=
## Targeted disassociation

`DELETE /v1alpha/associations` and gRPC `DisassociateThumbnail` remove **one** thumbnail from one association target. Both addresses are required and non-blank; `target_type` defaults to `FILE`; resource identities are optional on gRPC and are **ignored** — only addresses locate entries.

1. Canonicalize and validate (a blank address → `400` / `INVALID_ARGUMENT`, before any storage call; a missing token → `401` / `UNAUTHENTICATED`).
2. Enumerate the target's metadata and find the entry whose stored `thumbnail.resource_address` equals the requested thumbnail address.
3. Delete **only the target-side key**. The thumbnail resource, its `thumbnail_metadata`, and its blob are left intact — the caller owns the thumbnail's lifecycle on this path. `NOT_FOUND` from storage counts as success.
4. Publish one `omni.thumbnail.disassociated` event.

Outcomes (removed or already-unassociated success, enumeration failure, a rejected token) follow the standard
status/error-code contract for each transport, documented in the REST API and gRPC specifications. One
exception is worth calling out: when the removal itself fails after the entry is found, REST returns `502`
with `failed` naming the thumbnail and error, while gRPC returns `OK` with one `failed` entry instead — an
error status cannot carry a body, so gRPC reports the same failure in-band.

(thumbnail-service-associations-storage-deleted-sweep)=
## Storage-deleted sweep

When storage deletes a source, the `omni.storage.deleted` handler removes **every** association recorded on it (see {ref}`Storage deleted cleanup <thumbnail-service-event-consumption-storage-deleted-cleanup>`). The sweep is internal — clients cannot trigger it — and runs under the **service identity**.

1. Enumerate every `thumbnail_*` key on the deleted address. An entry with no stored thumbnail address is a permanent `MalformedEntry` failure, logged and never retried.
2. **Phase 1** — one `DeleteMetadataBatch` removing `thumbnail_metadata` from every **distinct** thumbnail address. Addresses are de-duplicated first: several entries can name the same thumbnail, and storage rejects a duplicate in one batch as a permanent "duplicate batch entry" error. A per-entry `NOT_FOUND` counts as success.
3. **Phase 2** — one `DeleteMetadataBatch` removing the target-side keys, **only for phase-1 successes**, so a key stays discoverable for a later sweep whenever its thumbnail side failed.
4. For every fully removed entry, publish `omni.thumbnail.disassociated` and apply the {ref}`blob deletion policy <thumbnail-service-associations-blob-deletion-policy>`.
5. Log a summary: `[OK] Disassociate sweep on <target> complete: N succeeded, M failed, K blobs left as caller-owned`.

Retryable per-entry failures leave the storage delivery negatively acknowledged; a run whose every failure is permanent is acknowledged with an error-level summary.

(thumbnail-service-associations-blob-deletion-policy)=
## Blob deletion policy

Only the sweep deletes blobs; targeted disassociation never does. A blob is deleted only when it is **positively identifiable as service-generated** — when the stored specification's `type` is `IMAGE_TO_IMAGE` **or** the thumbnail address matches the rendered shape of `THUMBNAIL_PATH_TEMPLATE`. The rule is a disjunction so that changing the template does not orphan thumbnails generated under the previous one.

A `MANUAL` association whose address does not match the template is **caller-owned**: its metadata is removed, the blob is left in storage, and the entry is logged at `info` (`Disassociate sweep: leaving '<address>' in storage — … treated as caller-owned. Metadata was still removed.`). This is what prevents a caller with metadata-write rights from directing a service-identity delete at an arbitrary address.

Deletion is **best-effort** and runs after the metadata result is determined, so it can never change a result the caller already received. Each `FileObjectService.Delete` carries the storage request deadline and refreshes the service token once on `UNAUTHENTICATED`; `NOT_FOUND` counts as deleted. Any other failure leaves an **orphan** — a blob nothing will enumerate again — which is logged at `error` with its address (`Generated thumbnail blob deletion failed after its metadata was removed; the blob is orphaned …`). The service identity is used because generated blobs are written under it and a caller may lack delete rights on the thumbnail path.

(thumbnail-service-associations-querying-thumbnails)=
## Querying thumbnails

`GET /v1alpha/thumbnails` and gRPC `GetThumbnails` read the association target's metadata **by address**, for `FILE` and `FOLDER` targets alike — no `Stat`, no identity resolution — so reads use the same key the writes use.

**Inputs.** `association_target_resource_address` (required, non-blank, at most 2048 characters) and optional exact-match filters: `specification` (`IMAGE_TO_IMAGE` is the only supported value), `width`, `height` (each in `1..=8192`), `encoding_format`, `transform_type`, `crop_strategy`. An unknown query parameter or unsupported enum value is `400` / `INVALID_ARGUMENT`; omitted filters do not narrow.

**Pipeline.** Fetch all user metadata on the canonical address; keep keys beginning `thumbnail_`; parse `thumbnail_<W>x<H>_<FORMAT>_<suffix>` (an unparseable key is logged and skipped); filter on the key's dimensions and format, then on the stored specification's `type`, `transform_type`, and `crop_strategy` (case-insensitive; an entry without a specification fails any specification filter); extract the entry. An entry with no usable thumbnail address is **skipped with a warning** naming its key, and a summary warning reports how many were skipped — one legacy or half-written record never hides the other thumbnails on the same target. An entry whose specification is not a JSON object is returned without its specification over gRPC (a `Struct` can carry only an object).

**Result.** Entries are **sorted by `thumbnail.resource_address`** and **truncated to 1,000**. There is no pagination and no continuation token: a target with more than 1,000 thumbnails has the remainder discarded, so a response holding exactly 1,000 entries may be incomplete, and callers needing more must narrow with filters. Each entry carries `target_type` (absent or unknown stored values render as `FILE`), the `association_target` (the requested address plus the stored identity), the `thumbnail` resource, the stored `specification` verbatim, and `create_time`.

A target with no thumbnail metadata, or none matching the filters, is a successful read of nothing: `200`/`OK`
with an empty list, never `404`/`NOT_FOUND`. Other failures (a rejected token, denied access, or an unavailable
storage backend) are reported through the standard status/error-code contract documented in the REST API and
gRPC specifications.

```{warning}
`DEVELOPMENT_MODE=true` makes the read path return one fabricated entry **without calling storage**. Because storage's permission check is the only authorization the read path has, this bypasses authorization entirely: any bearer-shaped token receives `200`. It is a local-development flag with no Helm value; never set it in a deployed environment.
```

(thumbnail-service-associations-which-credential-is-used)=
## Which credential is used

| Operation | Credential |
|---|---|
| `AssociateThumbnail`, `DisassociateThumbnail`, `GetThumbnails` metadata reads and writes | The **caller's** bearer token, forwarded to storage, whose permission check is the authorization decision |
| Storage `Stat` on `CreateThumbnails` | The caller's bearer token |
| Storage-deleted sweep (enumeration and both delete phases) | The **service identity** |
| Blob deletion in the sweep | The service identity |
| `omni.thumbnail.associated` / `omni.thumbnail.disassociated` publishes | The service identity |

Metadata reads and writes on the caller-token paths retry `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED`, and `ABORTED` up to `STORAGE_METADATA_MAX_RETRIES` (default `3`) with backoff from `STORAGE_METADATA_INITIAL_DELAY` (`200ms`) doubling to `STORAGE_METADATA_MAX_DELAY` (`2s`), under the channel deadline `STORAGE_GRPC_REQUEST_TIMEOUT` (`30s`). Batch deletes and `Stat` are not retried at this layer. See {ref}`Which token is used for which call <thumbnail-service-authentication-which-token-is-used-for-which-call>` for the complete matrix.

(thumbnail-service-associations-see-also)=
## See also

- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the caller of `AssociateThumbnail` after every upload, and how `failure_kind` drives its acknowledgements.
- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — the `omni.storage.deleted` handler that runs the sweep.
- {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>` — the template whose rendered shape identifies service-generated blobs.
- {ref}`Service notifications <thumbnail-service-notifications-service-notifications>` — the `omni.thumbnail.associated` and `omni.thumbnail.disassociated` events these operations publish.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — caller tokens versus the service identity.
- {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>` — the storage channel, retry, and `THUMBNAIL_*` default variables named here.
