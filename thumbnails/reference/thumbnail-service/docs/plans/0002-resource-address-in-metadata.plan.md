# Implementation plan — resource_address in association metadata

Companion to `thumbnail-service/docs/adr/0002-resource-address-in-metadata.md`. The ADR captures the *decision*; this file captures the *work*. Read the ADR first — the design rationale, on-the-wire shapes, response codes, and out-of-scope items are all there and not duplicated here.

## Scope summary

Both services change in lockstep:
- `thumbnail-service/` (Rust)
- `thumbnail-task-queue/` (Python)

Coordinated cutover required (regenerated proto must land in both repos at the same time). Hard break — no shim, no dual-write. Old stored metadata will be deleted out-of-band.

## Implementation steps

Sequenced for coordinated cutover. All numbered steps are required.

| # | Area | Files |
|---|------|-------|
| 1 | Forward `resource_address` in event handler | `thumbnail-service/crates/common/src/notifications/event_processing.rs` |
| 2 | Populate `TaskQueueSource.resource_address` | `thumbnail-service/crates/common/src/task_queue/client.rs` |
| 3 | Rename storage-metadata params to `resource_address` (clarity-only) | `thumbnail-service/crates/common/src/storage/metadata.rs` |
| 4 | Unify `ResourceRef` → `Resource`; validation by `target_type` | `thumbnail-service/crates/service/src/http/types/requests.rs` |
| 5 | Nested metadata value shape in association writes | `thumbnail-service/crates/common/src/storage/association.rs` |
| 6 | Associate endpoint at `POST /v1alpha/associations`; remove `PUT /v1alpha/thumbnails/{id}` route | `thumbnail-service/crates/service/src/http/mod.rs` |
| 7 | Targeted disassociate endpoint requiring target and thumbnail addresses | `thumbnail-service/crates/service/src/http/mod.rs`, `association.rs` |
| 8 | `CreateThumbnails` nested `source: Resource` | `thumbnail-service/crates/service/src/http/types/requests.rs` + handler |
| 9 | Proto rework + regenerated stubs (both repos) | `thumbnail-service/proto/.../thumbnail_service.proto` |
| 10 | Event publishers emit nested shape | `thumbnail-service/.../association.rs`, `thumbnail-task-queue/app/core/event_publisher.py` |
| 11 | Worker: nest `thumbnail` in HTTP body + adopt `POST /v1alpha/associations` + new gRPC stubs | `thumbnail-task-queue/app/thumbnail_metadata.py` |
| 12 | Task-queue API model: plumb `resource_address` from `/image-to-image` → Celery task → worker | `thumbnail-task-queue/app/thumbnail_core.py`, request/task models |
| 13 | Task-queue OpenAPI regen | `thumbnail-task-queue/openapi.json` |
| 14 | Worker docs | `thumbnail-task-queue/docs/omni-thumbnail-created-event-format.md`, `worker-metadata-structure.md` |
| 15 | Service tests, fixtures, `thumbnail.openapi.yaml` | `thumbnail-service/` |
| 16 | Task-queue tests, fixtures | `thumbnail-task-queue/tests/` |

## Test rewrite policy

Hard break. Delete existing tests that assert any of:
- The flat metadata shape.
- The `PUT /v1alpha/thumbnails/{id}` associate route.
- The old gRPC `AssociateThumbnailRequest` field names (`source_resource_identity`, `thumbnail_resource_identity`, `target_type`).
- The old disassociate query shape and any sweep-all public behavior.

Write fresh tests against the new shapes. Do not parameterize over old-vs-new.

Specifically rewrite:

- `thumbnail-service/crates/common/src/tests/storage_association_tests.rs`
- `thumbnail-service/crates/common/src/tests/storage_metadata_tests.rs`
- `thumbnail-service/crates/common/src/tests/notifications/event_processing_tests.rs`
- `thumbnail-service/crates/service/src/http/tests.rs` (association + disassociation cases)
- `thumbnail-task-queue/tests/` — any HTTP/gRPC call-site fixtures

## Watch-outs for the implementing agent

- The worker (`thumbnail-task-queue/app/thumbnail_metadata.py`) already builds a nested `association_target` HTTP body — but **not** a nested `thumbnail` body. Today the thumbnail identity sits in the URL path. Step 11 changes both at once.
- The current `thumbnail_service.proto` already has `target_type` on `AssociateThumbnailRequest` and a `DisassociateThumbnailRequest` message. Both are reworked in place; disassociate gains a required thumbnail `Resource`. The `worker-metadata-structure.md` doc is out of date and step 14 updates it.
- Storage `MetadataService.GetMetadata` returns *all* keys when `user_metadata_keys` is empty (per the proto comment in `metadata.proto:37-38`). This lets targeted disassociate find the matching thumbnail entry and lets storage-deletion handling perform its separate internal sweep without a list RPC.

## Suggested skills for the implementing agent

- `tdd` — for each step, given the delete-and-rewrite test policy.
- `verify` — end-to-end check after implementation: event → task-queue → worker → AssociateThumbnail → stored metadata, with both fields present.
- `code-review` — once a working tree exists, before any commit.

## Cross-references

- ADR: `thumbnail-service/docs/adr/0002-resource-address-in-metadata.md`
- Glossary: defined where enforced — see the ADR's Context section
- Related ADR (storage layout, out of scope here): branch `task/storage-adr`
