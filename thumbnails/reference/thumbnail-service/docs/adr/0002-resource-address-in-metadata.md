# Resource address in association metadata, events, and APIs

## Status

proposal

## Context

The `omni.storage.created` event now carries both `resource_identity` and `resource_address` for each storage object. Today the thumbnail service consumes only `resource_identity` from the event, forwards only identity through the task-queue, and stores only identity in association metadata. The address is dropped on the floor in three places:

1. `notifications::event_processing::handle_create_event` extracts identity and ignores address.
2. `task_queue::submit_task_to_queue` builds `TaskQueueSource { resource_identity: Some(...), resource_address: None }`.
3. `storage::association::build_association_metadata_writes` emits a metadata value containing only identity fields.

This forces every downstream consumer of stored metadata or `omni.thumbnail.*` events to round-trip identity → address via storage `Stat` to obtain a human-readable URL, even though the original event already had it.

The change also surfaces wider shape inconsistencies that have accumulated in the same surface area:

- HTTP `AssociateThumbnailRequest` uses a nested `ResourceRef` for the association target but a separate URL-path identity for the thumbnail.
- gRPC `AssociateThumbnailRequest` is flat (`source_resource_identity`, `thumbnail_resource_identity`) — different from HTTP.
- `omni.thumbnail.*` event payloads are flat and overload one field (`association_target`) to mean either an identity (for files) or an address (for folders).
- `CreateThumbnailsRequest.ResourceTarget.source_file` is a bare string while its sibling `association_target` is a nested object.
- Folders, which have no concept of resource identity in storage, are represented inconsistently across these surfaces.

Disassociate is the other affected surface. It removes one specific source ↔ thumbnail association, but its transports represented that target inconsistently. The canonical request should carry the association-target address and thumbnail address explicitly, with no identity fields: addresses locate both metadata records, while identities cannot narrow the removal. Broad cleanup remains a separate internal operation used when an `omni.storage.deleted` event requires every thumbnail for a deleted source to be removed.

The storage `MetadataService.uri` field — used as the key for `GetMetadata` / `UpdateMetadata` / `DeleteMetadata` — is natively a resource address. Today's code passes values labelled `resource_identity` into that field, but those values are already address-shaped strings; the parameter name is the only thing wrong.

Glossary terms used in this ADR are defined where they are enforced, not in a separate document: `Resource` and `TargetType` in `proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto`, the folder exception (files carry a `resource_identity`, folders do not) in `crates/common/src/resource.rs`, and "association target" in §1 below. An earlier draft pointed at a workspace-root `CONTEXT.md`, which does not exist — and could not be version-controlled with this service if it did, since the workspace root only holds the two repositories side by side.

## Decision

### 1. Unified `Resource` shape

A `Resource` has:

- `resource_address` — required, non-empty.
- `resource_identity` — optional; required when the resource is a file, omitted when the resource is a folder.

This single type replaces the existing `ResourceRef` (HTTP), the implicit flat-field pairs in gRPC, and the bare-string `source_file` in `CreateThumbnailsRequest`.

### 2. Stored association metadata value (both source and thumbnail sides)

```json
{
  "thumbnail": { "resource_identity": "...", "resource_address": "..." },
  "association_target": { "resource_identity": "...", "resource_address": "..." },
  "target_type": "FILE",
  "specification": { "image_to_image": { ... } }
}
```

For `target_type: "FOLDER"`, `association_target.resource_identity` is omitted from the JSON entirely (not `null`, not empty string).

The old top-level `thumbnail_resource_identity` field and the overloaded `association_target` string are removed.

### 3. Event payloads migrate in lockstep

`omni.thumbnail.created`, `omni.thumbnail.associated`, and `omni.thumbnail.disassociated` all use the same `event.message` shape as stored metadata. `omni.thumbnail.created` additionally carries a top-level `task_id` (string) at the same level as `thumbnail`. `omni.thumbnail.associated` and `omni.thumbnail.disassociated` do not have `task_id`.

