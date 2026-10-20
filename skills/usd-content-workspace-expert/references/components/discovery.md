# Discovery — component map

> **Applies to:** Auto Discovery operator, legacy static Discovery chart, discovery protos · **Released:** `auto-discovery-service` 1.1.1 (chart and image); legacy `discovery-service` chart 2.3.11 with `simple-nginx` 1.1.0 (previous profile) · **This repo's snapshot:** operator 1.2.0 ([`Cargo.toml`](../../../../discovery/reference/discovery-service/operator/Cargo.toml); unreleased; later publishes first added only license headers and docs tooling, then the most recent release commit changed the operator itself: capabilities v2beta with v2alpha fallback and pagination, the `/api/v2beta/services` catalog route, v2beta conformance profiles, and chart probes); legacy chart 2.4.1, unreleased ([`VERSION.toml`](../../../../discovery/reference/discovery-service/chart/VERSION.toml) carries a pre-release suffix); protos untagged `main` `a5f1df2`, latest release commit `Release discovery-protos 0.0.0-manual.a5f1df25` (Buf dependency only) · **Sources:** [`discovery/README.md`](../../../../discovery/README.md) and the files below; no upstream component skill exists for Discovery

Area index: [`discovery/README.md`](../../../../discovery/README.md). Deployment: [../deploy/discovery.md](../deploy/discovery.md). Contracts: [../apis/discovery.md](../apis/discovery.md).

## Auto Discovery operator — `discovery/reference/discovery-service/operator/`

