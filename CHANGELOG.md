<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Changelog

This file records releases of the usd-content-workspace repository as a whole. A repository release is a tagged snapshot of every component directory at once. It is not a combined build: each component keeps its own version, changelog and release cadence, and the versions in a repository release are the ones recorded in [SOURCE_VERSIONS.md](SOURCE_VERSIONS.md) at the tagged commit.

Repository releases are tagged `vMAJOR.MINOR.PATCH`, with an `-rc.N` suffix for release candidates. While the repository is in pre-release, a minor version can contain breaking changes in any component.

## [Unreleased] - 0.1.0

First public release of usd-content-workspace, as experimental pre-release source.

### Added

- APIs, reference services, conformance suites and client examples for Omniverse Content Services, organized by API area:

  | Area | Components | Declared version |
  | --- | --- | --- |
  | Storage | [Storage API](storage/api/README.md) | `1.0.0-beta.6` |
  | | [Storage Service](storage/reference/storage-service/README.md) | `1.2.3` |
  | | [Storage Core](storage/reference/storage-core/README.md) | `0.11.0-alpha.1` |
  | | [Azure SDK fork](storage/reference/externals/azure-sdk/) | — |
  | Permissions | [Permission Service](permissions/reference/permission-service/README.md) | `0.0.54` |
  | | [Envoy auth extension](permissions/reference/envoy-auth-extension/README.md) | `2.4.1` |
  | | [Permission UI](permissions/reference/permission-ui/README.md) | `0.0.23` |
  | | [Storage permission UI](permissions/reference/storage-permission-ui/README.md) | `0.0.24` |
  | User identity | [Microsoft Graph User Info Service](user-identity/reference/microsoft-graph-userinfo-service/README.md) | `0.0.9` |
  | Metadata | [Metadata API](metadata/api/README.md) | `0.3.0` |
  | Notifications | [Notification APIs](notifications/api/README.md) | — |
  | | [Event Aggregation Service](notifications/reference/event-aggregation-service/README.md) | `1.7.0` |
  | | [Event Consumer Service](notifications/reference/event-consumer-service/README.md) | `1.12.0` |
  | | [Notification shared library](notifications/reference/common/README.md) | `7.0.0` |
  | Discovery | [Discovery API](discovery/api/README.md) | — |
  | | [Discovery operator](discovery/reference/discovery-service/operator/README.md) | `1.2.0` |
  | | [Legacy discovery chart](discovery/reference/discovery-service/chart/README.md) | `2.4.1-rc.5` |
  | Thumbnails | [Thumbnail Service and conversion worker](thumbnails/reference/thumbnail-service/README.md) | `0.1.0` |
  | Shared libraries | [Observability](common/observability/README.md) | `0.14.0` |
  | | [URL normalizer](common/url-normalizer/README.md) | `0.1.0` |
  | Client examples | [Navigator UI](examples/navigator-ui/README.md) | `1.0.4` |
  | | [Navigator CLI](examples/navigator-cli/README.md) | `0.1.1` |

  A dash means the component declares no version; [SOURCE_VERSIONS.md](SOURCE_VERSIONS.md) records its source commit. Snapshots can include changes made after the declared version.
- Agent skills for the stack, starting from the [usd-content-workspace expert skill](skills/usd-content-workspace-expert/SKILL.md).
- Documentation under [docs/](docs/README.md): getting-started guides, an overview and component map, and the third-party dependency license inventory.
- Repository files for public release: [LICENSE](LICENSE) (Apache-2.0 for all NVIDIA-authored code), [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

### Component changes

Changes inside a component are recorded in its own changelog, where it keeps one:

- Storage: [Storage API](storage/api/CHANGELOG.md), [Storage Service](storage/reference/storage-service/CHANGELOG.md), and the Storage Core crates, for example [storage-core](storage/reference/storage-core/storage-core/CHANGELOG.md).
- Permissions: [Permission Service](permissions/reference/permission-service/CHANGELOG.md), [Envoy auth extension](permissions/reference/envoy-auth-extension/CHANGELOG.md), [Permission UI](permissions/reference/permission-ui/CHANGELOG.md).
- User identity: [Microsoft Graph User Info Service](user-identity/reference/microsoft-graph-userinfo-service/CHANGELOG.md).
- Metadata: [Metadata API](metadata/api/CHANGELOG.md).
- Notifications: [Event Aggregation Service](notifications/reference/event-aggregation-service/CHANGELOG.md), [Event Consumer Service](notifications/reference/event-consumer-service/CHANGELOG.md), [shared library](notifications/reference/common/CHANGELOG.md).
- Discovery: [legacy discovery chart](discovery/reference/discovery-service/chart/CHANGELOG.md).
- Shared libraries: [Observability](common/observability/CHANGELOG.md).
- Client examples: [Navigator UI](examples/navigator-ui/CHANGELOG.md).

The Discovery operator, the Discovery and Notification APIs, the Thumbnail Service, the Storage permission UI, the Navigator CLI and the URL normalizer keep no changelog; their README and the source commit in [SOURCE_VERSIONS.md](SOURCE_VERSIONS.md) identify the snapshot.
