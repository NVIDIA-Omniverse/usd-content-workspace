# Capabilities, API versions and storage modes

> **Applies to:** what a custom storage adapter advertises and which Storage API versions and optional behaviors it serves · **Released:** `storage-api` **1.0.0-beta.4**; Auto Discovery **1.1.1** · **This repo's snapshot:** Storage API **unreleased (after 1.0.0-beta.4)**; Discovery capabilities proto `v2alpha` ([`discovery/api/proto/`](../../../../../discovery/api/proto/), untagged `origin/main` per [`SOURCE_VERSIONS.md`](../../../../../SOURCE_VERSIONS.md)) · **Sources:** [`storage/api/proto/.../capabilities/`](../../../../../storage/api/proto/nvidia/omniverse/storage/capabilities/), [`conformance_tests/`](../../../../../storage/api/conformance_tests/), [`filesystem_example/`](../../../../../storage/api/filesystem_example/), discovery operator [`README.md`](../../../../../discovery/reference/discovery-service/operator/README.md)

## Two different capabilities APIs

An adapter answers two `ListServices` calls with different packages and different purposes. Do not confuse them.

| | Storage capabilities | Discovery capabilities |
|---|---|---|
| Package | `nvidia.omniverse.storage.capabilities.v1beta` (and `.v1alpha`) | `nvidia.omniverse.discovery.capabilities.v2alpha` |
| Caller | Storage clients and the conformance suite | The Auto Discovery operator |
| Response | `ServiceEntry { service_name, service_versions[] }`, e.g. `fileobject` → `["v1alpha","v1beta"]` | `ServiceEntry { service_name, grpc_services[].endpoints[].service_name, rest_services[].endpoints[] { name, versions[] { version, url_detail.path } } }` with pagination |
| Effect if missing | Every conformance scenario for that group **skips** (the suite checks capabilities before each scenario) | The operator never lists the service in `/api/v2/services` |
| Implemented by the Python reference | yes | **no** |
| Implemented by the Storage Service | yes (from the registered Storage Core services) | yes ([`api/discovery/`](../../../../../storage/reference/storage-service/storage-service/src/api/discovery/)) |