| Need | Go to |
| --- | --- |
| Architecture, configuration flags/env vars, annotations, port resolution, build, run, Docker, Helm | [`README.md`](../../../../discovery/reference/discovery-service/operator/README.md) |
| Service documentation set (Sphinx) | [`docs/service/docs/index.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/index.md) |
| Helm install and core values | [`deployment.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/deployment.md) |
| Reconciliation, failure behavior, static mode | [`discovery.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/discovery.md) |
| Visibility, internal/external URL resolution | [`endpoint-resolution.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/endpoint-resolution.md) |
| v1 compatibility, auth bootstrap, Contour ingress | [`compatibility-and-access.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/compatibility-and-access.md) |
| Probes, logging, resources, troubleshooting, local validation | [`operations.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/operations.md) |
| Service HTTP API | [`service-api.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/service-api.md) |
| API documentation set (Catalog v2beta, Capabilities REST v1beta and gRPC v2beta) | [`docs/api/docs/index.md`](../../../../discovery/reference/discovery-service/operator/docs/api/docs/index.md) |
| v1 schema and v1→v2 client migration | [`docs/v1-response-schema.md`](../../../../discovery/reference/discovery-service/operator/docs/v1-response-schema.md), [`docs/v1-to-v2-migration.md`](../../../../discovery/reference/discovery-service/operator/docs/v1-to-v2-migration.md) |
| OpenAPI | [`openapi/discovery.openapi.yaml`](../../../../discovery/reference/discovery-service/operator/openapi/discovery.openapi.yaml) (catalog v2), [`openapi/capabilities_clean.openapi.yaml`](../../../../discovery/reference/discovery-service/operator/openapi/capabilities_clean.openapi.yaml) (capabilities v1beta) |
| Protos used by the build | [`discovery-protos/`](../../../../discovery/reference/discovery-service/operator/discovery-protos/README.md): v2beta (compiled for the client) and v2alpha (deprecation markers added; compiled only for fallback tests). `discovery/api/proto` has only v2alpha |
| Helm chart | [`helm/auto-discovery-service/`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml) (`Chart.yaml` `0.0.0`) |
| Source entry points | `src/main.rs`, `src/config.rs` (CLI/env), `src/api/rest.rs` (routes), `src/k8s/discovery.rs` (reconcile, URL resolution), `src/v1.rs` (compatibility mapping), `src/static_discovery.rs` |
| Tests | `tests/rest_api.rs`, `tests/capabilities_client.rs`, `tests/capabilities_fallback.rs`, `tests/capabilities_pagination.rs`, `docker-compose.test.yml` (gripmock, v2beta protos), `scripts/test-discovery-endpoints.sh` |
| Local Contour end-to-end | [`deploy/local/README.md`](../../../../discovery/reference/discovery-service/operator/deploy/local/README.md) |
| Conformance runner (read-only, customer-facing) | [`conformance/README.md`](../../../../discovery/reference/discovery-service/operator/conformance/README.md) |
| Container | [`Dockerfile`](../../../../discovery/reference/discovery-service/operator/Dockerfile), [`Dockerfile.mock`](../../../../discovery/reference/discovery-service/operator/Dockerfile.mock) |

Facts: internal catalog 8080, external 8081; routes `/api/v2beta/services` (primary) and `/api/v2/services` (alias), `/api/v1/services`, `/api/v1alpha/services`, `/api/{v1,v1alpha,v2,v2beta}/auth-config`, `/` (root redirect) ([`src/api/rest.rs`](../../../../discovery/reference/discovery-service/operator/src/api/rest.rs)); chart liveness and readiness probe `/api/v2beta/services`; the client calls capabilities v2beta and falls back to v2alpha on `UNIMPLEMENTED` ([`src/client/capabilities.rs`](../../../../discovery/reference/discovery-service/operator/src/client/capabilities.rs)); Rust 1.97.0 plus `protoc`; image runs as UID 1000 on distroless cc v4.1.6; no CHANGELOG and no CI workflow in this snapshot. The conformance runner's hand-written sources, tests, packaging files, `Dockerfile`, and bundled `specs/discovery_catalog.openapi.yaml` carry SPDX `Apache-2.0` headers; its generated gRPC stubs, `README.md`, `uv.lock`, and `specs/provenance.json` do not. Its gRPC profile and generated stubs are now v2beta, so v2alpha-only targets fail that check, and its catalog profile names `/api/v2beta/services`.

## Legacy Discovery chart — `discovery/reference/discovery-service/chart/`

| Need | Go to |
| --- | --- |
| Usage | [`README.md`](../../../../discovery/reference/discovery-service/chart/README.md) (its values example does not match the template; see [../deploy/discovery.md](../deploy/discovery.md#legacy-static-chart)) |
| Values | [`helm/values.yaml`](../../../../discovery/reference/discovery-service/chart/helm/values.yaml) |
| URL defaults per service type | `discovery-service.mkurl` in [`helm/templates/_helpers.tpl`](../../../../discovery/reference/discovery-service/chart/helm/templates/_helpers.tpl) |
| nginx routes | [`helm/templates/configmap.yaml`](../../../../discovery/reference/discovery-service/chart/helm/templates/configmap.yaml) |
| Conformance tests and their chart | [`conformance_tests/README.md`](../../../../discovery/reference/discovery-service/chart/conformance_tests/README.md), [`helm/discovery-conformance-tests/`](../../../../discovery/reference/discovery-service/chart/helm/discovery-conformance-tests/values.yaml) |
| Helm smoke test | `helm/templates/tests/smoke-test.yaml`, `helm/tests/smoke_tests.py` |
| kind end-to-end | [`scripts/test-local-kind.sh`](../../../../discovery/reference/discovery-service/chart/scripts/test-local-kind.sh), [`tests/local/README.md`](../../../../discovery/reference/discovery-service/chart/tests/local/README.md) |
| Changelog | [`CHANGELOG.md`](../../../../discovery/reference/discovery-service/chart/CHANGELOG.md) |

## Discovery protos — `discovery/api/`

| Need | Go to |
| --- | --- |
| Contract source | [`proto/nvidia/omniverse/discovery/capabilities/v2alpha/`](../../../../discovery/api/proto/nvidia/omniverse/discovery/capabilities/v2alpha/capabilities.proto) |
| Buf module and lint/breaking rules | [`proto/buf.yaml`](../../../../discovery/api/proto/buf.yaml) (depends on the public `buf.build/protocolbuffers/wellknowntypes`; no `buf.lock`) |
| Conventions | [`README.md`](../../../../discovery/api/README.md) (names the package `v1beta`; the files are `v2alpha`) |

## Known documentation defects in this snapshot

- Operator README "License generation" step 3 runs `tools/generate_attributions.py`, which is not present.
- Operator README configuration table gives `--kube-namespace` default "all namespaces"; the Helm chart always sets `KUBE_NAMESPACE` (the release namespace when `config.kubeNamespace` is empty).
- `discovery/api/README.md` documents `v1beta/capabilities.proto` and lists consumers `discovery-service` and `discovery-service-operator`; the snapshot has `v2alpha` and the consumers listed in [../apis/discovery.md](../apis/discovery.md#contract-files).
- `discovery/api` has no v2beta package, while the operator's `discovery-protos/README.md` calls v2beta current and v2alpha deprecated. Until `discovery/api` publishes v2beta, the operator copy is the only v2beta source here.
- Legacy chart README: the values example uses flat `grpc:`/`rest:` URL strings that the template does not read (it reads `endpoints.grpc` / `endpoints.rest` objects), and "deplying" is misspelled.
