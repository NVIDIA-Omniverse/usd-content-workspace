# Building images, binaries, and charts from this repository

> **Applies to:** every component with a Dockerfile or Helm chart in this repository · **Released:** not applicable; released images come from NGC ([artifact-access.md](artifact-access.md)) · **This repo's snapshot:** Dockerfiles, `rust-toolchain.toml` files, and workflows at commit `189896e`; the component matrix builder and runtime columns and the notification commands re-checked at `189896e`, the other commands at `367f7ed` · **Sources:** component Dockerfiles and READMEs linked below; [`.github/workflows/`](../../../../.github/workflows/); [`.dockerignore`](../../../../.dockerignore); [`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md); notification `BUILDING.md` files

Build from source when you need a snapshot change that is not released or you must patch a component. You then own image scanning, signing, publishing, and upgrade testing. [`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md) lists which components this repository's CI checks, tests and builds images for (below); the others are not built here.

## Rules that apply to every build

- **Build context.** Several Rust workspaces have path dependencies outside their directory, so their images build with the **repository root** as context and `-f <component Dockerfile>`. Others build from their own directory. The table says which. The root [`.dockerignore`](../../../../.dockerignore) keeps the root context small and must not exclude `storage/reference/externals/azure-sdk`.
- **Toolchains are per component.** Each Rust workspace has its own `rust-toolchain.toml`; rustup honors it. CI deliberately does not pin a central toolchain ([`rust-check.yml`](../../../../.github/workflows/rust-check.yml)).
- **BuildKit.** Dockerfiles use `RUN --mount=type=cache` and, for the web apps, `--mount=type=secret`. Use `docker buildx build` or Docker with BuildKit enabled.
- **Third-party source archives.** Rust images include `vendor.tar.bz2` (from `cargo vendor`) as an open-source compliance artifact. Do not remove it to save space.
- **Snapshot charts carry `0.0.0`.** `Chart.yaml` `version` and `appVersion` are placeholders. Package with real versions and set `image.tag` explicitly:

  ```bash
  helm package storage/reference/storage-service/helm --version 1.2.4-custom.1 --app-version 1.2.4-custom.1 -d ./dist
  ```

  The Storage chart's image is `repository:tag` with no `appVersion` fallback; its default tag is `"0.0.0"`.
- **Pushing.** Tag for your registry (`<registry>/<org>/<image>:<tag>`), push with your registry credentials, and override `image.repository`/`image.tag` and pull secrets in values. Do not push to `nvcr.io/nvidia/omniverse`.

## Component matrix

| Component | Dockerfile | Context | Builder / toolchain | Runtime base | CI image build |
| --- | --- | --- | --- | --- | --- |
| Storage Service | [`storage/reference/storage-service/Dockerfile`](../../../../storage/reference/storage-service/Dockerfile) | repo root | `rust:1.96.1-bookworm` (toolchain 1.96.1); `python:3.13.12-slim` for attributions; protoc vendored | `nvcr.io/nvidia/distroless/cc:v4.1.5`, user `nvs` | yes ([`storage-service.yml`](../../../../.github/workflows/storage-service.yml)) |
| Auto Discovery operator | [`discovery/reference/discovery-service/operator/Dockerfile`](../../../../discovery/reference/discovery-service/operator/Dockerfile) | operator dir | `rust:1.97-bookworm` + `protobuf-compiler` (toolchain 1.97.0) | `nvcr.io/nvidia/distroless/cc:v4.1.6`, UID 1000; exposes 8080, 8081 | no workflow |
| Operator mock capabilities | [`Dockerfile.mock`](../../../../discovery/reference/discovery-service/operator/Dockerfile.mock) | operator dir | `rust:1.97-bookworm` | distroless cc v4.1.6; 50051 | no |
| Legacy Discovery | none (Helm chart over an nginx image) | — | — | snapshot default `nginx:1.31`; released profile `simple-nginx` 1.1.0 | no |
| Permission Service | [`permissions/reference/permission-service/docker/Dockerfile`](../../../../permissions/reference/permission-service/docker/Dockerfile) | repo root | `rust:1.91` (toolchain 1.91.0) | `gcr.io/distroless/cc-debian12`, UID 1000; 3000, 3010 | yes |
| Envoy auth extension | [`permissions/reference/envoy-auth-extension/docker/envoy-auth-ext.Dockerfile`](../../../../permissions/reference/envoy-auth-extension/docker/envoy-auth-ext.Dockerfile) | component dir (first-party crates vendored under `vendor/`) | `rust:1.97` + protoc (toolchain 1.97); `GENERATE_LICENSES=true` adds a 5–10 minute attribution step | `nvcr.io/nvidia/distroless/cc:v4.1.6`; 50051 | yes |
| User Info Service | [`user-identity/reference/microsoft-graph-userinfo-service/Dockerfile`](../../../../user-identity/reference/microsoft-graph-userinfo-service/Dockerfile) | component dir | `rust:1.91-bookworm` + protoc (toolchain 1.91) | distroless cc v4.1.4, UID 1000; 8080, 50051 | no (check/test only) |
| Thumbnail Service (`server`, `task-worker` targets) | [`thumbnails/reference/thumbnail-service/docker/Dockerfile`](../../../../thumbnails/reference/thumbnail-service/docker/Dockerfile) | repo root | `rust:1.94` (toolchain 1.94); generated protobuf code committed, no protoc | distroless cc v4.1.4; server 3000, 50051; worker 8080 | yes, both targets |
| Event Aggregation / Event Consumer | `notifications/reference/<service>/Dockerfile` | service dir plus named context `common` | `python:3.14-slim`, uv | `nvcr.io/nvidia/distroless/python:3.14-v4.1.4`, user `nvs` | no |
| Storage Navigator UI | [`examples/navigator-ui/Dockerfile`](../../../../examples/navigator-ui/Dockerfile) | component dir | `node:22-alpine`; `--secret id=npmrc`; `--build-arg APP_VERSION`; `ARG SERVE_IMAGE` and `ARG SERVE_USER` (default `nvs`, applied as `USER`) | `SERVE_IMAGE` default `public-nginx`, a stage built on public `nginxinc/nginx-unprivileged:1.31.7-alpine-slim` and laid out like simple-nginx (user `nvs`, config at `/usr/local/nginx/conf/nginx.conf`); pass `SERVE_IMAGE=<simple-nginx image>` to use that instead | no |
| Permission UI | [`permissions/reference/permission-ui/Dockerfile`](../../../../permissions/reference/permission-ui/Dockerfile) | component dir | `node:22-alpine`; `--secret id=npmrc`; `ARG SERVE_IMAGE` and `ARG SERVE_USER` (default `nvs`, applied as `USER`) | `nvcr.io/nvidia/omniverse/simple-nginx:1.1.0` by default (reference only; any nginx serving `/var/www/html` with the chart's config works) | no |
| Python filesystem example | [`storage/api/filesystem_example/Dockerfile`](../../../../storage/api/filesystem_example/Dockerfile) | component dir, after `poetry build` | Poetry wheel | `python:3.10-slim` | no |

Conformance and smoke-test images (`Dockerfile.conformance-test`, `Dockerfile.smoke-test`, `conformance/Dockerfile`) belong to the test references: [../test/conformance.md](../test/conformance.md).

Rust workspaces without images: Storage Core (no `rust-toolchain.toml`; `rust-version` 1.91.1; CI in [`storage-core.yml`](../../../../.github/workflows/storage-core.yml)), observability libraries (toolchain 1.91.0; tests need `cargo nextest`, see [`observability.yml`](../../../../.github/workflows/observability.yml)), URL normalizer 0.1.0, and the vendored Azure SDK fork (toolchain 1.88.0, used only as a path dependency).

## Commands

### Storage Service

```bash
# from the repository root
cargo metadata --format-version 1 --manifest-path storage/reference/storage-service/Cargo.toml > /dev/null
docker buildx build -f storage/reference/storage-service/Dockerfile \
  --build-arg GIT_REVISION="$(git rev-parse HEAD)" -t <registry>/storage-service:<tag> .
```

Run the `cargo metadata` step first. The Dockerfile bind-mounts the source **read-only** and vendors with `cargo vendor --locked`, so the image build cannot update `Cargo.lock`. The published lock does not resolve as-is in this layout: its first-party crate entries must be re-resolved by path, and in this snapshot it records a newer storage-core version than the sibling `storage-core` `Cargo.toml`. `cargo metadata` rewrites only the first-party path entries and keeps the third-party pins; CI runs the same step ([`docker-image.yml`](../../../../.github/workflows/docker-image.yml) "Reconcile the published lockfile"). Commit or discard the rewritten lock deliberately.

Path dependencies: `storage/reference/storage-core`, `storage/reference/externals/azure-sdk`, `common/url-normalizer`. A change to any of them can break the image. CI disk notes: the release build plus `cargo vendor` does not fit in the roughly 14 GB a hosted runner leaves free, so CI first frees about 25 GB ([`docker-image.yml`](../../../../.github/workflows/docker-image.yml)).

### Auto Discovery operator

```bash
cd discovery/reference/discovery-service/operator
cargo build --release --locked            # needs protoc on PATH
docker buildx build -t <registry>/auto-discovery-service:<tag> .
```

`discovery-protos/` is a plain directory in this snapshot (the README's `git submodule update --init` applies to the source project). The README's attribution step names `tools/generate_attributions.py`, which is not in this snapshot.

### Permission Service, Envoy auth extension, Thumbnail Service

```bash
docker buildx build -f permissions/reference/permission-service/docker/Dockerfile -t <registry>/permission-service:<tag> .
(cd permissions/reference/envoy-auth-extension && \
  docker buildx build -f docker/envoy-auth-ext.Dockerfile -t <registry>/envoy-auth-ext:<tag> .)
docker buildx build -f thumbnails/reference/thumbnail-service/docker/Dockerfile --target server      -t <registry>/thumbnail-service:<tag> .
docker buildx build -f thumbnails/reference/thumbnail-service/docker/Dockerfile --target task-worker -t <registry>/thumbnail-task-worker:<tag> .
```

All from the repository root. Details and configuration: [permissions-and-auth.md](permissions-and-auth.md), [thumbnails.md](thumbnails.md).

### User Info Service

```bash
cd user-identity/reference/microsoft-graph-userinfo-service
docker buildx build -t <registry>/microsoft-graph-userinfo-service:<tag> .
```

See [userinfo.md](userinfo.md).

### Event Aggregation and Event Consumer

Each service now publishes its `uv.lock`, resolved from public PyPI. Build the image from the service directory with BuildKit, passing the shared library as a named context:

```bash
cd notifications/reference/event-consumer-service   # or event-aggregation-service
docker build --build-context common=../common -t <registry>/event-consumer-service:<tag> .
```

Both locks record `notification-common` 7.0.0, the version in `../common`. The image exports the lock with `uv export --locked`, so a lock out of date with `pyproject.toml` or `../common` fails the build. For a process run, `uv sync` (or `uv sync --locked`) and `OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start` for a tokenless local run only (`BUILDING.md`). See [notifications.md](notifications.md).

### Storage Navigator UI and Permission UI

```bash
cd examples/navigator-ui
docker buildx build --secret id=npmrc,src="$HOME/.npmrc" --build-arg APP_VERSION=<version> \
  -t <registry>/storage-navigator:<tag> .
```

Notes:

- The serve stage's base image is a reference only: the Navigator UI defaults to its `public-nginx` stage, the Permission UI to `nvcr.io/nvidia/omniverse/simple-nginx:1.1.0`. Any nginx image can replace it as long as it serves the built app from `/var/www/html` with the chart's nginx config (mounted at `/usr/local/nginx/conf/nginx.conf`) and listens on port 8080.
- `@omniverse/storage-permission-ui` is an optional dependency declared as `file:../../permissions/reference/storage-permission-ui`. For a local build, build the library first (`npm ci && npm run build:lib` in that directory), then run `npm ci` in `examples/navigator-ui`; its `.npmrc` sets `install-links=true` ([README](../../../../examples/navigator-ui/README.md)). A Docker build from the component directory cannot see that path, so the image gets the build stub and no permission panel.

Never bake `.npmrc` into an image layer; pass it only as a BuildKit secret.

### Python filesystem example

```bash
cd storage/api/filesystem_example
.poetry_venv/bin/poetry build
docker buildx build -t <registry>/filesystem-storage-service:<tag> .
```

See [local-development/filesystem-example-stack.md](local-development/filesystem-example-stack.md#containerize).

## Private-registry and internal-only dependencies

From [`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md) and the files themselves:

