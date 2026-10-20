<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# otel-tools (vendored)

OpenTelemetry helpers and setup.

This is an unmodified copy of `otel-tools` 0.14.0 from NVIDIA's tonic-observability repository, vendored so that
thumbnail-service builds without NVIDIA's internal Cargo registry. Only the license headers, this
README and the manifest metadata (`authors`, `license`, `publish`, `repository`, and path dependencies on the
other vendored crates) differ from the released crate.

The source is published, under Apache-2.0, at [common/observability](https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/common/observability) in the
consolidated repository.

When upgrading, replace `src/` with the new release and keep the manifest edits above.
