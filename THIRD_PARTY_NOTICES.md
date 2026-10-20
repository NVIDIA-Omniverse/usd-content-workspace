<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Third-party notices

NVIDIA-authored code in this repository is licensed under the Apache License 2.0 (see [LICENSE](LICENSE)). This file lists the third-party code that ships in source form in this repository, and the copyleft-licensed packages that components download through their package managers.

Each component also keeps its own attribution file listing its package dependencies, for example `ATTRIBUTIONS.md`. A full inventory of direct and transitive package dependencies with their licenses is in [docs/licenses/third-party-dependencies.md](docs/licenses/third-party-dependencies.md).

## 1. Third-party source code in this repository

### Azure SDK for Rust (unofficial, legacy)

- **Location:** `storage/reference/externals/azure-sdk/`
- **License:** MIT. Copyright (c) Microsoft Corporation.
- **Upstream:** https://github.com/Azure/azure-sdk-for-rust, `legacy` branch, based on commit `25ebe5a599a88f311e28a0b63102ce318998f8b6` (2024-10-09), with NVIDIA modifications.
- **Notices:** the original license headers are kept. The upstream [LICENSE](storage/reference/externals/azure-sdk/LICENSE) and [NOTICE.txt](storage/reference/externals/azure-sdk/NOTICE.txt) in that directory apply.

The fork contains this further third-party code:

