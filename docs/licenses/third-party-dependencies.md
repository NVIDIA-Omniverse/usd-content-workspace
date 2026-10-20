<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

> Point-in-time inventory generated on 2026-10-19 from the lockfiles then in the repository; regenerate it when component snapshots change.

# Third-party dependency inventory - usd-content-workspace

Source: every tracked lockfile / pinned manifest in the repo (git ls-files).
Full data: [`third-party-dependencies.csv`](third-party-dependencies.csv) (one row per component x package x version); first-party
packages excluded from it are in first-party packages (not included here).

## Inputs and method

- Cargo.lock (9): common/observability, common/url-normalizer, discovery/reference/discovery-service/operator,
  permissions/reference/envoy-auth-extension, permissions/reference/permission-service, storage/reference/storage-core,
  storage/reference/storage-service, thumbnails/reference/thumbnail-service, user-identity/reference/microsoft-graph-userinfo-service.
  All registry crates come from crates.io; no crate comes from an internal NVIDIA registry
  and no git sources appear in any Cargo.lock. Scope (runtime/build/dev) from `cargo metadata` dep-kind
  reachability on a scratch copy; direct = dependency of a first-party crate in Cargo.lock. Runtime includes all
  target platforms in the lock (e.g. r-efi, windows-*), so some runtime crates are never compiled on Linux.
- package-lock.json (4, lockfileVersion 3): examples/navigator-cli, examples/navigator-ui, permissions/reference/permission-ui,
  permissions/reference/storage-permission-ui. Scope dev = `dev`/`devOptional` in lock. License from lock, else npm registry.
  Every package resolves from registry.npmjs.org.
- uv.lock (6): discovery/reference/discovery-service/chart/conformance_tests, discovery/reference/discovery-service/operator/conformance,
  notifications/reference/event-aggregation-service, notifications/reference/event-aggregation-service/conformance_tests,
  notifications/reference/event-consumer-service, notifications/reference/event-consumer-service/conformance_tests
  (the two notification services lock notification-common as a path dependency; its declared deps are part of the
  service runtime set; dev = the project dev group). poetry.lock (2): storage/api/conformance_tests,
  storage/api/filesystem_example (scope from poetry `groups`). Every package resolves from pypi.org.
- buf.lock (1): notifications/api/protos -> buf.build/protocolbuffers/wellknowntypes (Google protobuf
  well-known types from the public Buf Schema Registry; BSD-3-Clause, recorded manually).
  buf.yaml without a buf.lock (not rows in the CSV, the dependency is unpinned): discovery/api/proto and
  user-identity/reference/microsoft-graph-userinfo-service declare buf.build/protocolbuffers/wellknowntypes (BSD-3-Clause);
  thumbnails/reference/thumbnail-service/proto declares it plus buf.build/googleapis/googleapis (Apache-2.0).
- NO LOCKFILE - resolved from declared ranges with `uv pip compile` (PyPI, today, linux x86_64), NOT a lockfile;
  versions are what a fresh install would pick today and may differ from what is actually deployed:
  - requirements.txt: storage/api/examples/python_client and storage/api/examples/python_service (py3.12, runtime),
    thumbnails/reference/thumbnail-service/tests/e2e (py3.12, dev).
  - pyproject.toml without lock: notifications/reference/common (py3.14; runtime = [project].dependencies,
    dev = [dependency-groups].dev). Its locked resolution as a path dependency appears in the two notification services.
- License sources: local cargo registry cache (Cargo.toml `license`), crates.io API, package-lock `license`, npm
  registry, PyPI JSON (license_expression > license > classifiers). Python/cargo entries with ambiguous metadata
  (20 package versions here) were resolved manually from the shipped LICENSE file (license_source=manual).

## Totals

| ecosystem | unique (package,version) | unique package names | CSV rows (per component) |
|---|---|---|---|
| buf | 1 | 1 | 1 |
| cargo | 1671 | 869 | 3651 |
| npm | 1144 | 731 | 1912 |
| pypi | 289 | 186 | 651 |
| **total** | **3105** | **1787** | **6215** |

Deduplicated unique (ecosystem,package,version) across the repo: **3105** (3055 from lockfiles only; the rest appear only in range-resolved components).

### Per component (unique packages in that component)