`event.resource.resource_id` is the source `association_target.resource_address` for `omni.thumbnail.associated`, so SSE consumers can filter by source resource. Other `omni.thumbnail.*` events continue to use thumbnail identity where applicable.

### 4. Storage metadata is address-keyed

Parameters named `resource_identity` in `storage::metadata::{get,update,remove}_storage_metadata` are renamed to `resource_address`. This is a clarity sweep; the values passed at every call site were already addresses.

Storage layout decisions — the source-side metadata key suffix, thumbnail destination path derivation, and related concerns — are out of scope here. See the storage ADR in 'docs/ADR/0001-thumbnail-storage.md'

### 4a. A failed association rolls back its first write

An association is two metadata writes that storage cannot make atomic: the association-target-side key first, then the thumbnail-side `thumbnail_metadata` blob. Reads are not symmetric — `GET /thumbnails` resolves association state from the target-side key alone — so a failure between the two writes is visible as a *successful* association whose thumbnail-side metadata was never written.

When the thumbnail-side write fails, the service removes the association-target-side key it just wrote, then returns the failure. A rollback that finds the key already absent counts as successful. The caller receives the original write failure, so retry behavior is driven by the cause that matters.

The write order is deliberate and stays as it is. Writing the thumbnail side first would remove the false-visible association, but it trades one leak for a worse one: both targeted disassociation and storage-deletion cleanup discover thumbnails by enumerating `thumbnail_*` keys on the association target (§5). A `thumbnail_metadata` blob with no target-side key pointing at it cannot be found by either path. The current order leaves a discoverable — therefore reclaimable — artifact if the rollback fails.

Two gaps are accepted knowingly. Between the first write landing and the rollback completing, a concurrent read can still observe the association. And if the rollback itself fails, the end state is the pre-rollback behavior; both failures are then logged with the resource address and key needed to reconcile by hand, and the returned error names both.

### 5. HTTP API

**Associate** — `POST /v1alpha/associations` with body:

```json
{
  "target_type": "FILE" | "FOLDER",
  "thumbnail": { "resource_address": "...", "resource_identity": "..." },
  "association_target": { "resource_address": "...", "resource_identity": "..." },
  "specification": { ... }
}
```

