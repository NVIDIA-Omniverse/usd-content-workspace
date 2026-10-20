# Metadata API

> **Applies to:** the top-level Metadata API (`nvidia.omniverse.metadata.v1beta`) and the storage-namespaced Metadata API (`nvidia.omniverse.storage.metadata.v1alpha`), as implemented by the Storage Service and the Python filesystem reference · **Released:** no separate NGC API resource; the storage-namespaced v1alpha surface ships inside `storage-api` **1.0.0-beta.4**; released Storage Service **1.2.3** added the top-level v1beta API ([`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md) `[1.2.3]`) · **This repo's snapshot:** Metadata API **0.3.0** ([`metadata/api/CHANGELOG.md`](../../../../metadata/api/CHANGELOG.md); OpenAPI `info.version` reads `metadata-protos-0.1.0-14-g41e6e34`; [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md) records the package version `0.3.0`), storage-namespaced v1alpha at Storage API unreleased (after 1.0.0-beta.4) · **Sources:** [`metadata/api/`](../../../../metadata/api/), [`storage/api/proto/nvidia/omniverse/storage/metadata/v1alpha/metadata.proto`](../../../../storage/api/proto/nvidia/omniverse/storage/metadata/v1alpha/metadata.proto), [`storage/api/openapi/metadata/v1alpha/metadata-api.yaml`](../../../../storage/api/openapi/metadata/v1alpha/metadata-api.yaml), Storage Service `storage-service/src/api/metadata*` and [`docs/helm/storage-metadata.rst`](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst)

Key/value metadata attached to a resource address, a folder address or (v1alpha only) a resource identity. Values are JSON values (`google.protobuf.Value`) with an ETag per key; keys live in a namespace (default namespace when omitted). Metadata is separate from FileObject *native* metadata, which is stored per object version through `WriteParameters.native_metadata` ([storage.md](storage.md#v1alpha-additions)).

Confirm which wire versions the target serves (Discovery catalog or [capture a deployment](../versions/capture-a-deployment.md)) before generating a client.

## Two contracts, one backend

| | Storage-namespaced | Top level |
|---|---|---|
| Package | `nvidia.omniverse.storage.metadata.v1alpha` | `nvidia.omniverse.metadata.v1beta` |
| Definitions | [`storage/api/proto/.../metadata/v1alpha/metadata.proto`](../../../../storage/api/proto/nvidia/omniverse/storage/metadata/v1alpha/metadata.proto), [`storage/api/openapi/metadata/v1alpha/`](../../../../storage/api/openapi/metadata/v1alpha/metadata-api.yaml) | [`metadata/api/proto/.../v1beta/metadata.proto`](../../../../metadata/api/proto/nvidia/omniverse/metadata/v1beta/metadata.proto), [`metadata/api/openapi/metadata/v1beta/`](../../../../metadata/api/openapi/metadata/v1beta/metadata-api.yaml) |
| RPCs | `GetMetadata`, `UpdateMetadata`, `DeleteMetadata` (single item), plus `GetMetadataBatch`, `QueryMetadata`, `UpdateMetadataBatch`, `DeleteMetadataBatch` | batch and query only: `GetMetadataBatch`, `QueryMetadata`, `UpdateMetadataBatch`, `DeleteMetadataBatch` |
| `ResourceLocation` oneof | `resource_address` (1, string), `resource_identity` (2, `fileobject.v1alpha.ResourceIdentity`), `folder_address` (3, `filefolder.v1alpha.FolderAddress`) | `resource_address` (1, string), `folder_address` (2, string RFC 3986 URI) |
| Served by | Storage Service (all RPCs), filesystem reference (REST + gRPC) | Storage Service only |

The Storage Service backs both with the same metadata backend (PostgreSQL, DynamoDB; Cosmos DB is documented as non-functional in this snapshot), configured under `config.metadata` and bound per storage endpoint with `metadataBackend`. See [`storage-metadata.rst`](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst) and [components/storage-service.md](../components/storage-service.md).

## Batch semantics (both versions)

From the proto comments:

- Each entry succeeds or fails independently; the call succeeds if the request is well formed. Per-entry failures carry `BatchErrorEntry { code, message }` with `CODE_INVALID_ARGUMENT`, `CODE_NOT_FOUND`, `CODE_PERMISSION_DENIED`, `CODE_FAILED_PRECONDITION` (ETag mismatch), `CODE_INTERNAL`, `CODE_RESOURCE_EXHAUSTED` and `CODE_UNAVAILABLE` (both safe to retry with backoff).
- Responses are streamed and **unordered**; match each result to its request with the zero-based `request_index`.
- Updates and deletes accept `expected_etag` for optimistic concurrency; a successful update returns the new `etag`.
- Do not send conflicting writes to the same (namespace, address, key) in one batch: the surviving value is undefined.
- `QueryMetadata` returns every key and value for one location within one namespace, in pages.
- REST batch bodies are `{"requests": [...]}`; the v1alpha OpenAPI caps `requests` at 256 entries and addresses at 4096 characters.

## REST

### Storage-namespaced v1alpha (paths under `/v1alpha`)

| Method | Path | Operation |
|---|---|---|
| `POST` | `/metadata` | UpdateMetadataBatch |
| `DELETE` | `/metadata` | DeleteMetadataBatch |
| `POST` | `/metadata/get` | GetMetadataBatch |
| `POST` | `/metadata/query` | QueryMetadata |
| `POST` | `/metadata/{uri}` | GetMetadata — **deprecated** |
| `PUT` / `DELETE` | `/metadata/{uri}/{key}` | UpdateMetadata / DeleteMetadata — **deprecated** |

The Storage Service routes the same paths ([`storage-service/src/api/metadata/rest.rs`](../../../../storage/reference/storage-service/storage-service/src/api/metadata/rest.rs)). The deprecated single-item endpoints address only the default namespace.

### Top-level v1beta: the contract and the Storage Service disagree

Both conflicts below (REST routes and gRPC `ResourceLocation`) still hold in this snapshot: neither the `metadata/api` proto nor the Storage Service's vendored proto or REST router changed in the latest Metadata API (`41e6e343`) and Storage Service (`ba723011`) snapshots.

| Operation | `metadata/api` OpenAPI (under the v1beta prefix) | Storage Service snapshot ([`metadata_api/rest.rs`](../../../../storage/reference/storage-service/storage-service/src/api/metadata_api/rest.rs)) |
|---|---|---|
| GetMetadataBatch | `POST /metadata/get` | `POST /v1beta/metadata` |
| UpdateMetadataBatch | `POST /metadata` | `PUT /v1beta/metadata` |
| DeleteMetadataBatch | `DELETE /metadata` | `DELETE /v1beta/metadata` |
| QueryMetadata | `POST /metadata/query` | `POST /v1beta/metadata/query` |

A REST client generated from `metadata/api` and pointed at the Storage Service would send an update where the service expects a read. Until the source projects reconcile this, write REST clients for the Storage Service against the OpenAPI document the service itself serves at `/v1beta/metadata/openapi.json` (when `service.rest.serveOpenapi` is true), not against `metadata/api/openapi`.

### gRPC wire mismatch in `ResourceLocation`

The Storage Service compiles its own copy of the v1beta proto, [`storage-service-protos/proto/nvidia/omniverse/metadata/v1beta/metadata.proto`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/metadata/v1beta/metadata.proto). In that copy `ResourceLocation` keeps the v1alpha shape (`resource_identity = 2` as a message, `folder_address = 3` as a `FolderAddress` message). The `metadata/api` 0.3.0 contract moved `folder_address` to field `2` as a plain string and dropped `resource_identity`. Therefore:

- `resource_address` (field 1) means the same thing to both.
- A `folder_address` sent by a client built from `metadata/api` arrives at the Storage Service as field 2, which that service decodes as a `ResourceIdentity` message. Expect a decode error or a wrong location.

[`metadata/api/README.md`](../../../../metadata/api/README.md) says the API "stays wire-compatible" with the storage-namespaced schema; for `ResourceLocation` it does not. For folder metadata against the Storage Service, generate from the service's vendored copy or use the v1alpha API, and verify with a round trip.

## Example: read two keys (v1alpha, REST)

```bash
# Filesystem reference on localhost:8011; a deployed service needs TLS and a bearer token.
curl -s -X POST http://localhost:8011/v1alpha/metadata/get \
  -H 'Content-Type: application/json' \
  -d '{"requests":[
        {"resource_location":{"resource_address":"file-storage://fileservice/test.txt"},"user_metadata_key":"owner"},
        {"resource_location":{"resource_address":"file-storage://fileservice/test.txt"},"user_metadata_key":"status","namespace":"pipeline"}]}'