| component | ecosystem | resolution | packages | runtime | build | dev | unknown | direct |
|---|---|---|---|---|---|---|---|---|
| common/observability | cargo | lockfile | 220 | 202 | 18 | 0 | 0 | 35 |
| common/url-normalizer | cargo | lockfile | 60 | 60 | 0 | 0 | 0 | 3 |
| discovery/reference/discovery-service/chart/conformance_tests | pypi | lockfile | 13 | 13 | 0 | 0 | 0 | 2 |
| discovery/reference/discovery-service/operator | cargo | lockfile | 329 | 280 | 35 | 14 | 0 | 19 |
| discovery/reference/discovery-service/operator/conformance | pypi | lockfile | 35 | 35 | 0 | 0 | 0 | 7 |
| examples/navigator-cli | npm | lockfile | 331 | 12 | 0 | 319 | 0 | 21 |
| examples/navigator-ui | npm | lockfile | 679 | 70 | 0 | 609 | 0 | 60 |
| notifications/api/protos | buf | lockfile | 1 | 0 | 1 | 0 | 0 | 1 |
| notifications/reference/common | pypi | declared ranges | 117 | 96 | 0 | 21 | 0 | 37 |
| notifications/reference/event-aggregation-service | pypi | lockfile | 129 | 95 | 0 | 34 | 0 | 40 |
| notifications/reference/event-aggregation-service/conformance_tests | pypi | lockfile | 16 | 16 | 0 | 0 | 0 | 5 |
| notifications/reference/event-consumer-service | pypi | lockfile | 132 | 96 | 0 | 36 | 0 | 44 |
| notifications/reference/event-consumer-service/conformance_tests | pypi | lockfile | 16 | 16 | 0 | 0 | 0 | 5 |
| permissions/reference/envoy-auth-extension | cargo | lockfile | 381 | 340 | 41 | 0 | 0 | 43 |
| permissions/reference/permission-service | cargo | lockfile | 518 | 434 | 28 | 56 | 0 | 61 |
| permissions/reference/permission-ui | npm | lockfile | 490 | 54 | 0 | 436 | 0 | 39 |
| permissions/reference/storage-permission-ui | npm | lockfile | 412 | 44 | 0 | 368 | 0 | 30 |
| storage/api/conformance_tests | pypi | lockfile | 101 | 63 | 0 | 38 | 0 | 30 |
| storage/api/examples/python_client | pypi | declared ranges | 10 | 10 | 0 | 0 | 0 | 3 |
| storage/api/examples/python_service | pypi | declared ranges | 20 | 20 | 0 | 0 | 0 | 7 |
| storage/api/filesystem_example | pypi | lockfile | 57 | 30 | 0 | 27 | 0 | 18 |
| storage/reference/storage-core | cargo | lockfile | 591 | 552 | 37 | 2 | 0 | 95 |
| storage/reference/storage-service | cargo | lockfile | 712 | 635 | 53 | 24 | 0 | 125 |
| thumbnails/reference/thumbnail-service | cargo | lockfile | 404 | 384 | 12 | 8 | 0 | 55 |
| thumbnails/reference/thumbnail-service/tests/e2e | pypi | declared ranges | 5 | 0 | 0 | 5 | 0 | 1 |
| user-identity/reference/microsoft-graph-userinfo-service | cargo | lockfile | 436 | 390 | 23 | 23 | 0 | 45 |

## License distribution (unique package versions)

| license expression | count |
|---|---|
| MIT | 1438 |
| MIT OR Apache-2.0 | 854 |
| Apache-2.0 | 199 |
| Apache-2.0 OR MIT | 147 |
| Unicode-3.0 | 91 |
| BSD-3-Clause | 69 |
| ISC | 53 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 28 |
| BSD-2-Clause | 28 |
| Unlicense OR MIT | 21 |
| MPL-2.0 | 19 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 16 |
| Apache-2.0 OR ISC OR MIT | 15 |
| Apache-2.0 AND MIT | 11 |
| CC0-1.0 | 11 |
| Zlib | 9 |
| Zlib OR Apache-2.0 OR MIT | 9 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 6 |
| PSF-2.0 | 6 |
| MIT OR Apache-2.0 OR Zlib | 6 |
| (MIT OR CC0-1.0) | 5 |
| BlueOak-1.0.0 | 4 |
| MIT-0 | 4 |
| Apache-2.0 OR BSL-1.0 | 3 |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | 3 |
| CC-BY-4.0 | 3 |
| (MIT OR Apache-2.0) AND Apache-2.0 | 3 |
| Apache-2.0 OR BSL-1.0 OR MIT | 3 |
| 0BSD OR MIT OR Apache-2.0 | 2 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | 2 |
| Apache-2.0 OR BSD-2-Clause | 2 |
| ISC AND (Apache-2.0 OR ISC) | 2 |
| ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | 2 |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 2 |
| CDLA-Permissive-2.0 | 2 |
| 0BSD | 2 |
| Apache-2.0 OR BSD-3-Clause | 2 |
| LicenseRef-Public-Domain AND BSD-2-Clause AND PSF-2.0 | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| MIT AND BSD-3-Clause | 1 |
| MIT OR Zlib OR Apache-2.0 | 1 |
| Apache-2.0 AND ISC | 1 |
| Python-2.0 | 1 |
| (Apache-2.0 OR MPL-1.1) | 1 |
| Unlicense | 1 |
| Apache-2.0 AND BSD-3-Clause | 1 |
| Apache-2.0 WITH LLVM-exception | 1 |
| bzip2-1.0.6 | 1 |
| BSL-1.0 | 1 |
| ISC AND MIT AND OpenSSL | 1 |
| 0BSD OR CC0-1.0 | 1 |
| CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | 1 |
| Unlicense OR MIT OR Apache-2.0 OR CC0-1.0 | 1 |
| MPL-2.0+ | 1 |
| CC0-1.0 OR Apache-2.0 | 1 |
| (MIT OR Apache-2.0) AND NCSA | 1 |
| MIT OR Apache-2.0 OR BSD-1-Clause | 1 |
| CC0-1.0 OR MIT-0 | 1 |