| Dependency | Where | Effect |
| --- | --- | --- |
| `@omniverse` npm scope | Storage Permission UI publishing; Navigator installs the package from in-repo source | publishing only |
| `pb.api.nvidia.com` Buf registry | The Permission Service's `buf.yaml` and proto-crate `buf.gen.yaml` files. Discovery, notification and thumbnail protos now depend only on public `buf.build` modules, and the Thumbnail Service regenerates its bindings from the in-repo protos | `buf` lint/generate against these modules needs access; normal Rust builds use committed or vendored protos |
| Internal `[[tool.uv.index]]` entries | removed from published notification packages | `uv sync` resolves from PyPI |

Publicly pullable base images used here: `nvcr.io/nvidia/distroless/*`, `nvcr.io/nvidia/omniverse/simple-nginx`, `gcr.io/distroless/cc-debian12`, Docker Hub `rust`, `python`, `node`, `nginx`, `nginxinc/nginx-unprivileged`. CI builds without registry login, so a Dockerfile that starts using an internal registry fails CI by design ([`docker-image.yml`](../../../../.github/workflows/docker-image.yml)).

## What CI checks here

| Workflow | Checks |
| --- | --- |
| [`storage-service.yml`](../../../../.github/workflows/storage-service.yml) | `cargo check`, lib unit tests (with Postgres and Valkey service containers), image build |
| [`storage-core.yml`](../../../../.github/workflows/storage-core.yml) | check, tests excluding integration tests |
| [`permission-service.yml`](../../../../.github/workflows/permission-service.yml), [`envoy-auth-extension.yml`](../../../../.github/workflows/envoy-auth-extension.yml) | check, tests, image build |
| [`thumbnail-service.yml`](../../../../.github/workflows/thumbnail-service.yml) | check, tests (smoke/conformance crates excluded), both image targets |
| [`microsoft-graph-userinfo-service.yml`](../../../../.github/workflows/microsoft-graph-userinfo-service.yml), [`observability.yml`](../../../../.github/workflows/observability.yml) | check, tests |

No workflow builds or tests the Auto Discovery operator, legacy Discovery chart, notifications services, Navigator UI/CLI, Permission UI, or the Python components. Images are never pushed from CI. A green image build proves only that the Dockerfile and its sources produce an image.
