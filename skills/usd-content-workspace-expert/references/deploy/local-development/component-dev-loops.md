# Component development loops

> **Applies to:** Storage Service, Auto Discovery operator, legacy Discovery chart, Storage Navigator UI (other components linked) · **Released:** not applicable (source work) · **This repo's snapshot:** Storage Service 1.2.3 + `[Unreleased]`; operator 1.2.0; legacy chart 2.4.1 (unreleased); Navigator UI 1.0.4 · **Sources:** component `README.md`, `Makefile`, `package.json`, and [`.github/workflows/`](../../../../../.github/workflows/) at commit `367f7ed`

Run each command inside the component directory unless it says otherwise. Each component has its own toolchain; there is no repository-wide build. For container images and toolchain pins see [../building-from-source.md](../building-from-source.md).

## Storage Service

Directory: [`storage/reference/storage-service/`](../../../../../storage/reference/storage-service/README.md). Toolchain: Rust 1.96.1 (`rust-toolchain.toml`); protoc is vendored through `protoc-bin-vendored`, so no system protoc is needed for this workspace.

| Task | Command | Notes |
| --- | --- | --- |
| Build | `make build` | `cargo build --profile dev --workspace --all-targets` |
| Unit tests (what CI runs) | `cargo test --workspace --lib -- --skip pubsub::azure::identity::tests` | From [`storage-service.yml`](../../../../../.github/workflows/storage-service.yml). The L2 metadata cache suite needs a Valkey/Redis server at `REDIS_HOST`/`REDIS_PORT`; it uses `SET ... IFEQ`, which needs Valkey 8+ or Redis 8+ (CI uses `valkey/valkey:9.1.0-alpine`). |
| All tests | `make test` | `cargo test --no-fail-fast --profile dev --features test`. 11 of the 20 integration binaries under `storage-service/tests/` need live Azure or S3 configuration and panic without it. |
| Lint | `make lint` | Downloads taplo, cargo-deny, cargo-machete; also calls `scripts/ci-check-license-headers.sh` and `scripts/ci-check-versions.sh`, which are not in this snapshot, so the last steps fail here. Run `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -Dwarnings` directly. |
| Docs | `make docs` | Uses the packman/repoman tooling under `docs/`. |

The README's "Testing" section points at `scripts/env.sh`, `scripts/local-bootstrap.sh`, and `scripts/local-teardown.sh` for local Azure/Cosmos/Service Bus services. Those scripts are not in this snapshot (only `scripts/attributions.py` is). Provision the backends yourself or use CI-style unit tests.

To run a snapshot build in a cluster, build the image ([../building-from-source.md](../building-from-source.md#storage-service)), push it to a registry the cluster can pull from, and install the in-repo chart with `image.repository`/`image.tag` set (the snapshot chart defaults `image.tag` to `0.0.0`). Smoke tests: [`SMOKE_TESTS.md`](../../../../../storage/reference/storage-service/SMOKE_TESTS.md).

## Auto Discovery operator

Directory: [`discovery/reference/discovery-service/operator/`](../../../../../discovery/reference/discovery-service/operator/README.md). Toolchain: Rust 1.97.0 and a system `protoc` (`build.rs` compiles `discovery-protos/proto`). There is no GitHub workflow for this component in this snapshot; nothing in CI builds or tests it here.

```bash
cargo build --release --locked
cargo test

# Against your current kubeconfig context, one namespace, via the API server
./target/release/auto-discovery-service --kube-namespace <ns> --use-kube-portforward

# Without Kubernetes: static mode against a capabilities server
cargo run --release --bin mock-capabilities &            # mock server on 0.0.0.0:$PORT, default 50051
./target/release/auto-discovery-service --static-endpoint mock=http://localhost:50051#http://localhost:8082
curl http://localhost:8080/api/v2/services
```

`--static-endpoint` disables all Kubernetes access. Both forms are `NAME=GRPC_URL` or `NAME=GRPC_URL#REST_URL`; the gRPC URL is required because capabilities always come from gRPC `ListServices`.

`docker-compose.test.yml` starts a pinned gripmock server with the v2beta protos and `tests/stubs/` for tests that call a capabilities server.

The Contour end-to-end environment (`deploy/local/`) is described in [microk8s-and-ingress.md](microk8s-and-ingress.md#local-contour-test-environment-for-the-operator).

Conformance runner (read-only; checks an existing catalog and selected capability endpoints):

```bash
uv sync --project conformance --locked
uv run --project conformance --locked discovery-conformance \
  --catalog-url http://localhost:8080/api/v2/services --expect-service <name>
```

See [`conformance/README.md`](../../../../../discovery/reference/discovery-service/operator/conformance/README.md) and [../../test/conformance.md](../../test/conformance.md).

## Legacy Discovery chart

Directory: [`discovery/reference/discovery-service/chart/`](../../../../../discovery/reference/discovery-service/chart/README.md). It is a Helm chart wrapping nginx; there is no service code.

```bash
./scripts/test-local-kind.sh
```

This renders, installs into an isolated kind cluster, runs the Helm smoke test and conformance Job, upgrades, and uninstalls. The Discovery image must already be in the local Docker image store (default `simple-nginx-arm64:latest`; override with `DISCOVERY_IMAGE`). See [`tests/local/README.md`](../../../../../discovery/reference/discovery-service/chart/tests/local/README.md).

## Storage Navigator UI

Directory: [`examples/navigator-ui/`](../../../../../examples/navigator-ui/README.md). Node: the Dockerfile builds with `node:22-alpine` and `.nvmrc` pins `v22.23.3`, matching the README's Node 22+.

```bash
(cd ../../permissions/reference/storage-permission-ui && npm ci && npm run build:lib)   # optional permission panel, installed from in-repo source
npm ci               # without the library built, the app builds with a stub and no permission panel
npm run dev          # Vite on http://localhost:8080
npm test             # typecheck, prettier, eslint + stylelint, vitest
npm run build
```

In development the app fetches `/config.json`; a 404 yields empty defaults, so you add a connection manually by Discovery URL. To auto-connect, serve a `config.json` with `defaultDiscovery` (see [../navigator.md](../navigator.md#runtime-configuration)). Your identity provider must allow `http://localhost:8080/openid` as a redirect URI for the `navigator` (or `default`) client advertised by Discovery's `auth-config`.

## Other components

| Component | Local loop documented in | Owner reference |
| --- | --- | --- |
| Storage API, conformance suite, filesystem example | [`storage/api/README.md`](../../../../../storage/api/README.md), [`run_tests.sh`](../../../../../storage/api/run_tests.sh) | [../../test/conformance.md](../../test/conformance.md) |
| Storage Core | [`storage/reference/storage-core/README.md`](../../../../../storage/reference/storage-core/README.md) | [../../components/storage-core.md](../../components/storage-core.md) |
| Event Aggregation / Event Consumer / notification-common | each `BUILDING.md` under [`notifications/reference/`](../../../../../notifications/reference/) | [../notifications.md](../notifications.md) |
| Permission Service, Envoy auth extension, Permission UI | component `README.md` under [`permissions/reference/`](../../../../../permissions/reference/) | [../permissions-and-auth.md](../permissions-and-auth.md) |
| User Info Service | [`user-identity/reference/microsoft-graph-userinfo-service/`](../../../../../user-identity/reference/microsoft-graph-userinfo-service/README.md) | [../userinfo.md](../userinfo.md) |
| Thumbnail Service | [`thumbnails/reference/thumbnail-service/README.md`](../../../../../thumbnails/reference/thumbnail-service/README.md) | [../thumbnails.md](../thumbnails.md) |