How to add the Discovery API and annotate the Kubernetes Service: [develop/add-your-service.md](../add-your-service.md#expose-discovery-capabilities). The Discovery API reference: [apis/discovery.md](../../apis/discovery.md).

## What to serve

| Surface | Must | Should | Notes |
|---|---|---|---|
| v1beta FileObject, FileFolder, Versioning, Capabilities | yes | | The stable contract |
| v1alpha of the same groups | | if clients need `Copy`, `Move`, `CreateFolder`, `GetFolderMode`, `ListRoutes`, capability queries | v1alpha is a superset of v1beta; serving it means serving v1beta's RPCs under v1alpha too |
| v1alpha Metadata | | if clients store key/value metadata | Or serve the top-level `nvidia.omniverse.metadata.v1beta` ([apis/metadata.md](../../apis/metadata.md)) |

Advertise exactly what you serve. The conformance suite reads `ListServices` (trying `v1alpha`, `v1beta`, then `v1`) and skips scenarios for groups or versions you do not list, so an over-advertised version produces failures and an under-advertised one produces silent skips. The Storage Core builder derives `ListServices` from the registered services; a hand-written adapter must keep the list in sync itself.

### Top-level addresses and routes

- `ListTopLevelAddresses`: the roots clients browse, for example `file-storage://fileservice` or `https://<bucket>.s3.<region>.amazonaws.com`. The conformance suite needs `TEST_STORAGE_API_RESOURCE_BASE` to be under one of them.
- v1alpha `ListRoutes`: wildcard patterns for every address and identity scheme you serve, so a client or router with several adapters can pick one without a request. The reference returns its base URI and its identity scheme, each with `/**`. Matching rules are in [`capabilities/v1alpha/capabilities.proto`](../../../../../storage/api/proto/nvidia/omniverse/storage/capabilities/v1alpha/capabilities.proto): `*` matches one `[a-zA-Z0-9_-]+` segment, a trailing `**` matches descendants, the most specific route wins, ties go to the lexicographically first pattern.

## Folder modes

Pick one mode that matches the backend and report it through v1alpha `GetFolderMode`.

| Mode (`FolderMode`) | Backend shape | Reference flag |
|---|---|---|
| `FOLDER_MODE_NATIVE` | Real directories; empty folders persist until `DeleteFolder` | `--folder-mode native` |
| `FOLDER_MODE_NO_EMPTY` | Folders exist only through objects (S3 keys, Git) | `--folder-mode no_empty` |
| `FOLDER_MODE_HYBRID` | Explicit folders persist; implicit ones vanish with their last object | `--folder-mode placeholder` |

The reference uses the mode to choose some error codes (for example `NOT_FOUND` versus `INVALID_ARGUMENT` for file operations on a folder in hybrid mode); the `.feature` files decide.

## Versioned, unversioned and read-only storage

| Mode | v1alpha query | Contract (current conformance suite) | Reference |
|---|---|---|---|
| Versioned | `IsVersioned` → `true` | Every write creates a version; identities from `Write`, redirect completion and `Copy` stay bound to their bytes; `EnumerateVersions` lists them | default |
| Unversioned | `IsVersioned` → `false` | One current version; deletes are hard; a superseded identity returns `NOT_FOUND`/404, never newer bytes | `FILESERVICE_VERSIONING_MODE=unversioned` |
| Read-only | `IsReadOnly` → `true` | Write, delete, move, folder and metadata mutations are rejected (reference: `FAILED_PRECONDITION`/412) | `FILESERVICE_READ_ONLY=true` |

Both queries are per address: a service routing several backends can answer differently per route. Storage Service 1.2.3 serves both, with a per-endpoint `readOnly` setting. The mode-specific scenarios run only when the suite sees the same environment variables ([test/conformance.md](../../test/conformance.md#storage-api-conformance-suite)).

## Optimistic concurrency

- `previous_version` on `Write` and `Delete` (and source/destination versions on `Move`): the operation succeeds only if the current identity matches; mismatch returns `FAILED_PRECONDITION`/412. Check it atomically at commit time; unreleased source fixed a race in the reference for direct, redirect and multipart writes.
- `fail_if_exists` on `Write`, `Copy`, `Move` (unreleased): `ALREADY_EXISTS`/409 if the destination exists.
- v1alpha `GetOptimisticLockingSupport` reports which of these you honor (`supports_write`, `supports_delete`, `supports_copy`, `supports_move`, and the `*_fail_if_exists` flags). Report `false` rather than silently ignoring a precondition.

## Native metadata (v1alpha, unreleased)

Native metadata is stored with one object version (`WriteParameters.native_metadata`, returned on `Stat`, `Read`, `ReadFromAddress`, `Enumerate`, and `ListStat` when `include_native_metadata` is set). Rules from [`conformance_tests/README.md`](../../../../../storage/api/conformance_tests/README.md#native-metadata-conformance):

- Answer `GetNativeMetadataSupport` per exact address.
- If unsupported, reject non-empty native metadata before mutating anything: `UNIMPLEMENTED`/501. Malformed addresses and authorization failures keep their usual `INVALID_ARGUMENT`/400 and `PERMISSION_DENIED`/403 precedence.
- You may add backend-generated entries (an ETag); client-supplied entries must survive on the written version.
- Older services without the support endpoint may ignore native metadata; clients must not infer support from an empty field.

## Targeting a released versus a snapshot contract

If clients will run against released 1.2.3 Storage Services alongside your adapter, implement the beta.4 surface and treat the unreleased additions as optional extras you advertise only once implemented. If you build from this snapshot, record the exact definitions you generated from (the commit and `pyproject.toml` version) in your adapter's release notes, and regenerate when you move. The release history is in [`storage/api/CHANGELOG.md`](../../../../../storage/api/CHANGELOG.md); the version model is in [versions/api-contracts.md](../../versions/api-contracts.md).
