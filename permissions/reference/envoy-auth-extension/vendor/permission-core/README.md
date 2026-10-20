<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# permission-core (vendored)

A library crate from the permission-service workspace.

This is an unmodified copy of `permission-core` 0.0.54-rc.3 from NVIDIA's permission-service workspace, vendored so that
envoy-auth-extension builds without NVIDIA's internal Cargo registry. Only the license headers,
this README and the manifest metadata (`license`, `publish`, `repository`, `readme`, and
path dependencies on the other vendored crates) differ from the released crate.

The source workspace is published, under Apache-2.0, at [permissions/reference/permission-service](../../../permission-service) in the consolidated repository.

When upgrading, replace `src/` with the new release and keep the manifest edits above.