The `PUT /v1alpha/thumbnails/{thumbnail_resource_identity}` route is **deleted from the router** (no 410/404 stub — framework's default `404` applies).

**Disassociate** — `DELETE /v1alpha/associations?association_target_resource_address=<target>&thumbnail_resource_address=<thumbnail>[&target_type=<FILE|FOLDER>]`

Both addresses are required. `target_type` is optional and defaults to `FILE`. No `resource_identity` is required or used for either resource: the association-target address selects the metadata collection, and the thumbnail address selects the single stored entry to remove. Both transports convert into one shared `DisassociateRequest` and validate through its single `validate()`; identities carried by gRPC `Resource` messages are dropped at the transport boundary.

Targeted semantics: the service enumerates the target's `thumbnail_*` metadata keys, finds the one whose stored `thumbnail.resource_address` matches `thumbnail_resource_address`, and leaves all other entries unchanged. No match is an idempotent success with empty result arrays.

Removal uses two ordered `DeleteMetadata` calls: first the thumbnail-side `thumbnail_metadata` key, then the matching source-side key. The source-side key remains discoverable until phase one succeeds. `NOT_FOUND` from either phase counts as success, so a retry after a partial removal safely completes the operation. After both phases succeed, the service emits one best-effort `omni.thumbnail.disassociated` event and asks the task queue to delete the thumbnail blob; failure of either side effect is logged without changing the completed metadata result.

Response body (always JSON):

```json
{
  "succeeded": [
    { "resource_address": "...", "resource_identity": "..." }
  ],
  "failed": [
    { "resource_address": "...", "resource_identity": "...", "error": "<message>" }
  ]
}
```

Each array contains at most one entry. A completed request places the targeted thumbnail in either `succeeded` or `failed`; both arrays are empty when no matching association exists.

Status logic:

- `200 OK` — the thumbnail was removed, or no matching association exists.
- `502 Bad Gateway` — association metadata could not be enumerated, or one of the two metadata removals failed. `failed` identifies the thumbnail when a removal was attempted; both arrays are empty when enumeration failed.
- `401 Unauthorized` — no Bearer token was provided, or storage rejected it. Nothing was removed.
- `400 Bad Request` — either address is missing or blank, or `target_type` is present but unparseable. Validation runs before any storage call.

Disassociate fails fast on a missing token exactly as associate does.

**gRPC diverges deliberately for attempted-removal failures.** An error `Status` carries no response body, so `DisassociateThumbnail` returns `OK` with one `failed` entry when either metadata removal fails. Enumeration failure has no targeted result and returns `UNAVAILABLE`; missing or rejected credentials return `UNAUTHENTICATED`; invalid input returns `INVALID_ARGUMENT`. A thumbnail that is not associated returns `OK` with both arrays empty.

The old query-string `DELETE /v1alpha/thumbnails/{id}` disassociate path is deleted from the router.

**CreateThumbnails** — `POST /v1alpha/thumbnails`. `ResourceTarget.source_file: String` becomes `ResourceTarget.source: Resource`. Per-item `target_type` and `association_target` remain.

### 6. gRPC proto

Add to `thumbnail-service/proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto`:

```proto
// A storage resource (file or folder) identified by its address.
// resource_identity is omitted for folders, which have no identity.
message Resource {
  string resource_address = 1;
  optional string resource_identity = 2;
}
```

Replace `AssociateThumbnailRequest`:

```proto
message AssociateThumbnailRequest {
  Resource thumbnail = 1;
  Resource association_target = 2;
  optional TargetType target_type = 3;  // defaults to FILE
  // Presence is required by runtime validation (proto3 has no `required` message field).
  optional ThumbnailSpecification specification = 4;
}
```

For `target_type: FOLDER`, `association_target.resource_identity` is omitted, exactly as on the HTTP side. gRPC is at full parity with HTTP and supports both FILE and FOLDER associations.

`AssociateThumbnailResponse` reports only the outcome: `bool success` and `optional string error_message`. Its two `Resource` fields echoed the request back, which told a caller nothing it had not just sent, and no caller read them; they remain in the message marked deprecated so existing clients keep parsing, but nothing populates them. HTTP does the same thing more directly by answering `204 No Content`. Either way, a caller that wants the stored association reads it back from `GetThumbnails` / `GET /v1alpha/thumbnails`, where it gets the normalized specification and deterministic key the write actually produced.

`DisassociateThumbnail` **remains** in the service block, with the same targeted semantics as HTTP `DELETE /v1alpha/associations` (§5):

```proto
message DisassociateThumbnailRequest {
  Resource association_target = 1;
  optional TargetType target_type = 2;  // defaults to FILE
  Resource thumbnail = 3;               // required by runtime validation
}

message DisassociateThumbnailResponse {
  repeated DisassociateResult succeeded = 1;
  repeated DisassociateResult failed = 2;
}

message DisassociateResult {
  Resource thumbnail = 1;
  optional string error_message = 2;  // set only for entries in `failed`
}
```

Only `resource_address` is used from `association_target` and `thumbnail`; supplied identities are ignored. The service finds and removes the one stored association matching the thumbnail address. `succeeded` and `failed` each contain at most one result, and both are empty when no matching association exists.

`CreateThumbnailsRequest` updates to use `Resource source` in `ResourceTarget`:

```proto
message ResourceTarget {
  Resource source = 1;
  optional TargetType target_type = 2;  // defaults to FILE
  optional Resource association_target = 3;
}
```

Field numbers above are fresh; this is a hard break.

### 7. Incoming `omni.storage.created` handling

`resource_address` is treated as required on incoming events. If an event arrives with an empty or missing `resource_address` the handler logs at `warn!` with an unique identifier for the notification and skips (ACK without enqueuing). Do not NACK — redelivery won't fix a malformed payload. This mirrors the existing skip-on-missing-identity behavior at `event_processing.rs:145`.

### 8. Migration

Hard break. Old stored metadata will be deleted out-of-band. No reader shim, no dual-write, no deprecated-route stub. Both services deploy in lockstep with the regenerated proto.

### 9. Auth scope

Unchanged. The bearer token presented to associate / disassociate flows through to every storage `MetadataService` call the operation makes. Targeted disassociate touches only the specified association target and thumbnail.

## Consequences

- **One Resource shape end-to-end.** Stored metadata, events, HTTP, and gRPC all use the same `{ resource_address, resource_identity? }` object. The overloaded `association_target` string and the flat `*_resource_identity` field pairs disappear.
- **Folders are first-class.** `target_type: "FOLDER"` is structurally distinct (identity omitted), no longer a special case overlaid on the same string field.
- **HTTP and gRPC are at full parity.** Both interfaces expose the same associate and targeted-disassociate surface for FILE and FOLDER targets. Each disassociate request identifies one association target and one thumbnail by address.
- **Broad cleanup is internal.** The `omni.storage.deleted` consumer can still remove every thumbnail for a deleted source through `disassociate_all_thumbnails`, but clients cannot trigger that sweep through HTTP or gRPC.
- **A failed association leaves nothing behind, usually.** The two metadata writes are not atomic, so the association-target-side write is rolled back when the thumbnail-side write fails (§4a). This narrows rather than closes the inconsistency window: a concurrent read during the rollback, or a rollback that itself fails, can still observe an association with no thumbnail metadata behind it.
- **Hard break, no shim.** Existing stored metadata is deleted out-of-band. Existing API callers must update in lockstep. Old routes return `404` from the framework default.
- **Coordinated deploy required.** Service and worker must ship the regenerated proto together. The worker already builds a nested HTTP body for `association_target`; only the `thumbnail` side and the route path change.
- **Storage layout decisions remain elsewhere.** The source-side metadata key suffix and thumbnail-destination-path concerns are owned by the storage ADR on branch `docs/ADR/0001-thumbnail-storage.md` and not duplicated here.

## Considered alternatives

- **Minimal additive change: add `resource_address` fields alongside existing flat ones, keep `association_target` overloaded.** Rejected. The overload between FILE-identity and FOLDER-address would persist, and downstream code would still need to branch on `target_type` to interpret one field. We were going to touch every call site anyway.
- **gRPC stays file-only; folder association and disassociate are HTTP-only.** Rejected. The worker is the only gRPC caller *today*, but an end user may choose to drive operations over gRPC, and a narrowed gRPC surface would force them onto HTTP for folders or disassociate. We chose full parity: both interfaces expose the same operations and target types. The cost is a `target_type` path and a `DisassociateThumbnail` RPC with (currently) only an HTTP analogue exercised, which we accept to keep the two interfaces interchangeable.
- **Expose sweep-all disassociate through HTTP and gRPC.** Rejected. A broad client-triggered delete can remove unrelated thumbnail variants. Sweep-all behavior is reserved for cleanup after the source itself is deleted.
- **`DELETE` with a JSON body for disassociate, mirroring associate's nested shape.** Rejected. The two required addresses and optional target type have an unambiguous flat query representation, so a request body adds no useful structure.
- **Soft-migrate stored metadata with a reader shim.** Rejected. Old metadata is being deleted out-of-band; a shim would double every read path for a one-time transition.
- **Dotted query-string keys for nested disassociate input.** Rejected. Disassociate uses two explicitly named address parameters, eliminating the need for nested query keys.

## Out of scope

- Storage layout (path-template derivation, source-side metadata key suffix). See the storage ADR in 'docs/ADR/0001-thumbnail-storage.md`.
- Rename handling (`omni.storage.renamed`). No handler exists today; not introduced here.
- Public sweep-all disassociate. Broad cleanup remains internal to storage-deletion handling.
- Backward compatibility for stored metadata or API consumers.
