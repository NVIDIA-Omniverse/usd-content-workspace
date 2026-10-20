# API contract versions

> **Applies to:** every API in the stack · **Sources:** `.proto` and OpenAPI files in this repository (read 2026-10-12), vendored proto `VERSION` files, NGC API resource versions in [released-inventory.md](released-inventory.md)

An API has three version labels that change independently:

- **Wire version** — the proto package or REST path segment (`nvidia.omniverse.storage.fileobject.v1beta`, `/api/v1beta/...`). Clients bind to it; a service can serve several at once.
- **Document version** — the OpenAPI `info.version`, a changelog entry, or the API workspace's release tag (for example Storage API `1.0.0-beta.4`). It moves when definitions change within a wire version.
- **Resource version** — the NGC API resource a customer downloads (for example `storage-api` 1.0.0-beta.4). It can lag or lead the published HTML docs.

Within an `alpha` or `beta` wire version, fields and RPCs can still be added, and occasionally changed. Generate clients from the definitions the target service was built with, and check the Discovery catalog for the wire versions it actually serves.

## Contracts in this repository

| API | Wire packages | Definitions | Document version (snapshot) | Released resource |
| --- | --- | --- | --- | --- |
| Storage | `nvidia.omniverse.storage.{fileobject,filefolder,capabilities,versioning}.v1alpha` and `.v1beta`; `nvidia.omniverse.storage.metadata.v1alpha` | [`storage/api/proto`](../../../../storage/api/proto/), [`storage/api/openapi`](../../../../storage/api/openapi/) | unreleased (after 1.0.0-beta.4) | `storage-api` 1.0.0-beta.4 |
| Metadata (top level) | `nvidia.omniverse.metadata.v1beta` | [`metadata/api/proto`](../../../../metadata/api/proto/), [`metadata/api/openapi`](../../../../metadata/api/openapi/) | 0.3.0 | not a separate NGC resource |
| Discovery capabilities | `nvidia.omniverse.discovery.capabilities.v2alpha` (gRPC; what every service here serves); `...capabilities.v2beta` in the unreleased operator only; REST capabilities `v1beta` | [`discovery/api/proto`](../../../../discovery/api/proto/), operator [`discovery-protos/`](../../../../discovery/reference/discovery-service/operator/discovery-protos/README.md) and [`openapi/`](../../../../discovery/reference/discovery-service/operator/openapi/) | untagged main | — |
| Discovery catalog | REST `GET /api/v2/services` (released); the snapshot operator's primary route is `/api/v2beta/services` with `/api/v2/services` as an alias; `schema-version: 2` | operator [`openapi/discovery.openapi.yaml`](../../../../discovery/reference/discovery-service/operator/openapi/discovery.openapi.yaml), [catalog API doc](../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md) | v2beta | — |
| Notifications publisher | `nvidia.omniverse.notifications.publisher.v1beta` | [`event_publisher.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto), aggregation [`openapi.yaml`](../../../../notifications/reference/event-aggregation-service/openapi.yaml) | notification-protos manual snapshot; OpenAPI `v1beta` | `notifications-aggregation-api` 1.0.0-beta.3 |
| Notifications consumer | `nvidia.omniverse.notifications.consumer.v1beta` | [`event_consumer.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto), consumer [`openapi.yaml`](../../../../notifications/reference/event-consumer-service/openapi.yaml) | OpenAPI `v1beta` | `notifications-consumer-api` 1.0.0-beta.3 |
| Permissions | `nvidia.omniverse.permission.v1beta` | [`permission-service/protos`](../../../../permissions/reference/permission-service/protos/), [`openapi.json`](../../../../permissions/reference/permission-service/openapi.json) | OpenAPI 0.0.53 | `permission-api` 1.0.0-beta.1 |
| User Info | `nvidia.omniverse.userinfo.v1alpha` | [`userinfo protos`](../../../../user-identity/reference/microsoft-graph-userinfo-service/protos/), [`openapi.yaml`](../../../../user-identity/reference/microsoft-graph-userinfo-service/openapi.yaml) | OpenAPI `v1alpha` | `userinfo-api` 1.0.0-alpha.1 |
| Thumbnails | `nvidia.omniverse.thumbnail.v1alpha` | [`thumbnail-service/proto`](../../../../thumbnails/reference/thumbnail-service/proto/), [`thumbnail.openapi.yaml`](../../../../thumbnails/reference/thumbnail-service/thumbnail.openapi.yaml) | 0.1.0 | no public API resource |
| Live Edit | `nvidia.omniverse.live.v1alpha.LiveService` (gRPC only) | [ovlive `live.proto`](https://github.com/NVIDIA-Omniverse/ovlive) (vendored in the client) | — | released service 0.1.1 |

## Copies of contracts inside implementations

Services compile their own copies of the contracts they call, pinned in `VERSION` files. A contract change in `api/` reaches a service only when that service updates its pin.

| Copy | Pinned to |
| --- | --- |
| Storage Service [`storage-service-protos/proto/nvidia/omniverse/*/VERSION`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/) | storage, permission, and notifications protos by Buf registry commit; discovery protos by `discovery/api` commit |
| Storage Core [`storage-core-protos/proto/storage-protos/VERSION`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/VERSION) | storage-protos tag 0.28.0, plus hand-applied `native_metadata` hunks from unreleased Storage API source and local `IsReadOnly` / `IsVersioned` RPCs not yet in any tag. The file warns that re-vendoring from the tag deletes them; storage-core conformance derives its version from the `tag:` line |
| Discovery operator [`discovery-protos/`](../../../../discovery/reference/discovery-service/operator/discovery-protos/) | its own discovery-protos snapshot |
| Notification services `nvidia/` directories | generated Python bindings for the notification protos |
| Thumbnail Service `crates/proto/*` | storage, discovery, notifications, and thumbnail protos |

To check whether a service and the API workspace agree, compare the pinned commit or tag with the API workspace's history, or diff the copied `.proto` files against the `api/` directory.

Known divergence in this snapshot: the Storage Service's copy of Metadata v1beta does not match `metadata/api` 0.3.0. `ResourceLocation` field 2 has a different type, and the REST routes for get and update differ. Clients for the Storage Service must use the service's own copy; see [../apis/metadata.md](../apis/metadata.md#grpc-wire-mismatch-in-resourcelocation).

## What a deployment advertises

The Discovery catalog lists, per discovered service, the REST resources with their version keys and the fully qualified gRPC service names, for example `nvidia.omniverse.storage.fileobject.v1beta.FileObjectService`. That is the authoritative list of wire versions a client can call in that environment. See [capture-a-deployment.md](capture-a-deployment.md).

## Rules for answers and generated code

- Name the wire package and the definitions path you used, for example "generated from `storage/api/proto` from unreleased Storage API source at commit `<sha>`, package `...fileobject.v1beta`".
- If the caller's service is the released 1.2.3 Storage Service, warn that snapshot-only fields might not be served; prefer the released resource (1.0.0-beta.4) or verify against the catalog.
- Read the API changelog between the two versions before upgrading clients: [Storage API](../../../../storage/api/CHANGELOG.md), [Metadata API](../../../../metadata/api/CHANGELOG.md).
- Per-API detail: [../apis/](../apis/) (storage, metadata, discovery, notifications, permissions, userinfo, thumbnails, live-edit).