- **autorust_openapi 0.2.0**
  - Location: `storage/reference/externals/azure-sdk/services/autorust/openapi/`
  - License: MIT. Copyright (c) 2020 Cameron Taggart and Copyright (c) 2017 Doug Tangren. See its [LICENSE](storage/reference/externals/azure-sdk/services/autorust/openapi/LICENSE).
  - Origin: a fork of the `openapi` crate (https://github.com/softprops/openapi).
- **openapiv3 status codes**
  - Location: `src/status_code.rs` in that crate.
  - License: MIT. Covered by the "License notice for glademiller/openapiv3" section of the fork's NOTICE.txt.
  - Origin: adapted from https://github.com/glademiller/openapiv3. The upstream authors did not record which revision they used.

### Envoy, xDS, protoc-gen-validate and Google API protobuf definitions

- **Location:** `permissions/reference/envoy-auth-extension/proto/vendor/`
- **License:** Apache-2.0
- **Upstream:** exported unmodified with `buf export` from the Envoy v1.33.0 API module (`envoyproxy/envoy:v1.33.0`, path `envoy/service/auth/v3/external_auth.proto`) together with its dependencies:

  | Directory | Upstream project |
  |---|---|
  | `envoy/` | https://github.com/envoyproxy/envoy (data-plane API) |
  | `udpa/`, `xds/` | https://github.com/cncf/xds |
  | `validate/` | https://github.com/bufbuild/protoc-gen-validate |
  | `google/rpc/status.proto` | https://github.com/googleapis/googleapis (Copyright 2024 Google LLC) |

### gRPC health checking protocol

- **Location:** `permissions/reference/envoy-auth-extension/tools/health.proto`
- **License:** Apache-2.0. Copyright 2015 The gRPC Authors.
- **Upstream:** https://github.com/grpc/grpc-proto

### Cedar syntax highlighting grammar

- **Location:** `permissions/reference/permission-ui/public/js/hljs-cedar.min.js`
- **License:** Apache-2.0. Copyright Cedar Contributors.
- **Upstream:** https://github.com/cedar-policy

### Contour deployment manifest

- **Location:** `discovery/reference/discovery-service/operator/deploy/local/contour-v1.33.5.yaml`
- **License:** Apache-2.0. Copyright Project Contour Authors.
- **Upstream:** https://github.com/projectcontour/contour, rendered v1.33.5 example deployment.
- **Use:** local development only.

### Tabler Icons webfont

- **Location:** `examples/navigator-ui/src/assets/tabler-icons/` (`tabler-icons.min.css` and `fonts/`)
- **License:** MIT. Copyright (c) 2020-2026 Paweł Kuna.
- **Upstream:** https://github.com/tabler/tabler-icons, release 3.44.0, vendored unmodified.
- **License text:**

  ```text
  MIT License

  Copyright (c) 2020-2026 Paweł Kuna

  Permission is hereby granted, free of charge, to any person obtaining a copy
  of this software and associated documentation files (the "Software"), to deal
  in the Software without restriction, including without limitation the rights
  to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  copies of the Software, and to permit persons to whom the Software is
  furnished to do so, subject to the following conditions:

  The above copyright notice and this permission notice shall be included in all
  copies or substantial portions of the Software.

  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
  IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
  AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
  SOFTWARE.
  ```

### Contributor Covenant Code of Conduct

- **Location:** `CODE_OF_CONDUCT.md`
- **License:** CC-BY-SA-4.0. Contributor Covenant is stewarded by the Organization for Ethical Source.
- **Upstream:** https://www.contributor-covenant.org/version/3/0/, version 3.0, as adapted in an NVIDIA open-source repository template. NVIDIA changes are limited to the project name. The attribution and license link are kept in the file.

### repo_man launcher and bootstrapper (MIT exception)

- **Locations:**
  - `storage/api/tools/repoman/repoman_bootstrapper.py`
  - `metadata/api/tools/repoman/repoman_bootstrapper.py`
  - `storage/reference/storage-service/docs/tools/repoman/repoman_bootstrapper.py`
  - `discovery/reference/discovery-service/operator/docs/api/`: `repo.sh`, `repo.bat` and `tools/repoman/repoman_bootstrapper.py`
  - `discovery/reference/discovery-service/operator/docs/service/`: `repo.sh`, `repo.bat` and `tools/repoman/repoman_bootstrapper.py`
- **License:** MIT. Copyright (c) 2019-2026 NVIDIA CORPORATION & AFFILIATES, as stated in each file's SPDX header.
- **Origin:** NVIDIA repo_man build tooling, vendored unmodified, as in other public NVIDIA-Omniverse repositories. It is refreshed from that shared source, so the copies are snapshots taken at different times and are not byte-identical.
- **Exception:** these files stay under MIT, not Apache-2.0. The rest of the packman and repo_man bootstrap in the same directories is Apache-2.0.

## 2. Weak-copyleft (MPL-2.0) package dependencies

None of these packages are vendored or modified in this repository. Package managers download them from the public registries. Their source code is available at the upstream locations below.

| Package | License | Used by | Scope | Source |
|---|---|---|---|---|
| certifi | MPL-2.0 | Python components (CA bundle via requests/httpx) | runtime and test | https://github.com/certifi/python-certifi |
| smartstring 1.0.1 | MPL-2.0+ | `storage/reference/storage-service`, via opentelemetry-prometheus-text-exporter | runtime | https://github.com/bodil/smartstring |
| pathspec | MPL-2.0 | Python conformance tests and dev tooling (mypy, black) | dev and test | https://github.com/cpburnz/python-pathspec |
| colored | MPL-2.0 | `user-identity/reference/microsoft-graph-userinfo-service`, via mockito | test only | https://github.com/mackwic/colored |
| axe-core | MPL-2.0 | `examples/navigator-ui`, via eslint-plugin-jsx-a11y | lint only | https://github.com/dequelabs/axe-core |
| lightningcss 1.33.0 and its per-platform binary packages | MPL-2.0 | `examples/navigator-cli`, via vite (used by the vitest test runner) | test only | https://github.com/parcel-bundler/lightningcss |

## 3. Strong-copyleft package dependencies

No component depends on a GPL, AGPL or LGPL-only package. `aiostream` (GPL-3.0-or-later) was previously a runtime dependency of `notifications/reference/event-consumer-service`; it has been removed and is no longer declared or imported by any component in this repository.