Copyleft flag (unique package versions): none=3085, weak=20, strong=0, unknown=0

## Copyleft packages (strong + weak)

| package | version | license | flag | ecosystem | component | scope | direct | how used |
|---|---|---|---|---|---|---|---|---|
| axe-core | 4.10.3 | MPL-2.0 | weak | npm | examples/navigator-ui | dev | no | dev-only, via eslint-plugin-jsx-a11y (lint) in navigator-ui |
| certifi | 2026.10.18 | MPL-2.0 | weak | pypi | thumbnails/reference/thumbnail-service/tests/e2e | dev | no | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| certifi | 2026.10.18 | MPL-2.0 | weak | pypi | notifications/reference/common; storage/api/examples/python_client | runtime | no | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| certifi | 2026.2.25 | MPL-2.0 | weak | pypi | discovery/reference/discovery-service/chart/conformance_tests; notifications/reference/event-aggregation-service; notifications/reference/event-aggregation-service/conformance_tests; notifications/reference/event-consumer-service/conformance_tests | runtime | no | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| certifi | 2026.2.25 | MPL-2.0 | weak | pypi | notifications/reference/event-consumer-service | runtime | yes | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| certifi | 2026.7.22 | MPL-2.0 | weak | pypi | storage/api/filesystem_example | dev | no | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| certifi | 2026.7.22 | MPL-2.0 | weak | pypi | discovery/reference/discovery-service/operator/conformance; storage/api/conformance_tests | runtime | no | Mozilla CA bundle, unmodified; transitive via requests/httpx/httpcore; direct only in event-consumer-service (`certifi>=2026.2.25` in pyproject; imported by public_scripts/apply_queue_policy.py Helm hook) |
| colored | 3.1.1 | MPL-2.0 | weak | cargo | user-identity/reference/microsoft-graph-userinfo-service | dev | no | dev-only, via mockito 1.7.2 (dev-dependency of microsoft-graph-userinfo-service tests) |
| lightningcss | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-android-arm64 | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-darwin-arm64 | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-darwin-x64 | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-freebsd-x64 | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-linux-arm-gnueabihf | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-linux-arm64-gnu | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-linux-arm64-musl | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-linux-x64-gnu | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-linux-x64-musl | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-win32-arm64-msvc | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| lightningcss-win32-x64-msvc | 1.33.0 | MPL-2.0 | weak | npm | examples/navigator-cli | dev | no | dev-only, via vite 8.3.1 (peer of vitest 5.0.3, the navigator-cli test runner); only the one platform binary matching the host is installed |
| pathspec | 1.0.4 | MPL-2.0 | weak | pypi | notifications/reference/event-aggregation-service; notifications/reference/event-consumer-service | dev | no | transitive via black / mypy (lint/type-check tooling); in storage/api/conformance_tests poetry "main" group because mypy is a main dep |
| pathspec | 1.1.1 | MPL-2.0 | weak | pypi | notifications/reference/common; storage/api/filesystem_example | dev | no | transitive via black / mypy (lint/type-check tooling); in storage/api/conformance_tests poetry "main" group because mypy is a main dep |
| pathspec | 1.1.1 | MPL-2.0 | weak | pypi | storage/api/conformance_tests | runtime | no | transitive via black / mypy (lint/type-check tooling); in storage/api/conformance_tests poetry "main" group because mypy is a main dep |
| smartstring | 1.0.1 | MPL-2.0+ | weak | cargo | storage/reference/storage-service | runtime | no | runtime, via opentelemetry-prometheus-text-exporter 0.2.1 -> storage-service binary (statically linked, unmodified) |

