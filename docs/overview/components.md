<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Component map

Each API area has an index page with `api/` (contract), `reference/` (implementation, build, and Helm material), and `conformance/` (tests) where they apply. Some `api/` and `conformance/` directories are indexes into an implementation's build workspace rather than copies of its files.

| Area | Contract and tests | Implementation and usage |
| --- | --- | --- |
| [Storage](../../storage/README.md) | [API](../../storage/api/README.md) · [Conformance](../../storage/conformance/README.md) | [Storage Service](../../storage/reference/storage-service/README.md) · [Storage Core](../../storage/reference/storage-core/README.md) · [Python filesystem reference](../../storage/api/filesystem_example/README.md) · [Azure SDK fork](../../storage/reference/externals/azure-sdk/) |
| [Permissions](../../permissions/README.md) | [API](../../permissions/api/README.md) · [Conformance](../../permissions/conformance/README.md) | [Permission Service](../../permissions/reference/permission-service/README.md) · [Envoy auth extension](../../permissions/reference/envoy-auth-extension/README.md) · [Permission UI](../../permissions/reference/permission-ui/README.md) · [Storage permission UI](../../permissions/reference/storage-permission-ui/README.md) |
| [User identity](../../user-identity/README.md) | [API](../../user-identity/api/README.md) · [Conformance](../../user-identity/conformance/README.md) | [Microsoft Graph User Info Service](../../user-identity/reference/microsoft-graph-userinfo-service/README.md) |
| [Metadata](../../metadata/README.md) | [API guide](../../metadata/api/README.md) · [gRPC contract](../../metadata/api/proto/nvidia/omniverse/metadata/v1beta/metadata.proto) · [REST contract](../../metadata/api/openapi/metadata/v1beta/metadata-api.yaml) | Implemented by the Storage Service (`v1beta`). The API workspace publishes definitions only. |
| [Notifications](../../notifications/README.md) | [Protobuf contracts](../../notifications/api/README.md) · [API usage guide](../../notifications/reference/event-consumer-service/docs/index.md) | [Event Aggregation Service](../../notifications/reference/event-aggregation-service/README.md) · [Event Consumer Service](../../notifications/reference/event-consumer-service/README.md) · [Shared library](../../notifications/reference/common/README.md). Each service carries its OpenAPI specification and conformance suite. |
| [Discovery](../../discovery/README.md) | [API](../../discovery/api/README.md) · [Conformance runner](../../discovery/reference/discovery-service/operator/conformance/README.md) | [Discovery operator and its Helm chart](../../discovery/reference/discovery-service/operator/README.md) · [Legacy discovery chart](../../discovery/reference/discovery-service/chart/README.md) |
| [Thumbnails](../../thumbnails/README.md) | [API](../../thumbnails/api/README.md) · [Conformance](../../thumbnails/conformance/README.md) | [Thumbnail Service and conversion worker](../../thumbnails/reference/thumbnail-service/README.md) |
| Shared libraries (`common/`) | — | [Observability](../../common/observability/README.md) · [URL normalizer](../../common/url-normalizer/README.md) |
| [Client examples](../../examples/README.md) | — | [Navigator UI](../../examples/navigator-ui/README.md) |

## Repository files

| Path | Contents |
| --- | --- |
| [skills/](../../skills/README.md) | Agent skills; the `usd-content-workspace-expert` skill routes to the component skills. |
| [docs/](../README.md) | This documentation, the [dependency license inventory](../licenses/third-party-dependencies.md). |
| [SOURCE_VERSIONS.md](../../SOURCE_VERSIONS.md), `SOURCE_MANIFEST.json`, [IMPORT_STATUS.md](../../IMPORT_STATUS.md) | Component source revisions, file checksums and import status. See [source and releases](source_and_releases.md). |
