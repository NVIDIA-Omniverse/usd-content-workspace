# 0.3.0 – 2026-08-18

Promote the Metadata API to `v1beta` and refine it to a batch-only surface.

- Promotes the top-level metadata API from `v1alpha` to `v1beta` (package `nvidia.omniverse.metadata.v1beta`), matching the storage-service which now serves the top-level API at v1beta.
- Removes the single-entry `GetMetadata`, `UpdateMetadata`, and `DeleteMetadata` RPCs and their request/response messages; the batch and query RPCs are now the only surface.
- Refines `ResourceLocation` to a oneof of two plain strings, `resource_address` and `folder_address`. The new `v1beta` message uses sequential fields 1 and 2; `folder_address` is a bare RFC 3986 URI string, matching the OpenAPI spec. The metadata API therefore owns its whole schema and takes no dependency on another buf module.
- Drops the matching single-entry REST operations from the OpenAPI spec and the single-entry scenarios, step code, and example-service handlers from the conformance suite.
- Removes the unused `v1testmetadata` proto and OpenAPI twin; only the promoted `v1beta` contract is generated.
- Removes the `metadata-protos` Rust crate (`src/`, `build.rs`, `tests/`, `Cargo.toml`, `.cargo/config.toml`, `release-plz.toml`, `cliff.toml`, `CHANGELOG-rust.md`, and the `release-plz-*` CI jobs). This repository publishes protocol definitions only — the proto and OpenAPI files — and consumers generate their own bindings. The hand-written serde for `google.protobuf.Value`/`Struct`/`ListValue` and the externally tagged `ResourceLocation` oneof already lives in `storage-service/storage-service-protos`, so no consumer loses it. The previously published `metadata-protos` 0.1.0 crate version and its `metadata-protos-0.1.0` tag are left in place.

# 0.2.0 – 2026-08-12

Refresh the Metadata API from `storage-protos` `trunk` (commit `fca1a66`).

- Adds the batch/query surface: `GetMetadataBatch`, `QueryMetadata`, `UpdateMetadataBatch`, `DeleteMetadataBatch` (all server-streaming), with `ResourceLocation`, `BatchErrorEntry`, and per-entry `namespace` support.
- Defines `ResourceIdentity` and `FolderAddress` locally so the Metadata API is self-contained (no import from `storage-protos`); wire-compatible with the storage-namespaced types.
- Refreshes the OpenAPI spec, the example in-memory service, and the BDD conformance suite to cover the batch/query/namespace scenarios.
- Drops the legacy `nvidia.omniverse.storage.metadata.*` namespace from the example service — this repo hosts the top-level `nvidia.omniverse.metadata.*` API only.
- Adds `proto/nvidia/omniverse/metadata/VERSION` recording the storage-protos commit this schema was synced from, a `buf.yaml` module definition, and fixes stale `storage-protos` paths in the Makefile.

# 0.1.0 – 2025-10-02

Initial extraction of the Metadata API from `storage-protos`.

- Provides standalone proto and OpenAPI definitions for `nvidia.omniverse.metadata`.
- Includes example in-memory Metadata service exposing both legacy and new namespaces.
- Copies BDD conformance tests and supporting infrastructure from `storage-protos`.
- Aligns CI, tooling, and documentation scaffolding with the original repository.
