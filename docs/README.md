<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Documentation

The usd-content-workspace documentation is written in Markdown and reStructuredText and is meant to be read directly in the repository. Back to the [usd-content-workspace README](../README.md).

## Getting started

- [Choosing components](getting_started/choosing_components.md) — decide which services your workflow needs and the order to stand them up.
- [Connect to a deployment](getting_started/connect.md) — browse content on an existing deployment with the Navigator UI, or embed access with ovstorage.
- [Deploy a service](getting_started/deploy.md) — deploy the reference services to Kubernetes with their Helm charts.
- [Implement a compatible service](getting_started/implement.md) — build against a contract and check it with the conformance suite.

## Overview

- [Component map](overview/components.md) — every contract, implementation, and conformance link.
- [Where usd-content-workspace fits](overview/content_services.md) in Omniverse Content Services.
- [Source and releases](overview/source_and_releases.md) — where component changes belong and how versions work.

## AI coding agents

- [AI coding agents](agents.md) — the agent skills in this repository.

## Other files

- [`index.rst`](index.rst) — the landing page and table of contents of the documentation set.
- [`licenses/`](licenses/third-party-dependencies.md) — the third-party dependency license inventory.

Components keep their own documentation in their directories; the [component map](overview/components.md) links to it.
