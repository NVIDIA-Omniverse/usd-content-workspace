<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Source and releases

Each component is developed in its own source project. Released source arrives here as snapshot pull requests that apply the source project's export rules and replace the component's destination directory. Component changes therefore belong in the source project; an edit made under a component directory is overwritten by the next snapshot.

Maintained directly in this repository:

- the root `README.md`, `AGENTS.md`, `CLAUDE.md`, `llms.txt`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and `SECURITY.md`;
- this documentation (`docs/`) and the `skills/` directory;
- the area index pages (`<area>/README.md`, and the `api/` and `conformance/` indexes that point into an implementation's workspace);
- repo-layout adaptations needed for components to build in this layout, recorded in [SOURCE_VERSIONS.md](../../SOURCE_VERSIONS.md).

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for the full rules.

## Versions

There is no single monorepo release. Use each component's package manifest, documentation, and changelog for its version and requirements, for example the [Storage Service](../../storage/reference/storage-service/CHANGELOG.md), [Metadata API](../../metadata/api/CHANGELOG.md), [User Info Service](../../user-identity/reference/microsoft-graph-userinfo-service/CHANGELOG.md), and notification [aggregation](../../notifications/reference/event-aggregation-service/CHANGELOG.md) and [consumer](../../notifications/reference/event-consumer-service/CHANGELOG.md) changelogs. Snapshots can include prerelease or unreleased changes.

[SOURCE_VERSIONS.md](../../SOURCE_VERSIONS.md) lists each component's version, source tag and source commit; `SOURCE_MANIFEST.json` records the checksum of every file in each component; [IMPORT_STATUS.md](../../IMPORT_STATUS.md) summarizes where the components come from, their licensing, and which builds CI verifies.