```

Check the exact JSON shape against [`openapi/metadata/v1alpha/metadata-api.yaml`](../../../../storage/api/openapi/metadata/v1alpha/metadata-api.yaml) for the version you target; the v1alpha `UpdateMetadataBatch` entry changed incompatibly in unreleased Storage API source after 1.0.0-beta.4.

## Authorization

The Storage Service checks metadata operations through the Permission Service when `config.permissions.enabled` is true; `config.permissions.includeAclInAllKeys` controls whether the `acl` key is returned when no keys are requested ([`storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst)). Action names are listed there. See [apis/permissions.md](permissions.md).

## Generating bindings

```bash
# Top-level v1beta (self-contained, imports only google/protobuf/struct.proto)
cd metadata/api
protoc -I proto --python_out=. --grpc_python_out=. proto/nvidia/omniverse/metadata/v1beta/metadata.proto
```

`protoc` alone has no `--grpc_python_out` plugin; use `python -m grpc_tools.protoc` (as in [storage.md](storage.md#minimal-client-calls)) or your language's gRPC plugin. The storage-namespaced v1alpha proto imports the v1alpha `fileobject` and `filefolder` protos, so compile it with `--proto_path=storage/api/proto` and those files.

## Validation

There is no Metadata API conformance suite in this snapshot ([`metadata/conformance/README.md`](../../../../metadata/conformance/README.md)). The 0.1.0 and 0.2.0 changelog entries describe a BDD suite and an in-memory example service in the source project; neither was imported here. Storage-namespaced v1alpha metadata is covered by the Storage API suite (`features/metadata/v1alpha/metadata.feature`); see [test/conformance.md](../test/conformance.md#storage-api-conformance-suite). The Storage Service smoke test exercises the deprecated single-item v1alpha REST and gRPC calls.

Related: [components/metadata-api.md](../components/metadata-api.md), [storage.md](storage.md).
