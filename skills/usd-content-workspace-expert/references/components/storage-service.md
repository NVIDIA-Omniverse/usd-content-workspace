# Storage Service — component map

> **Applies to:** NVIDIA Storage Service (S3 / Azure Blob Storage production adapter) · **Released:** chart and image 1.2.3 (Developer Guide 1.3.0; published docs at `https://docs.omniverse.nvidia.com/ovstorage/storage-service/1.2.3/`) · **This repo's snapshot:** 1.2.3 + `[Unreleased]` ([`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md); workspace version in [`Cargo.toml`](../../../../storage/reference/storage-service/Cargo.toml); source commit in [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md); `[Unreleased]` includes an unreleased Storage Core update) · **Sources:** in-repo skill [`skills/storage-service/SKILL.md`](../../../../storage/reference/storage-service/skills/storage-service/SKILL.md) (upstream storage-service skill, identical content); component files linked below

**Start with the in-repo skill:** [`storage/reference/storage-service/skills/storage-service/SKILL.md`](../../../../storage/reference/storage-service/skills/storage-service/SKILL.md). It routes each configuration question to a page under `docs/helm/` and lists upgrade traps. Its links are relative to `storage/reference/storage-service/`. Its frontmatter says version 1.2.2; the snapshot is 1.2.3 + unreleased changes. This page adds what that skill does not cover.

## What the in-repo skill omits

| Topic | Where |
| --- | --- |
| URL normalizer (on by default in the snapshot; changes address resolution) | [`docs/helm/url-normalizer.rst`](../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst) |
| Metadata API `nvidia.omniverse.metadata.v1beta` served alongside `nvidia.omniverse.storage.metadata.v1alpha` (1.2.3) | [`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md), [../apis/metadata.md](../apis/metadata.md) |
| Per-endpoint `readOnly` and `IsReadOnly`/`IsVersioned` RPCs (1.2.3) | [`docs/helm/storage-backend.rst`](../../../../storage/reference/storage-service/docs/helm/storage-backend.rst) |
| `service.rest.serveOpenapi` (opt out of serving OpenAPI documents) | [`docs/helm/misc.rst`](../../../../storage/reference/storage-service/docs/helm/misc.rst) |
| `observability.headers.forward` (copy request headers to permission-service calls; unreleased) | [`docs/helm/observability.rst`](../../../../storage/reference/storage-service/docs/helm/observability.rst) |
| Unreleased Storage Core behavior: body writes read exactly `data_object_size` bytes (short bodies rejected, long ones truncated); `IsVersioned` fails with `NOT_FOUND` for an endpoint strict mode does not serve; the `Versioning` trait method it calls changed shape (matters only if you patch the service's layers) | [storage-core.md](storage-core.md#released-versus-snapshot), [../apis/storage.md](../apis/storage.md#upload-and-download-flows) |
| Discovery registration (Service annotations; headless Service duplicate) | [../deploy/production-stack/storage-service.md](../deploy/production-stack/storage-service.md#register-with-discovery) |
| Smoke tests (`helm test`, 15 REST + 15 gRPC tests) | [`SMOKE_TESTS.md`](../../../../storage/reference/storage-service/SMOKE_TESTS.md) |
| Building the image (repo-root context) and CI | [../deploy/building-from-source.md](../deploy/building-from-source.md#storage-service) |
| Local tests and their environment | [../deploy/local-development/component-dev-loops.md](../deploy/local-development/component-dev-loops.md#storage-service) |
| End-to-end deployment in the stack | [../deploy/production-stack.md](../deploy/production-stack.md) |

## Files

| Kind | Path |
| --- | --- |
| Deployment guide (example walkthrough; not the source of truth) | [`docs/deployment.rst`](../../../../storage/reference/storage-service/docs/deployment.rst) |
| Chart value reference (authoritative) | [`docs/helm/index.rst`](../../../../storage/reference/storage-service/docs/helm/index.rst) and its pages |
| Migration between chart versions | [`docs/helm/migration.rst`](../../../../storage/reference/storage-service/docs/helm/migration.rst) |
| Helm chart | [`helm/`](../../../../storage/reference/storage-service/helm/Chart.yaml) (`Chart.yaml` `0.0.0`; `values.yaml`; templates for Deployment, Service, headless Service, HTTPProxy, Certificate, smoke tests) |
| Dashboards | [`helm/files/dashboards/`](../../../../storage/reference/storage-service/helm/files/dashboards/) |
| Protos vendored for the build | [`storage-service-protos/proto/nvidia/omniverse/`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/) (`discovery`, `metadata`, `notifications`, `permission`, `storage`, each with a `VERSION` pin) |
| Storage API contract (protos, OpenAPI, conformance) | [`storage/api/`](../../../../storage/api/README.md); map: [storage-api.md](storage-api.md) |
| Storage Core (backend library the service builds on) | [`storage/reference/storage-core/`](../../../../storage/reference/storage-core/README.md); map: [storage-core.md](storage-core.md) |
| Changelog | [`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md) |
| Container build | [`Dockerfile`](../../../../storage/reference/storage-service/Dockerfile) |
| Lint, build, test targets | [`Makefile`](../../../../storage/reference/storage-service/Makefile) |

