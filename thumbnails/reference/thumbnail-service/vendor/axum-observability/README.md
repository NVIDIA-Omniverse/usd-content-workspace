<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# axum-observability (vendored)

Tracing and metric layers for Axum-based HTTP servers.

This is a vendored copy of `axum-observability` 0.14.0 from NVIDIA's tonic-observability repository, vendored so that
thumbnail-service builds without NVIDIA's internal Cargo registry. Only the license headers, this
README and the manifest metadata (`authors`, `license`, `publish`, `repository`, and path dependencies on the
other vendored crates), and the local changes below differ from the released crate.

The source is published, under Apache-2.0, at [common/observability](https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/common/observability) in the
consolidated repository.

## Local changes

The published 0.14.0 package contains unit tests that reference unpublished upstream
workspace helpers. `src/test_support.rs` provides the small body and no-op meter
fixtures those tests need, using existing dependencies. The body retains the
upstream fixture's size hint (42), stream state, and polling behavior.

When upgrading, replace upstream sources and retain or re-evaluate the manifest
edits and local changes above. Run this crate's unit and documentation tests
separately from the service workspace with
`cargo test --manifest-path vendor/axum-observability/Cargo.toml`.
Standalone test lockfiles and target directories are local build artifacts; the
service build continues to use the repository-root `Cargo.lock`.
