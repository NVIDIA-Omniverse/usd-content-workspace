<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Import status

[SOURCE_VERSIONS.md](SOURCE_VERSIONS.md) lists each component's version, source tag and source commit. SOURCE_MANIFEST.json records the SHA-256 of every file in each component directory, and any file edited here after its snapshot.

## Sources

Every component directory except the three below is a snapshot written by its source project's release job: the job filters the source through the project's publish rules (ignore file, prepare and verify scripts) and replaces the destination directory. A component directory differs from its latest snapshot only by the files listed under Layout adaptations in [SOURCE_VERSIONS.md](SOURCE_VERSIONS.md).

- `discovery/reference/discovery-service/chart`, the legacy static Discovery chart, is still the initial import (2.4.1-rc.5). Its README has since been edited here.
- `common/url-normalizer` and `storage/reference/externals/azure-sdk` (a fork of the Azure SDK for Rust) were added by hand so that the Storage Service builds by path in this layout. Their source commits were not recorded.

Notifications include the API protos, `notification_common` 7.0.0, the event consumer service 1.12.0 and the event aggregation service 1.7.0. Both services pin `notification-common==7.0.0` and resolve it from `notifications/reference/common`.

Not part of this repository: the Nucleus migration tool, the archive service, the `simple-nginx` image used by the legacy Discovery chart, and deployment configuration.

## Licensing and publish rules

- NVIDIA-authored components are licensed under Apache-2.0 ([LICENSE](LICENSE)). Third-party code keeps its own license, for example the MIT-licensed Azure SDK fork; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- Protobuf modules take their dependencies from the public Buf registry (`buf.build`), except the Permission Service: its `buf.yaml` and proto-crate `buf.gen.yaml` files still reference NVIDIA's private Buf registry, which `buf lint` and `buf generate` need. Its Rust build uses the generated code committed in the component.
- No Cargo manifest depends on a private registry: first-party crates resolve by path within this repository. The observability crates' `publish` key still names an NVIDIA registry; it only affects `cargo publish`.
- References to publicly reachable NVIDIA endpoints are kept: NGC (`nvcr.io`), the Omniverse content servers, the packman bootstrap, and the `nvidia.omniverse.*` protobuf packages.

## Build verification

The workflows in [.github/workflows](.github/workflows) run `cargo check` and the tests for the Rust components when they change: Storage Service (with storage-core, url-normalizer and the Azure SDK fork), storage-core, Permission Service, Envoy auth extension, Microsoft Graph User Info Service, the observability crates, and Thumbnail Service. They also build the container images of Storage Service, Permission Service, the Envoy auth extension and Thumbnail Service. The other components (the notification services and library, the Discovery operator and chart, the navigator UI and CLI, the permission UIs, and the API and conformance workspaces) are not built by this repository's workflows; use their own README build instructions.
