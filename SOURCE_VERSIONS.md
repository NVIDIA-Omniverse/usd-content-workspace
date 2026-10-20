<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Source versions

Each component directory is a snapshot of its source project. The table lists, for every component, the version its package manifest or changelog declares, and the source tag and full source commit of the snapshot now in this repository. `untagged` means the snapshot was taken from a branch commit rather than a release tag.

| Component | Declared version | Source tag | Source commit |
| --- | --- | --- | --- |
| common/observability | `0.14.0` | untagged | `ec17cdeadc88326776c88b30a18b8257df231984` |
| common/url-normalizer | `0.1.0` | not recorded | not recorded |
| discovery/api | — | untagged | `a5f1df256d29a4b5aab43b99050f0390ffca06f4` |
| discovery/reference/discovery-service/chart | `2.4.1-rc.5` | `2.4.1-rc.5` | `aa91f16da8b0a44fe4b105ccd58e5d1ada056402` |
| discovery/reference/discovery-service/operator | `1.2.0` | `1.2.0-rc.6` | `c16c080c801225672aced01365aab1529cf32a9d` |
| examples/navigator-cli | `0.1.1` | untagged | `2caba896e62bf79afda7f792d63938ef555fcb4b` |
| examples/navigator-ui | `1.0.4` | untagged | `83269087eafe52d338cb160cd39aef3e29c22e4a` |
| metadata/api | `0.3.0` | untagged | `41e6e343a9aedcc523f763eae8845fb2f1a6ef6a` |
| notifications/api | — | untagged | `38b13644bf4e132b3739c425dc07f96e5e2a7097` |
| notifications/reference/common | `7.0.0` | `7.0.0` | `482e717334ea338736deb890bd1106852b848cc9` |
| notifications/reference/event-aggregation-service | `1.7.0` | untagged | `47c7f62bda188c5b0b11566d5d5f67755292a31f` |
| notifications/reference/event-consumer-service | `1.12.0` | untagged | `b2764a5bd2ffe02e264a35031dd2b2746f11cbe9` |
| permissions/reference/envoy-auth-extension | `2.4.1` | untagged | `5d47450ae913396834ad802cb5c7535ae9a6d761` |
| permissions/reference/permission-service | `0.0.54` | untagged | `5505cfa52d4c27029e58e5586dbad31653c89b19` |
| permissions/reference/permission-ui | `0.0.23` | untagged | `a0d11d585b8a3988b1b7350c8dc2acf64a1b1d82` |
| permissions/reference/storage-permission-ui | `0.0.24` | untagged | `f5871f45848993ecf13615e2ca5d8233a470c0e7` |
| storage/api | `1.0.0-beta.6` | untagged | `0d287bd15e1eeb1f7fea5e4597b0e23006392d02` |
| storage/reference/externals/azure-sdk | — | untagged | `6fa4bd47fe5000881982b75d07416419200ddb5d` |
| storage/reference/storage-core | `0.11.0-alpha.1` | untagged | `1a7de88f57a53bd8abc447322a428e97295372ad` |
| storage/reference/storage-service | `1.2.3` | untagged | `0ce69bc8a8666e628dbb22d4f1b4d756911b99d7` |
| thumbnails/reference/thumbnail-service | `0.1.0` | untagged | `f73b1c42b66d70ba8b660189408d6cd195a16bb2` |
| user-identity/reference/microsoft-graph-userinfo-service | `0.0.9` | untagged | `67500ad5db1be8a5a4aca5efb0eeef7078b85307` |

Each component's file list and SHA-256 checksums are in SOURCE_MANIFEST.json. Original licenses and notices remain with their components.

`common/url-normalizer` and `storage/reference/externals/azure-sdk` (a fork of the Azure SDK for Rust) let the Storage Service build by path in this layout; they were added by hand and no release snapshot has replaced them yet. The discovery chart is still the initial import.

## Layout adaptations

