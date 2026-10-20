# Metadata API — component map

> **Applies to:** `metadata/api` (top-level `nvidia.omniverse.metadata.v1beta` contract) · **Released:** no separate NGC API resource; released Storage Service **1.2.3** serves `nvidia.omniverse.metadata.v1beta` ([`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md) `[1.2.3]`) · **This repo's snapshot:** **0.3.0** ([`metadata/api/CHANGELOG.md`](../../../../metadata/api/CHANGELOG.md), 2026-08-18); OpenAPI `info.version` `metadata-protos-0.1.0-14-g41e6e34`; release commit "Release metadata-protos 0.0.0-manual.41e6e343" (only that string changed; the proto, `CHANGELOG.md` and READMEs are unchanged); [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md) still lists `metadata-protos-0.1.0` · **Sources:** [`metadata/`](../../../../metadata/README.md), [`metadata/api/`](../../../../metadata/api/README.md)

This page is a table of contents. The API reference, including the conflicts with the Storage Service, is [apis/metadata.md](../apis/metadata.md).

## What it is

Definitions only: one proto and one OpenAPI file for batch and query access to key/value metadata on resource and folder addresses. No implementation, client library, crate or conformance suite lives here (0.3.0 removed the `metadata-protos` Rust crate and the example service). The Storage Service is the implementation.

## Files

| Question | Go to |
|---|---|
| gRPC contract (package `nvidia.omniverse.metadata.v1beta`; RPCs `GetMetadataBatch`, `QueryMetadata`, `UpdateMetadataBatch`, `DeleteMetadataBatch`, all server-streaming) | [`proto/nvidia/omniverse/metadata/v1beta/metadata.proto`](../../../../metadata/api/proto/nvidia/omniverse/metadata/v1beta/metadata.proto) |
| REST contract | [`openapi/metadata/v1beta/metadata-api.yaml`](../../../../metadata/api/openapi/metadata/v1beta/metadata-api.yaml) |
| History and intentional differences from the storage-namespaced API | [`CHANGELOG.md`](../../../../metadata/api/CHANGELOG.md) |
| Generating bindings | [`README.md`](../../../../metadata/api/README.md#generating-bindings) (see defects) |
| Conformance | [`metadata/conformance/README.md`](../../../../metadata/conformance/README.md): none |
| Implementation configuration (backends PostgreSQL/DynamoDB/Cosmos DB, binding per storage endpoint) | [`storage/reference/storage-service/docs/helm/storage-metadata.rst`](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst), [storage-service.md](storage-service.md) |
| The older storage-namespaced API (`nvidia.omniverse.storage.metadata.v1alpha`) | [storage-api.md](storage-api.md), [apis/metadata.md](../apis/metadata.md#two-contracts-one-backend) |
| Security reporting, license | [`SECURITY.md`](../../../../metadata/api/SECURITY.md), [`LICENSE.txt`](../../../../metadata/api/LICENSE.txt) (Apache-2.0) |

## Version history (from `CHANGELOG.md`)

| Version | Change |
|---|---|
| 0.1.0 (2025-10-02) | Extracted from storage-protos; example in-memory service and BDD suite in the source project |
| 0.2.0 (2026-08-12) | Batch/query surface, `ResourceLocation`, `BatchErrorEntry`, per-entry namespaces; local `ResourceIdentity`/`FolderAddress` types; `buf.yaml`, `proto/.../VERSION` |
| 0.3.0 (2026-08-18) | Promoted to `v1beta`; single-entry RPCs removed; `ResourceLocation` reduced to `resource_address` (1) and string `folder_address` (2); Rust crate removed |

The `buf.yaml`, `Makefile` and `proto/nvidia/omniverse/metadata/VERSION` mentioned by 0.2.0 are not in this snapshot.

## Known defects and conflicts

Re-checked against the current Metadata API (`41e6e343`) and Storage Service (`ba723011`) snapshots: all of the following still hold.

- **Wire conflict with the Storage Service.** The Storage Service compiles its own v1beta copy ([`storage-service-protos/proto/nvidia/omniverse/metadata/v1beta/metadata.proto`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/metadata/v1beta/metadata.proto)) in which `ResourceLocation` field 2 is a `ResourceIdentity` message and field 3 a `FolderAddress` message. A `folder_address` sent per this contract is misread by that service.
- **REST conflict with the Storage Service.** This OpenAPI maps `POST /metadata` to update and `POST /metadata/get` to get; the Storage Service serves `POST /v1beta/metadata` as get and `PUT /v1beta/metadata` as update.
- [`README.md`](../../../../metadata/api/README.md) says the schema "stays wire-compatible" with the storage-namespaced API; `ResourceLocation` does not.
- [`README.md`](../../../../metadata/api/README.md) example `protoc -I proto --python_out=. --grpc_python_out=. …` needs the gRPC Python plugin; use `python -m grpc_tools.protoc`.
- The OpenAPI file declares `security: [bearerAuth]` but defines no `securitySchemes`, so generators cannot resolve the scheme.
- [`metadata/README.md`](../../../../metadata/README.md) says "The Storage Service implements the `v1beta` Metadata API" without the conflicts above.

Related: [apis/metadata.md](../apis/metadata.md), [versions/api-contracts.md](../versions/api-contracts.md).