### Dual/multi-licensed with a copyleft option (flag=none because a permissive option exists)

| package | version | license | components | scope |
|---|---|---|---|---|
| harmony-reflect | 1.6.2 | (Apache-2.0 OR MPL-1.1) | 1: examples/navigator-ui | dev |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | 7: common/observability, discovery/reference/discovery-service/operator, permissions/reference/envoy-auth-extension, storage/reference/storage-core, storage/reference/storage-service, thumbnails/reference/thumbnail-service, user-identity/reference/microsoft-graph-userinfo-service | runtime |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | 6: discovery/reference/discovery-service/operator, permissions/reference/envoy-auth-extension, permissions/reference/permission-service, storage/reference/storage-core, thumbnails/reference/thumbnail-service, user-identity/reference/microsoft-graph-userinfo-service | build/runtime |

## Unknown or non-SPDX licenses

| ecosystem | package | version | license | source | note |
|---|---|---|---|---|---|
| pypi | docutils | 0.21.2 | LicenseRef-Public-Domain AND BSD-2-Clause AND PSF-2.0 | manual | public domain + BSD-2-Clause + PSF; GPL-3.0+ only for sdist-only emacs rst.el (not in wheel); dev-only (sphinx/myst docs) |
| pypi | docutils | 0.22.4 | LicenseRef-Public-Domain AND BSD-2-Clause AND PSF-2.0 | manual | public domain + BSD-2-Clause + PSF; GPL-3.0+ only for sdist-only emacs rst.el (not in wheel); dev-only (sphinx/myst docs) |

Other non-trivial (valid SPDX, permissive) entries worth noting: ring 0.16.20 declares only a license-file;
read from cache as `ISC AND MIT AND OpenSSL` (OpenSSL/SSLeay BSD-style); ring 0.17.14 declares `Apache-2.0 AND ISC`.
smartstring declares deprecated `MPL-2.0+`.

## Specific checks

- **aiostream (GPL-3.0-or-later)**: REMOVED. It was a direct runtime dependency of notifications/reference/event-consumer-service; it is no longer declared in that service's pyproject.toml nor present in its uv.lock, and is not imported anywhere in this repository.
- **certifi (MPL-2.0)**: CONFIRMED. Present in 11 component(s): notifications/reference/common 2026.10.18 runtime direct=no; storage/api/examples/python_client 2026.10.18 runtime direct=no; thumbnails/reference/thumbnail-service/tests/e2e 2026.10.18 dev direct=no; discovery/reference/discovery-service/chart/conformance_tests 2026.2.25 runtime direct=no; notifications/reference/event-aggregation-service 2026.2.25 runtime direct=no; notifications/reference/event-aggregation-service/conformance_tests 2026.2.25 runtime direct=no; notifications/reference/event-consumer-service 2026.2.25 runtime direct=yes; notifications/reference/event-consumer-service/conformance_tests 2026.2.25 runtime direct=no; discovery/reference/discovery-service/operator/conformance 2026.7.22 runtime direct=no; storage/api/conformance_tests 2026.7.22 runtime direct=no; storage/api/filesystem_example 2026.7.22 dev direct=no.
- **progenitor / progenitor-client (MPL-2.0)**: ABSENT. They are no longer declared in any Cargo.toml (the two unused
  `[workspace.dependencies]` entries in storage/reference/storage-service were removed) and appear in no Cargo.lock.
- **Other GPL/AGPL/LGPL/MPL/EPL/CDDL/SSPL/BUSL/Commons-Clause hits**: weak (MPL-2.0) - pathspec (Python, dev/lint),
  axe-core (npm, dev), lightningcss and its 12 per-platform binary packages (npm, dev, via vite 8 in navigator-cli),
  smartstring (cargo, runtime in storage-service), colored (cargo, dev). No AGPL, LGPL-only, EPL, CDDL, SSPL, BUSL or
  Commons-Clause anywhere. Copyleft appears only as an alternative in dual licenses: r-efi
  (MIT OR Apache-2.0 OR LGPL-2.1-or-later), harmony-reflect (Apache-2.0 OR MPL-1.1, dev), docutils (GPL only for an
  sdist-only emacs file).

