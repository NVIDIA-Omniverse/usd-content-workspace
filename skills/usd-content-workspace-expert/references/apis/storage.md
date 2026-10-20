# Storage API

> **Applies to:** Storage API contract (gRPC + REST) and its two implementations in this repo (Python filesystem reference, Storage Service) · **Released:** NGC API resource `storage-api` **1.0.0-beta.4**; published HTML docs [1.0.0-beta.3](https://docs.omniverse.nvidia.com/ovstorage/storage-api/1.0.0-beta.3/index.html) · **This repo's snapshot:** **unreleased (after 1.0.0-beta.4)** (read from every OpenAPI `info.version`, both `pyproject.toml` files; source commit in [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md)) · **Sources:** [`storage/api/proto/`](../../../../storage/api/proto/), [`storage/api/openapi/`](../../../../storage/api/openapi/), [`storage/api/CHANGELOG.md`](../../../../storage/api/CHANGELOG.md), [`storage/api/conformance_tests/`](../../../../storage/api/conformance_tests/), [`storage/api/filesystem_example/`](../../../../storage/api/filesystem_example/), Storage Service and Storage Core source, the [source skill package](../../NOTICE.md)

The Storage API is the data-access contract of the stack: read, write, list, version, and (in `v1alpha`) copy, move, and attach metadata to objects in any backend. A **storage adapter** implements it over a backend (S3, Azure Blob, a filesystem, a database). Clients bind to the contract, not to an adapter.

Before you generate a client or apply any example, confirm which version the target actually runs ([capture a deployment](../versions/capture-a-deployment.md)) and read the Discovery catalog or `ListServices` for the wire versions it serves. See [API contract versions](../versions/api-contracts.md) for how wire, document and resource versions differ.

## Versions and wire packages

| Group | gRPC package (wire version) | Proto | OpenAPI | Snapshot has |
|---|---|---|---|---|
| FileObject | `nvidia.omniverse.storage.fileobject.v1beta` / `.v1alpha` | [`fileobject/`](../../../../storage/api/proto/nvidia/omniverse/storage/fileobject/) | [`openapi/fileobject/`](../../../../storage/api/openapi/fileobject/) | `v1alpha`, `v1beta` |
| FileFolder | `nvidia.omniverse.storage.filefolder.v1beta` / `.v1alpha` | [`filefolder/`](../../../../storage/api/proto/nvidia/omniverse/storage/filefolder/) | [`openapi/filefolder/`](../../../../storage/api/openapi/filefolder/) | `v1alpha`, `v1beta` |
| Versioning | `nvidia.omniverse.storage.versioning.v1beta` / `.v1alpha` | [`versioning/`](../../../../storage/api/proto/nvidia/omniverse/storage/versioning/) | [`openapi/versioning/`](../../../../storage/api/openapi/versioning/) | `v1alpha`, `v1beta` |
| Capabilities | `nvidia.omniverse.storage.capabilities.v1beta` / `.v1alpha` | [`capabilities/`](../../../../storage/api/proto/nvidia/omniverse/storage/capabilities/) | [`openapi/capabilities/`](../../../../storage/api/openapi/capabilities/) | `v1alpha`, `v1beta` |
| Metadata (storage-namespaced) | `nvidia.omniverse.storage.metadata.v1alpha` | [`metadata/v1alpha/`](../../../../storage/api/proto/nvidia/omniverse/storage/metadata/v1alpha/) | [`openapi/metadata/v1alpha/`](../../../../storage/api/openapi/metadata/v1alpha/) | `v1alpha` only |

There is no `v1` package in this snapshot. The top-level Metadata API (`nvidia.omniverse.metadata.v1beta`) is a separate contract: see [metadata.md](metadata.md).

`v1alpha` is a strict superset of `v1beta` for each group (the reference service's [`rest_service/routes.py`](../../../../storage/api/filesystem_example/src/local_filesystem_service/rest_service/routes.py) states the channel rule). `v1beta` is the stable surface a custom adapter must implement; `v1alpha` adds preview RPCs that may still change.

### Released 1.0.0-beta.4 versus the unreleased snapshot

From [`CHANGELOG.md`](../../../../storage/api/CHANGELOG.md). Everything below beta.4 is snapshot-only; a client built from this repo can call RPCs or send fields a released 1.2.3 Storage Service or a beta.4-based adapter does not have.

| Version | Change |
|---|---|
| Unreleased (after 1.0.0-beta.4) | `fail_if_exists` on `WriteParameters`, `CopyRequest`, `MoveRequest`; `UpdateMetadataBatch` added; v1alpha native storage metadata on `Write`/`Read`/`Stat`/`ListStat` plus `GetNativeMetadataSupport`; v1alpha `IsVersioned` and `IsReadOnly`; v1alpha Metadata `GetMetadataBatch`, `DeleteMetadataBatch`, `QueryMetadata` (single-item REST endpoints deprecated); **breaking (v1alpha):** `UpdateMetadataBatch` entries redesigned around `ResourceLocation` and per-entry namespaces; `Enumerate` defined as recursive in v1alpha and v1beta; `ListRoutes` matching rules clarified; identity-binding conformance tightened; OpenAPI `$ref` bundling defect fixed |

The released beta.4 ZIP (SHA-256 `e7339b1ff65bc765a3696e31c510b5cab4f9f3f9a5d7b2f6c13ae68741e30e75`, inspected for the source skill package) contains OpenAPI YAML whose `$ref` keys were stripped, so none of its nine OpenAPI files parse. Do not generate REST clients from those files and do not silently repair them; use its protobuf definitions, or obtain a corrected versioned artifact. All nine OpenAPI files in this snapshot parse (checked with a YAML loader); that does not make them the released contract.

### Copies of the contract inside implementations

Generated clients must come from the definitions the deployed service was built with. The implementations here do **not** compile `storage/api/proto` directly:

| Copy | Pin | What differs from `storage/api` |
|---|---|---|
| Storage Core [`storage-core-protos/proto/storage-protos/`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/) | [`VERSION`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/VERSION): storage-protos tag `0.28.0` (hash `df15f414…`), plus hand-applied native-metadata hunks from unreleased source and local `IsReadOnly`/`IsVersioned` RPCs | Every vendored file differs textually; the v1alpha Metadata service has only `GetMetadata`, `UpdateMetadata`, `UpdateMetadataBatch`, `DeleteMetadata` (no `GetMetadataBatch`, `QueryMetadata`, `DeleteMetadataBatch`) |
| Storage Service [`storage-service-protos/proto/nvidia/omniverse/storage/`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/storage/) | [`VERSION`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/storage/VERSION): Buf registry commit `b191bc7cf41c4951afe74d802459b650` | Vendors only the v1alpha metadata proto (identical to `storage/api`) and its imports, plus an experimental `metadata/v1testmetadata` proto that is compiled but not served |
| Filesystem reference and conformance suite `src/nvidia/` | Generated Python stubs committed in this snapshot | Generated from this snapshot's protos |

## Core concepts

Read the full definitions in the published user manual or its source, [`docs/user_manual.md`](../../../../storage/api/docs/user_manual.md).

| Concept | Points to | Content | Used by |
|---|---|---|---|
| **Resource address** | A location (URI; RFC 3986 for file-system-like stores) | Mutable: the same address can return different bytes over time | `Stat`, `ReadFromAddress`, `Write`, `Delete`, `Enumerate`, `List`, `EnumerateVersions`, copy/move destination |
| **Resource identity** (`ResourceIdentity.encoded_identity`) | One immutable version of an object | Always the same bytes while it exists | `Read`, returned by `Stat`/`Write`/`EnumerateVersions`/`CompleteRedirectUpload`/`Copy` |

Rules a client may rely on:

- Treat identities as opaque. Do not parse them; they are suitable as cache keys but not as equality tests (two identities can name the same bytes).
- An adapter must recognize its own addresses and identities, so a router can send a request to the right service (`ListTopLevelAddresses`, and `ListRoutes` in v1alpha).
- On versioned storage an identity stays bound to the bytes it was returned for. On unversioned storage a superseded identity must return `NOT_FOUND`/404, never the replacement bytes (current conformance suite).
- Folder behavior is one of `FOLDER_MODE_NATIVE`, `FOLDER_MODE_NO_EMPTY`, `FOLDER_MODE_HYBRID` (v1alpha `GetFolderMode`). An adapter picks the mode that matches its backend.

Address formats differ by implementation; ask the service rather than guessing:

| Implementation | Address form | Confirm with |
|---|---|---|
| Python filesystem reference | `file-storage://fileservice/<path>` (CLI `--base-uri`, env `FILESERVICE_SERVER_BASE_URI`) | `GET /v1beta/capabilities/top-level-addresses` |
| Storage Service (S3) | The bucket's HTTPS endpoint URL, for example `https://<bucket>.s3.<region>.amazonaws.com/<key>`; not `s3://` | `ListTopLevelAddresses`, v1alpha `GET /v1alpha/capabilities/routes` |
| Storage Service (Azure Blob) | `https://<account>.blob.core.windows.net/<container>/<blob>`; not `azureblob://` | same |

The Storage Service snapshot normalizes the path component of addresses by default (`config.urlNormalizer.enabled: true`, an `[Unreleased]` change); released 1.2.3 does not. See [components/storage-service.md](../components/storage-service.md) before upgrading.

## gRPC services

### v1beta (implement all of these for a conforming adapter)

| Service | RPC | Streaming | Purpose |
|---|---|---|---|
| `CapabilitiesService` | `ListServices` | unary | API groups and versions this server implements |
| | `ListTopLevelAddresses` | unary | Root addresses for browsing |
| `FileObjectService` | `Stat` | unary | Identity and `Metadata` (`data_object_size`, `last_modified_timestamp`) at an address |
| | `Read` | server | Bytes for an identity: `metadata`, then `chunk`s or a `redirect` |
| | `ReadFromAddress` | server | Current bytes at an address: `resource_info`, then `chunk`s or a `redirect` |
| | `Write` | bidirectional | Client sends `WriteParameters`, server answers with chunks-accepted, `resource_info`, a write redirect, or a multipart session |
| | `FetchWriteTypeInfo` | unary | Preferred upload method per size interval |
| | `CompleteRedirectUpload` | unary | `ResourceInfo` after a redirect upload |
| | `UploadPart` | unary | Presigned URLs for further multipart parts |
| | `CompleteMultipartUpload` / `AbortMultipartUpload` | unary | Finish or cancel a multipart session |
| | `Delete` | unary | Remove the object and its versions; optional `previous_version` precondition |
| | `Enumerate` | server | Every data object below an address, recursively (current definition) |
| `FileFolderService` | `List` | server | Subfolder and sub-resource addresses |
| | `ListStat` | server | `List` plus identity and metadata per entry; can be much slower |
| | `DeleteFolder` | unary | Delete a folder; must fail if not empty |
| `VersioningService` | `EnumerateVersions` | server | `VersionInfo` items and a `VersionsOrder` (`NEWEST_FIRST`, `OLDEST_FIRST`, `BY_KEY`) |

v1beta has **no** `CreateFolder`. [`storage/api/AGENTS.md`](../../../../storage/api/AGENTS.md) and [`templates/production_impl/README.md`](../../../../storage/api/templates/production_impl/README.md) list it under v1beta; the proto does not.

### v1alpha additions

| Service | RPC | Purpose |
|---|---|---|
| `FileObjectService` | `Copy`, `Move` | Copy from an identity; move an address. Both accept `fail_if_exists` |
| | `GetOptimisticLockingSupport` | Which operations honor `previous_version` / `fail_if_exists` |
| | `GetNativeMetadataSupport` | Whether non-empty `native_metadata` writes are accepted at this exact address (unreleased) |
| | `IsReadOnly` | Whether an address rejects mutations (unreleased) |
| `FileFolderService` | `CreateFolder`, `GetFolderMode` | Explicit folders and the folder mode; `ListStatRequest.include_native_metadata` (unreleased) |
| `VersioningService` | `IsVersioned` | Whether an address keeps version history (unreleased) |
| `CapabilitiesService` | `ListRoutes` | Wildcard route patterns: `*` matches one segment, trailing `**` matches descendants, most specific wins, lexicographic tie-break |
| `MetadataService` | `GetMetadata`, `UpdateMetadata`, `DeleteMetadata`, `GetMetadataBatch`, `QueryMetadata`, `UpdateMetadataBatch`, `DeleteMetadataBatch` | Key/value metadata; see [metadata.md](metadata.md) |

Native metadata rule (from [`conformance_tests/README.md`](../../../../storage/api/conformance_tests/README.md)): an empty `native_metadata` field is not a support signal. Query `GetNativeMetadataSupport` for the exact address. When support is reported `false`, a non-empty native-metadata write must be rejected before mutation with `UNIMPLEMENTED`/501.

## REST endpoints

Paths are relative to a version prefix (`/v1beta` or `/v1alpha`) on the REST base URL. Percent-encode address and identity path parameters once.

| Method | Path | Operation | Success codes |
|---|---|---|---|
| `GET` | `/capabilities/services` | ListServices | 200 |
| `GET` | `/capabilities/top-level-addresses` | ListTopLevelAddresses | 200 |
| `GET` | `/capabilities/routes` (v1alpha) | ListRoutes | 200 |
| `HEAD` | `/fileobject/by-address/{resource_address}` | Stat; result in headers | 204 |
| `GET` | `/fileobject/by-address/{resource_address}` | ReadFromAddress; `download_preference=body\|redirect` | 200 body, 300 redirect JSON |
| `PUT` | `/fileobject/by-address/{resource_address}` | Write; `data_object_size` (required), `upload_preference=body\|redirect\|multipart`, `previous_version`, `fail_if_exists` | 201; 300 redirect/multipart; errors 400, 409, 501 |
| `DELETE` | `/fileobject/by-address/{resource_address}` | Delete; optional `previous_version` | 204 |
| `POST` | `/fileobject/by-address/{resource_address}/redirect/complete` | CompleteRedirectUpload | 200 |
| `POST` | `/fileobject/by-address/{resource_address}/multipart/prepare` \| `complete` \| `abort` | UploadPart / CompleteMultipartUpload / AbortMultipartUpload | 200 / 200 / 204 |
| `GET` | `/fileobject/by-identity/{resource_identity}` | Read | 200, 300 |
| `GET` | `/fileobject/data-objects/{resource_address}` | Enumerate; `max_page_size`, `continuation_handle` | 200 |
| `GET` | `/fileobject/upload-options/by-address/{resource_address}` | FetchWriteTypeInfo | 200 |
| `POST` | `/fileobject/by-identity/{resource_identity}/copy` (v1alpha) | Copy | — |
| `POST` | `/fileobject/by-address/{resource_address}/move` (v1alpha) | Move | — |
| `GET` | `/fileobject/native-metadata-support/{a}`, `/fileobject/is-read-only/{a}`, `/fileobject/optimistic-locking-support/{a}` (v1alpha) | Capability queries | 200 |
| `GET` / `DELETE` | `/filefolder/list/{folder_address}` | List / DeleteFolder (paginated list) | 200 |
| `GET` | `/filefolder/liststat/{folder_address}` | ListStat | 200 |
| `PUT` | `/filefolder/{folder_address}` (v1alpha) | CreateFolder | — |
| `GET` | `/filefolder/get-folder-mode/{folder_address}` (v1alpha) | GetFolderMode | 200 |
| `GET` | `/versioning/{resource_address}/versions` | EnumerateVersions | 200 |
| `GET` | `/versioning/is-versioned/{resource_address}` (v1alpha) | IsVersioned | 200 |

Stat and reads return the response headers `x-nvidia-omniverse-storage-metadata` (JSON `Metadata`) and `x-nvidia-omniverse-storage-resource-identity`. A redirect body for reads carries `redirect_target_url` and `additional_headers`; a write redirect also carries `method` and `completion_header_names`. Paginated responses return `next_continuation_handle`; pass it back as `continuation_handle` until it is absent.

The Storage API contract does not define a health endpoint. The filesystem reference serves none; the Storage Service serves the standard gRPC health service on a separate probe port (`8100`). The source skill's "REST `GET /health` on any conforming adapter" is not supported by this snapshot.

## Upload and download flows

**Download:** call `ReadFromAddress` (or `Read` with an identity). The first stream message is metadata; then either data chunks or one redirect. For a redirect, fetch `redirect_target_url` with the given headers. REST returns 200 with the body or 300 with the redirect JSON.

**Upload:**

1. For large objects, call `FetchWriteTypeInfo` and pick the interval that contains the size. Small writes may stream chunks immediately (opportunistic write).
2. Start `Write` with `WriteParameters` (`destination_resource_address`, `data_object_size`, optional `upload_preference`, `previous_version`, `fail_if_exists`).
3. Follow the server's answer:
   - chunks accepted: stream `Chunk` messages, close, read `resource_info`;
   - `resource_info` immediately (zero-byte writes);
   - write redirect: upload to the URL with the given method and headers, then call `CompleteRedirectUpload` with the response headers named in `completion_header_names`;
   - multipart: upload part 1 to `first_part_write_redirect`, call `UploadPart` for the rest within `minimum_size_per_part`/`maximum_size_per_part`/`maximum_parts_number`, then `CompleteMultipartUpload` with each part's returned headers, or `AbortMultipartUpload`.

REST follows the same pattern through `PUT` (201 or 300 with `redirect` or `multipart` in the body).

Send exactly `data_object_size` bytes in a body write. The contract and the conformance suite do not define what happens when the body length differs. Storage Core in this snapshot (and therefore a Storage Service built from it) reads exactly `data_object_size` bytes from a gRPC chunk stream or REST body: a shorter body fails with `INVALID_ARGUMENT`/400 ("request body is too short"), and bytes beyond the declared size are **silently discarded**, so the stored object is truncated without an error ([`storage-core-builder/src/api_services/fileobject/utils.rs`](../../../../storage/reference/storage-core/storage-core-builder/src/api_services/fileobject/utils.rs)). The previous Storage Core snapshot buffered the whole body without comparing it to `data_object_size`, and nothing in this repo shows released 1.2.3 enforcing the size. The change also does not cap `data_object_size` itself. Do not rely on either behavior; compute the size from the bytes you send. Complete runnable clients for every flow live in the reference service's step code under [`conformance_tests/src/conformance_tests/steps/`](../../../../storage/api/conformance_tests/src/conformance_tests/steps/) and the [client example](../../../../storage/api/docs/client_example.md), whose code is in [`examples/python_client/`](../../../../storage/api/examples/python_client/) (`grpc_*.py`, `rest_*.py` for list, upload, download, delete).

Redirect and presigned URLs are bearer capabilities: do not log them, and keep their lifetime short on the adapter side.

## Minimal client calls

Against the local filesystem reference (REST `8011`, gRPC `50051`, plaintext, no authentication; see [develop/custom-storage-adapter/python-backend.md](../develop/custom-storage-adapter/python-backend.md) to start it):

```bash
# List implemented APIs and versions
curl -s http://localhost:8011/v1beta/capabilities/services

# Write 11 bytes, then Stat and read them back
ADDR=$(python3 -c 'import urllib.parse;print(urllib.parse.quote("file-storage://fileservice/test.txt", safe=""))')
curl -s -X PUT "http://localhost:8011/v1beta/fileobject/by-address/${ADDR}?data_object_size=11" \
  -H "Content-Type: application/octet-stream" --data-binary "hello world"
curl -sI "http://localhost:8011/v1beta/fileobject/by-address/${ADDR}"
curl -s "http://localhost:8011/v1beta/fileobject/by-address/${ADDR}?download_preference=body"
```

```python
import grpc
from nvidia.omniverse.storage.fileobject.v1beta import fileobject_service_pb2 as svc
from nvidia.omniverse.storage.fileobject.v1beta import fileobject_service_pb2_grpc as stub

with grpc.insecure_channel("localhost:50051") as channel:
    client = stub.FileObjectServiceStub(channel)
    info = client.Stat(svc.StatRequest(resource_address="file-storage://fileservice/test.txt")).resource_info
    print(info.resource_identity.encoded_identity, info.metadata.data_object_size)
```

The reference service enables gRPC reflection, so `grpcurl -plaintext localhost:50051 list` works against it. A plaintext channel is only for a local reference service. A deployed Storage Service sits behind TLS ingress and Envoy bearer-token authorization ([deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md)); use `grpc.secure_channel` and a token from the deployment's identity provider, never a hard-coded secret.

Generate Python bindings from the definitions matching the target (repeat the `.proto` list for the groups and versions you need):

```bash
cd storage/api
python -m grpc_tools.protoc --proto_path=proto --python_out=. --pyi_out=. --grpc_python_out=. \
  proto/nvidia/omniverse/storage/fileobject/v1beta/fileobject.proto \
  proto/nvidia/omniverse/storage/fileobject/v1beta/fileobject_service.proto \
  proto/nvidia/omniverse/storage/filefolder/v1beta/filefolder_service.proto \
  proto/nvidia/omniverse/storage/capabilities/v1beta/capabilities.proto \
  proto/nvidia/omniverse/storage/versioning/v1beta/versioning.proto
```

## Implementations and their endpoints

| | Python filesystem reference | Storage Service (S3/Azure) |
|---|---|---|
| Purpose | Learning, adapter prototyping, conformance target | Production adapter |
| gRPC | `50051` (`--grpc-port`, `GRPC_SERVER_PORT`) | `8011` (`service.grpc.port`) |
| REST | `8011` (`--http-port`, `HTTP_SERVER_PORT`); `/v1beta` and `/v1alpha` mounts | `8012` (`service.rest.port`); `/v1beta` and `/v1alpha` |
| Metrics / health | none / none | `8013` (`service.metrics.port`, off unless enabled) / gRPC health on `8100` |
| Auth | none | via ingress Envoy auth extension; authorization via Permission Service |
| Discovery | Does **not** implement `nvidia.omniverse.discovery.capabilities.v2alpha`, so the Auto Discovery operator cannot register it unmodified | Implements discovery v2alpha `ListServices`; opt in with Service annotations |
| Detail | [develop/custom-storage-adapter/](../develop/custom-storage-adapter/README.md) | [components/storage-service.md](../components/storage-service.md), [deploy/production-stack.md](../deploy/production-stack.md) |

Ports for the Storage Service come from the snapshot chart [`values.yaml`](../../../../storage/reference/storage-service/helm/values.yaml); read the values of the chart you deploy.

## Validate

- Contract behavior: run the conformance suite ([test/conformance.md](../test/conformance.md#storage-api-conformance-suite)). The Gherkin `.feature` files under [`conformance_tests/src/conformance_tests/features/`](../../../../storage/api/conformance_tests/src/conformance_tests/features/) are the normative behavior specification where the proto and OpenAPI are silent.
- A deployed Storage Service: its Helm smoke test ([test/conformance.md](../test/conformance.md#storage-service-smoke-test)) and the released integration tests ([test/integration-tests.md](../test/integration-tests.md)).

Related: [components/storage-api.md](../components/storage-api.md) (file map), [apis/discovery.md](discovery.md), [apis/permissions.md](permissions.md), [apis/notifications.md](notifications.md) (storage events).
