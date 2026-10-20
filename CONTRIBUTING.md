<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Contributing

This project is currently not accepting external contributions.

## Where changes go

Each service, API and library in this repository is developed in its own source project. Released versions arrive here as squashed snapshots that replace the component's directory. To change a component, change it in its source project. An edit made directly under a component directory will be overwritten by the next snapshot.

Changes that belong in this repository directly:

- Repository governance: the root `README.md`, `CHANGELOG.md`, `AGENTS.md`, `CLAUDE.md`, `llms.txt`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md` and `SECURITY.md`.
- The documentation under `docs/` and the agent skills under `skills/`.
- Area index pages: `<area>/README.md`, `examples/README.md`, `storage/skills/`, and the `api/` and `conformance/` indexes that point into an implementation's workspace (for permissions, thumbnails, user identity, storage conformance, and metadata conformance).
- Repo-layout adaptations needed for components to build in this layout. Record them in `SOURCE_VERSIONS.md`.

## Pull requests

- Maintainers: if this checkout contains `docs/plc/MAINTAINING.md`, follow it for lifecycle metadata, CI and review routing.

## Licensing

Components keep their own licenses. Check the LICENSE file in the component's directory before copying code between components. New files at the repository root carry SPDX headers.