## First-party (excluded from the CSV)

| component | ecosystem | first-party packages (workspace / path members / project roots) |
|---|---|---|
| common/observability | cargo | axum-observability, otel-asserts, otel-tools, protos, testing, tonic-observability |
| common/url-normalizer | cargo | url_normalizer |
| discovery/reference/discovery-service/chart/conformance_tests | pypi | discovery-conformance-tests |
| discovery/reference/discovery-service/operator | cargo | auto-discovery-service |
| discovery/reference/discovery-service/operator/conformance | pypi | discovery-conformance |
| examples/navigator-cli | npm | @storage-navigator/cli |
| examples/navigator-ui | npm | storage-navigator |
| notifications/reference/common | pypi | notification-common |
| notifications/reference/event-aggregation-service | pypi | event-aggregation-service, notification-common |
| notifications/reference/event-aggregation-service/conformance_tests | pypi | event-aggregation-conformance-tests |
| notifications/reference/event-consumer-service | pypi | event-consumer-service, notification-common |
| notifications/reference/event-consumer-service/conformance_tests | pypi | event-consumer-conformance-tests |
| permissions/reference/envoy-auth-extension | cargo | envoy-auth-ext, otel-tools, permission-auth, permission-core, tonic-observability |
| permissions/reference/permission-service | cargo | axum-observability, otel-tools, permission-auth, permission-core, permission-db, permission-service, permission-service-capabilities-proto, permission-service-conformance-test, permission-service-notification-proto, permission-service-proto, permission-service-publishing, permission-smoke-test |
| permissions/reference/permission-ui | npm | permission-ui |
| permissions/reference/storage-permission-ui | npm | @omniverse/storage-permission-ui |
| storage/api/conformance_tests | pypi | omniverse-storageapi-conformance-tests |
| storage/api/filesystem_example | pypi | omniverse-filesystem-storage-service |
| storage/reference/storage-core | cargo | basic-service, boxed-builders, greeter-service, manifest-service, remapper-service, storage-core, storage-core-builder, storage-core-integration-tests, storage-core-protos, storage-core-test, storage-core-types, storage-core-util, traffic-generator |
| storage/reference/storage-service | cargo | storage-core, storage-core-builder, storage-core-protos, storage-core-types, storage-core-util, storage-service, storage-service-grpc, storage-service-metrics, storage-service-permissions, storage-service-protos, storage-service-test, storage-service-util, url_normalizer |
| thumbnails/reference/thumbnail-service | cargo | axum-observability, discovery-proto, notifications-proto, otel-tools, permission-auth, storage-proto, thumbnail-auth, thumbnail-common, thumbnail-conformance-test, thumbnail-convert, thumbnail-proto, thumbnail-service, thumbnail-smoke-test, thumbnail-task-worker, tonic-observability |
| user-identity/reference/microsoft-graph-userinfo-service | cargo | microsoft-graph-userinfo-capabilities-proto, microsoft-graph-userinfo-licensing, microsoft-graph-userinfo-publishing, microsoft-graph-userinfo-service, microsoft-graph-userinfo-smoke-test, userinfo-conformance-test |

**First-party-vendored third-party: `storage/reference/externals/azure-sdk`** - NVIDIA fork of Azure SDK for Rust
(upstream MIT). Path/[patch.crates-io] crates in use: azure_core 0.21.0 (storage/reference/storage-core, storage/reference/storage-service); azure_data_cosmos 0.21.1 (storage/reference/storage-service); azure_identity 0.21.0 (storage/reference/storage-core, storage/reference/storage-service); azure_messaging_servicebus 0.21.0 (storage/reference/storage-service); azure_storage 0.21.0 (storage/reference/storage-core, storage/reference/storage-service); azure_storage_blobs 0.21.4 (storage/reference/storage-core, storage/reference/storage-service); azure_svc_blobstorage 0.21.1 (storage/reference/storage-core, storage/reference/storage-service).
Note: the committed Cargo.lock of storage-core (azure_core 0.21.0, azure_identity 0.21.0, azure_storage 0.21.0) and storage-service (azure_core 0.21.0, azure_identity 0.21.0, azure_messaging_servicebus 0.21.0, azure_storage 0.21.0)
still records those crates as crates.io packages (MIT) although the Cargo.toml points to the vendored path (the lock
is stale; `cargo --locked` fails); those 7 rows are kept in the CSV and marked "stale" in the resolution column.

CSV extra column `resolution`: `lockfile` or `declared-ranges (uv pip compile, not a lockfile)`.