## Facts

| | Value | Source |
| --- | --- | --- |
| Ports | gRPC 8011, REST 8012, Prometheus 8013 (`service.metrics.enabled`), gRPC health probe 8100 | `values.yaml`, `templates/storage-service/deployment.yaml` |
| Service port names | `grpc`, `rest` (and `metrics`) | `templates/storage-service/service.yaml` |
| REST capability routes | `/v1alpha/capabilities/services`, `/v1beta/capabilities/services` (storage capabilities); `/v2alpha/services` (discovery capabilities) | Storage Core builder; `storage-service/src/api/discovery/mod.rs` |
| gRPC discovery | `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService` (since 1.2.0) | `CHANGELOG.md` |
| Default pod security | non-root UID/GID 1000, all capabilities dropped, no privilege escalation (1.2.3) | `values.yaml` |
| Toolchain | Rust 1.96.1 | `rust-toolchain.toml` |
| Path dependencies | `../storage-core`, `../externals/azure-sdk`, `../../../common/url-normalizer` | `Cargo.toml`, `Dockerfile` |

## Upgrade traps not in the in-repo skill

- **URL normalization defaults to on** in the snapshot (unreleased). Audit stored keys for empty path segments and `.`/`..` segments, or set `config.urlNormalizer.enabled: false`.
- **Body writes must match `data_object_size`** in the snapshot (unreleased Storage Core changes): shorter bodies fail with `INVALID_ARGUMENT`/400, longer ones are stored truncated without an error.
- **Snapshot chart `image.tag` is `"0.0.0"`** with no `appVersion` fallback. Always set it.
- **`serviceIdentity.clientCredentials` is deprecated** from 1.2.1 in favor of the access-token-file provisioner ([`migration.rst`](../../../../storage/reference/storage-service/docs/helm/migration.rst)).
- **Flat pre-1.2.0 AWS credential keys were removed**; use `credentials.accessKey.*` and `credentials.assumeRole.*`.

## Known documentation defects in this snapshot

- The `[Unreleased]` Storage Core update and this workspace's `Cargo.lock` record newer storage-core and storage-core-builder versions than the sibling Storage Core snapshot it builds against by path, whose `Cargo.toml` files were not bumped. The [`Dockerfile`](../../../../storage/reference/storage-service/Dockerfile) mounts the source read-only and runs `cargo vendor --locked`, so the lock must be reconciled before `docker build`; CI does this with `cargo metadata` ([`docker-image.yml`](../../../../.github/workflows/docker-image.yml)). See [../deploy/building-from-source.md](../deploy/building-from-source.md#storage-service).
- `values.yaml` comments cite `docs/helm/storage-backend.md`, `storage-permissions.md`, and `url-remapper.md`; the files are `.rst`.
- `docs/deployment.rst` uses `extraEnv:` in the PostgreSQL example; the chart key is `extraEnvs`.
- `README.md` contains a placeholder changelog block (`[0.9.9] - 2077-01-01`) and points to `scripts/env.sh`, `scripts/local-bootstrap.sh`, `scripts/local-teardown.sh`, which are not present; `make lint` calls two missing `scripts/ci-check-*.sh` scripts.
- The HTTPProxy streaming-route list names `/nvidia.omniverse.storage.fileobject.v1alpha.FileFolderService/List` and `...fileobject.v1beta.FileFolderService/List`; FileFolderService is in the `filefolder` package, so those two entries never match and `List` uses the standard timeout policy.
