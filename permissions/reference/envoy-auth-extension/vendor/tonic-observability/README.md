<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# tonic-observability (vendored)

Trace and metric layers for Tonic-based gRPC clients and servers.

This is an unmodified copy of `tonic-observability` from NVIDIA's tonic-observability workspace at commit `ec17cde` of
its main branch (release 0.14.0 plus the changes merged since), vendored so that envoy-auth-extension builds
without NVIDIA's internal Cargo registry. `src/` is exactly what `cargo package` produces at that commit; only
this README and the manifest metadata (`publish`, `include`, the dropped `homepage`, and path dependencies on
the other vendored crates) differ.

The source workspace is published, under Apache-2.0, at [common/observability](../../../../../common/observability) in the consolidated repository.

When upgrading, replace `src/` with the packaged crate from the new upstream commit and keep the manifest edits above.