Component snapshots resolve their first-party dependencies by path within this repository; each source project's publish step applies those rewrites, so they are not local edits. Links to this repository use its public name, `NVIDIA-Omniverse/usd-content-workspace`.

Files edited in this repository after their snapshot, with the original and current checksums in SOURCE_MANIFEST.json:

- `common/observability`: `crates/axum-observability/Cargo.toml`, `crates/otel-asserts/Cargo.toml`, `crates/otel-tools/Cargo.toml`, `crates/tonic-observability/Cargo.toml`
- `common/url-normalizer`: `Cargo.toml`, `LICENSE`, `examples/basic_usage.rs`, `src/lib.rs`, `src/main.rs`
- `discovery/reference/discovery-service/chart`: `README.md`
- `examples/navigator-ui`: `docs/app/docs/index.md`
- `metadata/api`: `openapi/metadata/v1beta/metadata-api.yaml`
- `notifications/reference/common`: `CHANGELOG.md`
- `notifications/reference/event-aggregation-service`: `AGENTS.md`, `CHANGELOG.md`, `README.md`
- `notifications/reference/event-consumer-service`: `AGENTS.md`, `CHANGELOG.md`, `README.md`, `helm/README.md`
- `permissions/reference/envoy-auth-extension`: `attributions-2026-10-13.md`, `vendor/otel-tools/Cargo.toml`, `vendor/otel-tools/README.md`, `vendor/permission-auth/Cargo.toml`, `vendor/permission-auth/README.md`, `vendor/permission-core/Cargo.toml`, `vendor/permission-core/README.md`, `vendor/tonic-observability/Cargo.toml`, `vendor/tonic-observability/README.md`
- `permissions/reference/permission-service`: `docs/service/docs/index.md`
- `storage/api`: `AGENTS.md`, `PROMPTS.md`, `README.md`, `conformance_tests/README.md`, `docs/changes.rst`, `docs/index.rst`, `filesystem_example/src/local_filesystem_service/__main__.py`, `filesystem_example/src/local_filesystem_service/grpc_service/__init__.py`, `filesystem_example/src/local_filesystem_service/grpc_service/__main__.py`, `filesystem_example/src/local_filesystem_service/rest_service/__main__.py`, `openapi/capabilities/v1alpha/capabilities-api.yaml`, `openapi/capabilities/v1beta/capabilities-api.yaml`, `openapi/filefolder/v1alpha/filefolder-api.yaml`, `openapi/filefolder/v1beta/filefolder-api.yaml`, `openapi/fileobject/v1alpha/fileobject-api.yaml`, `openapi/fileobject/v1beta/fileobject-api.yaml`, `openapi/metadata/v1alpha/metadata-api.yaml`, `openapi/versioning/v1alpha/versioning-api.yaml`, `openapi/versioning/v1beta/versioning-api.yaml`, `skills/storage-api-contract-map/SKILL.md`, `templates/production_impl/README.md`, `templates/python_backend/README.md`, `templates/python_backend/__init__.py`, `templates/python_backend/my_storage_provider.py`
- `storage/reference/storage-core`: `Cargo.toml`
- `storage/reference/storage-service`: `Dockerfile`, `docs/deployment.rst`, `docs/index.rst`, `helm/Chart.yaml`
- `thumbnails/reference/thumbnail-service`: `Dockerfile.conformance-test`, `docker/Dockerfile`, `vendor/axum-observability/Cargo.toml`, `vendor/axum-observability/README.md`, `vendor/otel-tools/Cargo.toml`, `vendor/otel-tools/README.md`, `vendor/permission-auth/Cargo.toml`, `vendor/permission-auth/README.md`, `vendor/tonic-observability/Cargo.toml`, `vendor/tonic-observability/README.md`
- `user-identity/reference/microsoft-graph-userinfo-service`: `docs/api/docs/index.md`, `docs/api/docs/use-cases.md`
