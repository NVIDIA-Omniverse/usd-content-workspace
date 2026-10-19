<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# usd-content-workspace — modular content services for your existing storage

**usd-content-workspace** is the service foundation of **Omniverse Content Services**, bringing APIs, reference services, tests, and client examples into one product monorepo. Use it to build content workflows on your existing storage and infrastructure.

> **[Start with the Storage API](storage/api/README.md)** · Put access, permissions, identity, metadata, events, endpoint discovery, and previews around the S3 or Azure Blob Storage you already run, through gRPC and REST services you deploy with Helm.
>
> *Pre-release source snapshots. Each component keeps its own version, build workspace, and deployment requirements; there is no combined build or installer, so follow the guide for the component and version you select.*

---

## 1. What is usd-content-workspace?

**usd-content-workspace** brings together the services applications use to access and manage USD content on existing storage. It organizes storage, permissions, user identity, metadata, notifications, endpoint discovery, and thumbnails by API area, so each contract sits next to its reference implementation and its tests. Storage remains the source of the content; these services add access, policy, metadata, events, and previews around it.

Each component is released here as a snapshot from its own source project. The related Omniverse Content Services repositories are [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage), [ovpackage](https://github.com/NVIDIA-Omniverse/ovpackage), [ovlive](https://github.com/NVIDIA-Omniverse/ovlive), [usd-caches](https://github.com/NVIDIA-Omniverse/usd-caches), and [usd-search](https://github.com/NVIDIA-Omniverse/usd-search); see [where it fits](docs/overview/content_services.md).

---

## 2. What functionalities are available, and who are the target users?

**What you can do with it:**

- **Access content on existing storage** — list, read, write, and manage files and folders through the Storage API, with S3 and Azure Blob Storage reference backends and file versions where the backend supports versioning.
- **Compose content workspaces** — use the Storage Service's [manifest overlay](storage/reference/storage-service/docs/helm/manifest.rst) to expose referenced files in a workspace folder alongside its own objects.
- **Apply access policies and resolve identities** — evaluate permissions, authorize bearer tokens through the Envoy auth extension, and look up users and groups through the Microsoft Graph-backed User Info Service.
- **Attach metadata and react to changes** — read and write key/value metadata through the Metadata API, and publish and consume events through the notification services over RabbitMQ.
- **Discover endpoints and generate previews** — query available APIs and versions through Discovery; create and associate thumbnails through the Thumbnail Service and its conversion worker.
- **Build and validate compatible services** — start from the Protobuf and OpenAPI contracts, the conformance suites, the Storage Core Rust framework, and the Navigator UI example.

**Who benefits:**

- **Application and simulation developers** — connect content browsers, scene assembly tools, and USD workflows to shared content services.
- **Platform and storage teams** — deploy the services over existing object storage and integrate identity, permissions, and event delivery.
- **ISVs and service implementers** — implement or adapt a service contract and check its behavior with the matching conformance suite.
- **Developers using coding agents** — open the repository in an agent that reads [AGENTS.md](AGENTS.md), which loads the [usd-content-workspace expert skill](skills/usd-content-workspace-expert/SKILL.md) and the component skills it routes to.

---

## 3. Documentation and reference links

- **Documentation:** [docs/](docs/README.md) — [choosing components](docs/getting_started/choosing_components.md), [connecting to a deployment](docs/getting_started/connect.md), [deploying a service](docs/getting_started/deploy.md), and [implementing a compatible service](docs/getting_started/implement.md).
- **Connect to a deployment:** the [Storage Navigator UI](examples/navigator-ui/README.md) browses and manages content in a browser; embedded applications use [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage).
- **Prebuilt containers and Helm charts:** the [usd-content-workspace collection on NGC](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis). The collection still uses an earlier catalog name, which will change.
- **AI coding agents:** [AGENTS.md](AGENTS.md) and the [skills index](skills/README.md).

**Browse the repository** — each API area has an index page with `api/` (contract), `reference/` (implementation, build, and Helm material), and `conformance/` (tests) where they apply.

| Area | Contract and tests | Implementation |
| --- | --- | --- |
| [**Storage**](storage/README.md) | [API](storage/api/README.md) · [Conformance](storage/conformance/README.md) | [Storage Service](storage/reference/storage-service/README.md) · [Storage Core](storage/reference/storage-core/README.md) · [Azure SDK fork](storage/reference/externals/azure-sdk/) |
| [**Permissions**](permissions/README.md) | [API](permissions/api/README.md) · [Conformance](permissions/conformance/README.md) | [Permission Service](permissions/reference/permission-service/README.md) · [Envoy auth extension](permissions/reference/envoy-auth-extension/README.md) · [Permission UI](permissions/reference/permission-ui/README.md) · [Storage permission UI](permissions/reference/storage-permission-ui/README.md) |
| [**User identity**](user-identity/README.md) | [API](user-identity/api/README.md) · [Conformance](user-identity/conformance/README.md) | [Microsoft Graph User Info Service](user-identity/reference/microsoft-graph-userinfo-service/README.md) |
| [**Metadata**](metadata/README.md) | [API](metadata/api/README.md) | Implemented by the Storage Service (`v1beta`); the API workspace publishes definitions only. |
| [**Notifications**](notifications/README.md) | [API](notifications/api/README.md) · [Usage guide](notifications/reference/event-consumer-service/docs/index.md) | [Event Aggregation Service](notifications/reference/event-aggregation-service/README.md) · [Event Consumer Service](notifications/reference/event-consumer-service/README.md) · [Shared library](notifications/reference/common/README.md) |
| [**Discovery**](discovery/README.md) | [API](discovery/api/README.md) · [Conformance runner](discovery/reference/discovery-service/operator/conformance/README.md) | [Discovery operator](discovery/reference/discovery-service/operator/README.md) · [Legacy discovery chart](discovery/reference/discovery-service/chart/README.md) |
| [**Thumbnails**](thumbnails/README.md) | [API](thumbnails/api/README.md) · [Conformance](thumbnails/conformance/README.md) | [Thumbnail Service and conversion worker](thumbnails/reference/thumbnail-service/README.md) |
| **Shared libraries** (`common/`) | — | [Observability](common/observability/README.md) · [URL normalizer](common/url-normalizer/README.md) |
| [**Client examples**](examples/README.md) | — | [Navigator UI](examples/navigator-ui/README.md) |

Outside the areas: [docs/](docs/README.md) (documentation and license inventory), [skills/](skills/README.md), and the import records [SOURCE_VERSIONS.md](SOURCE_VERSIONS.md), [SOURCE_MANIFEST.json](SOURCE_MANIFEST.json), and [IMPORT_STATUS.md](IMPORT_STATUS.md). For component versions and where changes belong, see [source and releases](docs/overview/source_and_releases.md).

---

## 4. System requirements

Requirements are component-specific; there is no single monorepo installation or build command.

- **Deployment:** the service Helm charts target Kubernetes. Provide the storage, identity provider, databases, and message brokers the selected services require; the notification services use RabbitMQ.
- **Rust services:** the service workspaces pin their toolchains, for example Rust `1.96.1` for the [Storage Service](storage/reference/storage-service/rust-toolchain.toml) and `1.97.0` for the [Discovery operator](discovery/reference/discovery-service/operator/rust-toolchain.toml).
- **Python notification services:** the [aggregation](notifications/reference/event-aggregation-service/pyproject.toml) and [consumer](notifications/reference/event-consumer-service/pyproject.toml) services require Python `>=3.14`.
- **Navigator UI:** [Node.js 22+](examples/navigator-ui/README.md#prerequisites). Other workspaces document their own prerequisites, and some build from the repository root as their Docker context.

---

## 5. Licensing

- **Source:** all NVIDIA-authored code, in every component, is under the [Apache License 2.0](LICENSE). Components carry their own LICENSE files.
- **Third-party components:** keep their original licenses and notices, for example the MIT-licensed [Azure SDK fork](storage/reference/externals/azure-sdk/LICENSE) and the `repo_man` launcher files. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and the [dependency license inventory](docs/licenses/third-party-dependencies.md).
- **Contributions and security:** this project is not currently accepting contributions; see [CONTRIBUTING.md](CONTRIBUTING.md). Participation is covered by the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities through [SECURITY.md](SECURITY.md).
- **Status and support:** the repository is actively developed, and every component is experimental pre-release software: APIs, configuration, and behavior can change between releases without a deprecation period. Report bugs through GitHub Issues; maintainers review them on a best-effort basis, and response times vary. Feature requests are not accepted for now. Repository releases are recorded in [CHANGELOG.md](CHANGELOG.md).

> **Note:** source snapshots can include prerelease or unreleased changes. Do not treat a monorepo revision as a single deployable release.

---

*usd-content-workspace · Omniverse Content Services · Copyright (c) 2026 NVIDIA Corporation.*
