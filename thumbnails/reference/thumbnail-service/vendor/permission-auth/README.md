<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# permission-auth (vendored)

Authentication and authorization client for the Permission Service.

This copy of `permission-auth` 0.0.51-rc.17 from NVIDIA's permission-service repository is vendored so that
thumbnail-service builds without NVIDIA's internal Cargo registry. The license headers, this
README and the manifest metadata (`license`, `publish`, `repository`, and path dependencies on the
other vendored crates) are adapted for publication.

Local fixes, covered by unit tests:

- JWT-only discovery accepts omitted token and userinfo endpoints; modes that use userinfo validate its endpoint at startup.
- Token-file deletion, read failures and malformed replacements clear cached credentials, with recovery when a valid file returns.
- Userinfo rate limits and server failures are retryable errors and do not cache a valid bearer as rejected. Error responses are logged by status only.

The source is published, under Apache-2.0, at [permissions/reference/permission-service](https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/permissions/reference/permission-service) in the
consolidated repository.

When upgrading, retain these fixes and regression tests until the upstream release includes them, along with the manifest edits above.
