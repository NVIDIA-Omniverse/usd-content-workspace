# Conformance suites and smoke tests

> **Applies to:** every conformance suite, smoke test and component test harness in this repository · **Released:** suites ship inside API resources (`storage-api` 1.0.0-beta.4) or as images/charts named in each section; smoke tests are hooks in the released service charts; smoke base image `smoke-test-base` **1.0.9** in the released inventory · **This repo's snapshot:** versions per component (see [versions/source-snapshot.md](../versions/source-snapshot.md)); conformance and smoke charts here carry `0.0.0` placeholder versions · **Sources:** the `conformance_tests/`, `conformance/`, `*-conformance-test*`, `*-smoke-test*`, `helm/**/templates/tests/` and `files/tests/` paths cited below, each component README

**A conformance suite** checks that an implementation honors a contract; it is black-box and can run against any implementation. **A smoke test** checks that one deployment of one service works end to end; it is a Helm hook in that service's chart. Neither replaces the other, and neither replaces the released cross-service [integration tests](integration-tests.md).

How to read any result: record the exit code and the selected, passed, failed, skipped and deselected counts. A green run with many skips usually means a capability or endpoint was not advertised or configured, not that it passed. Run suites only against non-production namespaces and disposable data prefixes; several of them write data they do not clean up.

Running a suite in a cluster means installing a Job into it. This reference explains how; do it only when asked, in a namespace you own.

## Inventory

| Component | Conformance suite | Smoke test / harness |
|---|---|---|
| Storage API | [Storage API suite](#storage-api-conformance-suite) (pytest-bdd) | — |
| Storage Service | runs the Storage API suite | [Helm smoke test](#storage-service-smoke-test), [Rust integration tests](#storage-service-integration-tests) |
| Storage Core | — | [integration tiers](#storage-core-integration-tests) |
| Metadata API | none in this snapshot ([`metadata/conformance/README.md`](../../../../metadata/conformance/README.md)) | — |
| Auto Discovery operator | [read-only conformance runner](#discovery-operator-conformance-runner) | — |
| Legacy Discovery chart | [chart conformance tests](#legacy-discovery-chart) | [smoke hook and kind test](#legacy-discovery-chart) |
| Permission Service | [conformance test](#permission-service) | [smoke test](#permission-service) |
| Envoy auth extension | — | [docker-compose integration test](#envoy-auth-extension) |
| Permission UI | — | [`helm test` connection check](#permission-ui) |
| User Info Service | [conformance test](#user-info-service) | [smoke test](#user-info-service) |
| Thumbnail Service | [conformance test](#thumbnail-service) | [smoke test](#thumbnail-service) |
| Event Aggregation Service | [conformance tests](#event-aggregation-service) | [smoke tests](#event-aggregation-service) |
| Event Consumer Service | [conformance tests](#event-consumer-service) | [smoke tests](#event-consumer-service) |
| Cross-service | [released integration tests 1.0.23](integration-tests.md) (not in this repo) | — |

---

## Storage API conformance suite

**Location:** [`storage/api/conformance_tests/`](../../../../storage/api/conformance_tests/) (package `omniverse-storageapi-conformance-tests`, an unreleased build (see its `pyproject.toml`); released copy inside `storage-api` 1.0.0-beta.4). Runner: [`storage/api/run_tests.sh`](../../../../storage/api/run_tests.sh). Instructions: [`conformance_tests/README.md`](../../../../storage/api/conformance_tests/README.md).

**What a pass covers.** 333 Gherkin scenarios in [`features/`](../../../../storage/api/conformance_tests/src/conformance_tests/features/), most run once per protocol (GRPC, REST): capabilities (`ListTopLevelAddresses` v1alpha/v1beta), filefolder (`List`/`ListStat`, `DeleteFolder`, v1alpha `CreateFolder`), fileobject (`Stat`, `Read`, `Write` including redirect and multipart, `Delete`, `Enumerate`, `FetchWriteTypeInfo`, v1alpha `Copy`, `Move`, `IsReadOnly`, native metadata, read-only storage), metadata v1alpha, versioning (`EnumerateVersions`, v1alpha `IsVersioned`, unversioned storage), plus an optional OpenAPI comparison (`test_openapi.py`). It is a consistency contract for generic clients, not a performance or quality benchmark. It sends **no credentials** and opens a **plaintext** gRPC channel, so it does not test authentication, authorization or TLS. Groups or versions the service does not list in `ListServices` are skipped, not failed.

**Run locally against the reference service** (from `storage/api`, not `conformance_tests/`; the [`README.md`](../../../../storage/api/README.md) quick start has this wrong):

```bash
cd storage/api
export POETRY_VIRTUALENVS_IN_PROJECT=true
(cd filesystem_example && poetry install)
(cd conformance_tests && poetry install && .venv/bin/pip install -e ../filesystem_example)
source conformance_tests/.venv/bin/activate
./run_tests.sh                       # start reference on a temp dir, run all, stop
./run_tests.sh -k stat -v --no-parallel
```

Details, custom backends via `BACKEND_ARGS`, and the `-m grpc`/`-m rest` defect: [develop/custom-storage-adapter/python-backend.md](../develop/custom-storage-adapter/python-backend.md#run-the-conformance-suite).

**Run against any running service:**

```bash
source storage/api/conformance_tests/.venv/bin/activate
export TEST_STORAGE_API_REST_ENDPOINT=http://localhost:8011     # "off" or "" disables REST scenarios
export TEST_STORAGE_API_GRPC_ENDPOINT=localhost:50051
export TEST_STORAGE_API_RESOURCE_BASE=<prefix under one of the service's top-level addresses>
run-conformance-tests -n 8           # extra args go to pytest; -n needs pytest-xdist (installed)
```

| Variable | Default | Purpose |
|---|---|---|
| `TEST_STORAGE_API_REST_ENDPOINT` | `http://localhost:8011` | REST root (without version prefix) |
| `TEST_STORAGE_API_GRPC_ENDPOINT` | `localhost:50051` | gRPC `host:port` (plaintext) |
| `TEST_STORAGE_API_RESOURCE_BASE` | `file-storage://fileservice` | Where test namespaces are created |
| `STORAGEAPI_TEST_HTTP_TIMEOUT` | `60` | REST timeout, seconds |
| `TEST_INVALID_RESOURCE_ADDRESS` / `_IDENTITY` | `c:d:e:\0` | Values that must be invalid for your service |
| `OASDIFF_EXECUTABLE`, `TEST_EXACT_OPENAPI_MATCH` | unset / `false` | Enable the OpenAPI diff tests |
| `PYTEST_PLUGINS` | `conformance_tests.example_fixtures.storageapi_testdata_generator` | Test-data generator; `...boto3_testdata_generator` writes fixtures directly to S3 (`AWS_*`, `TEST_STORAGE_API_BOTO3_BUCKET_NAME` default `sapiv`, `CREATE_FOLDER`) |
| `FILESERVICE_VERSIONING_MODE=unversioned`, `FILESERVICE_READ_ONLY=true` | unset | Enable the `@unversioned` / `@read_only` scenarios; the service must run in the same mode |

Use `-m "not optional"` to skip the 15 `@optional` scenarios. Reports and logs go to the current directory.

### Run against a deployed adapter

No Helm chart ships for this suite; run it from a workstation through a port-forward. Point it at a namespace and prefix you can write to and lose.

| Target | Port-forward | Endpoints | Resource base |
|---|---|---|---|
| Filesystem reference in-cluster | `kubectl -n <ns> port-forward svc/<svc> 8011:8011 50051:50051` | REST `http://localhost:8011`, gRPC `localhost:50051` | `file-storage://fileservice/<prefix>` (or your `--base-uri`) |
| Storage Service | `kubectl -n <ns> port-forward svc/<storage-service> 8012:8012 8011:8011` | REST `http://localhost:8012`, gRPC `localhost:8011` | e.g. `https://<bucket>.s3.<region>.amazonaws.com/<prefix>` (a top-level address it reports) |

The source skill forwarded `8011:8011 50051:50051` for any Storage Service deployment; that matches the filesystem reference, not the Storage Service chart (`service.grpc.port: 8011`, `service.rest.port: 8012`). Because the suite sends no `Authorization` header, a Storage Service with `config.permissions.enabled: true` rejects every call (missing bearer credentials). Run it only against a non-production instance with permissions disabled, or rely on the [smoke test](#storage-service-smoke-test) and [integration tests](integration-tests.md) for authenticated checks.

---

## Storage Service smoke test

**Location:** [`storage/reference/storage-service/helm/files/tests/`](../../../../storage/reference/storage-service/helm/files/tests/), template [`helm/templates/tests/smoke-test.yaml`](../../../../storage/reference/storage-service/helm/templates/tests/smoke-test.yaml), docs [`SMOKE_TESTS.md`](../../../../storage/reference/storage-service/SMOKE_TESTS.md).

**Covers:** 30 tests (15 REST, 15 gRPC) over the **v1alpha** surface: `ListServices`, `ListTopLevelAddresses`, `ListRoutes`, `Write`, `Stat`, `ReadFromAddress`, `Read`, `Delete`, `Copy`, `EnumerateVersions`, `List`, `ListStat`, and the deprecated single-item `UpdateMetadata`/`GetMetadata`/`DeleteMetadata`. Markers `rest`, `grpc`. It writes under `tests.storageApiResourceBase` (comma-separated for several backends). With permissions on, grant the test identity read/write/delete on that prefix first.

**In-cluster:** set `tests.enabled: true` and `tests.storageApiResourceBase`. The Job is a hook on `test,post-install,post-upgrade`, so it also runs on every install and upgrade, not only on `helm test <release>`. Keys: `tests.grpcStorageEndpoint` / `tests.httpStorageEndpoint` (default: ingress URLs from `httpProxy.fqdn` when ingress is on, in-cluster URLs otherwise), `tests.runGrpcTests`, `tests.runRestTests`, `tests.ttlSecondsAfterFinished` (1800), `tests.auth.{microsoftUrl,credentials,scope}` (each `value` or `valueFrom.secretKeyRef`; use the secret form), `tests.serviceAccount.*`, `tests.serviceIdentity.accessTokenFile.*`, `smokeTest.image.*`.

Image tag: the snapshot chart defaults `smokeTest.image.tag` to `1.0.5`; `SMOKE_TESTS.md` shows `1.0.1`; the released inventory's `smoke-test-base` is `1.0.9`. Set the tag you are entitled to explicitly.

**Locally:** `SMOKE_TESTS.md` says `cd tests` and `PYTHONPATH="../helm/files/generated:."`; there is no `tests/` directory at the chart root. From the repo:

```bash
cd storage/reference/storage-service/helm/files/tests
python3 -m venv .venv && source .venv/bin/activate && pip install grpcio protobuf pytest requests
export SMOKE_TESTS_RESOURCE_BASE=<disposable prefix> SMOKE_TESTS_REST_ENDPOINT=<url> SMOKE_TESTS_GRPC_ENDPOINT=<host:port>
export PYTHONPATH="../generated:."
python3 -m pytest smoke-tests/ -v -m rest
```

Supply auth through `SMOKE_TESTS_AUTH_*` from a secret store or token file, not inline in shell history.

## Storage Service integration tests

[`storage-service/tests/`](../../../../storage/reference/storage-service/storage-service/tests/) holds Rust integration tests (routing, URL normalizer and remapper, permissions forwarding, metadata backends, discovery v2alpha, pub/sub). This snapshot adds router cases for v1alpha `IsVersioned`: an S3 or Azure address is forwarded to its backend, and an address no route covers fails with `INVALID_ARGUMENT` ([`tests/router_v1alpha.rs`](../../../../storage/reference/storage-service/storage-service/tests/router_v1alpha.rs)). Run from `storage/reference/storage-service`: `cargo test --features test -- --nocapture`, or one file with `--test <name>` ([`README.md`](../../../../storage/reference/storage-service/README.md)). External prerequisites per test file (databases, cloud credentials) are not documented in one place; read the test's fixtures.

## Storage Core integration tests

[`storage-core-integration-tests/`](../../../../storage/reference/storage-core/storage-core-integration-tests/README.md): cumulative nextest profiles `integration-small` (1,106 cases), `integration-medium`, `integration-large`, `integration-full` (7,964; CI, sharded) from [`.config/nextest.toml`](../../../../storage/reference/storage-core/.config/nextest.toml).

```bash
cd storage/reference/storage-core
cargo nextest run --workspace                                            # all crates + small integration subset
cargo nextest run -p storage-core-integration-tests -P integration-medium
```

Covers FileObject, FileFolder, Capabilities, Versioning, metadata and routing, manifest, observability and OpenAPI against real backends. Cloud-backed cases need `OMNI_STORAGE_SERVICE_*` credentials from the environment. The README and the fixtures' error messages say to `source tools/vault/local_env.sh`; that script is **not in this snapshot**, so export the variables from your own secret store. The README now states that the local services compose file is not published. Provide MinIO (`OMNI_STORAGE_SERVICE_MINIO_ADDRESS`; when it is absent the suite logs a `docker run` command for a pinned MinIO release), Valkey/Redis (`OMNI_STORAGE_SERVICE_REDIS_ADDRESS`) and the proxy yourself, or expect those cases to skip or fail. This snapshot adds MinIO cases `write_rejects_too_short_payloads` and `write_truncates_too_long_payloads` (gRPC and REST) for the new `data_object_size` enforcement.

---

## Discovery operator conformance runner

**Location:** [`discovery/reference/discovery-service/operator/conformance/`](../../../../discovery/reference/discovery-service/operator/conformance/README.md). Read-only; never deploys or mutates.

**Covers** (each profile independent): the catalog `GET /api/v2beta/services` (or its `/api/v2/services` alias) against a bundled draft OpenAPI (schema-version 2; `--expect-service` asserts presence), REST capabilities `v1beta` (`/v1beta/services`) on explicit backend URLs, and gRPC capabilities **v2beta** `ListServices` on explicit targets, with bounded pagination. Targets that serve only the deprecated v2alpha, which includes every service in this snapshot, fail the gRPC check with `UNIMPLEMENTED`; the operator itself still registers them through its v2alpha fallback. Not covered: auth challenges, visibility, inventory parity, freshness. Exit codes `0` pass, `1` contract violation, `2` configuration/report error, `3` prerequisites unavailable; a run with no checks cannot pass.

```bash
cd discovery/reference/discovery-service/operator     # the README's "repository root"
uv sync --project conformance --locked
uv run --project conformance --locked discovery-conformance \
  --catalog-url https://<discovery-host>/api/v2/services --expect-service fileobject \
  --json-report result.json --junit-report result.xml
```

`--token-file` authenticates the catalog, `--capabilities-token-file` the backend targets; tokens require TLS and are never accepted on the command line; `--ca-bundle` for a private CA; `--plaintext-grpc` only for a local fixture. In-cluster: build `conformance/Dockerfile` (runs as UID 10001) and run it as a Job with mounted secret files; no chart ships.

## Legacy Discovery chart

**Conformance:** [`chart/conformance_tests/`](../../../../discovery/reference/discovery-service/chart/conformance_tests/README.md) checks `GET /health`, `GET /api/v1alpha/services` and `GET /api/v1/services` (schema-version 1, required fields, at least one endpoint per service).

```bash
cd discovery/reference/discovery-service/chart/conformance_tests
uv sync
CONFORMANCE_REST_ENDPOINT=http://<discovery>:8080 uv run run-conformance-tests -v
docker compose up --abort-on-container-exit --exit-code-from conformance-tests   # against a bundled nginx fixture
```

In-cluster: chart [`helm/discovery-conformance-tests/`](../../../../discovery/reference/discovery-service/chart/helm/discovery-conformance-tests/) (Job; `service.restEndpoint`, `service.healthEndpoint`, `skipOptionalTests`; image `nvcr.io/nvidia/omniverse/discovery-conformance-tests`, tag placeholder `0.0.0`, set a real one).

**Smoke:** `smokeTest.enabled` renders a `helm test` hook running [`helm/tests/smoke_tests.py`](../../../../discovery/reference/discovery-service/chart/helm/tests/smoke_tests.py) against `/api/v1alpha/services` and `/api/v1/services`; default image `smoke-test-base:1.0.1`. **Kind lifecycle test:** `scripts/test-local-kind.sh` from the chart directory lints, installs, checks payloads, runs the smoke hook and the 14 conformance tests in an isolated kind cluster ([`tests/local/README.md`](../../../../discovery/reference/discovery-service/chart/tests/local/README.md)); it never touches your current kube context.

---

## Permission Service

**Conformance:** [`crates/permission-service-conformance-test/`](../../../../permissions/reference/permission-service/crates/permission-service-conformance-test/README.md). REST `GET /health`, `POST /v1beta/authorization/` allow and deny, `POST /v1beta/authorization/batch/` with `and`/`or`/none conditions and short-circuit `skip`; gRPC `CheckPermission`, `CheckPermissionBatch` (`CONDITION_AND`/`OR`/`UNSPECIFIED`). Preconditions: an allow rule for the given principal/action/resource; seed it with the `conformance-test-setup` binary in the `permission-service` crate (`--rest-url`, defaults principal `conformance-test-principal`, action `read`, service `conformance-test`).

```bash
cd permissions/reference/permission-service
cargo run --bin conformance-test-setup -- --rest-url http://localhost:3000
cargo run --bin permission-service-conformance-test -- \
  --rest-url http://localhost:3000 --grpc-url http://localhost:3010 \
  --principal conformance-test-principal --action-name read --action-service conformance-test \
  --resource-id conformance-test-resource --resource-type ConformanceTestResource
```

In-cluster: chart [`helm/permission-service-conformance-test/`](../../../../permissions/reference/permission-service/helm/permission-service-conformance-test/) (Job; `conformanceTest.*`, `serviceIdentity.clientCredentials` with `clientSecretRef` or `accessTokenFile`). The crate README links an `AGENTS.md` that is not in this snapshot.

**Smoke:** [`crates/permission-smoke-test/`](../../../../permissions/reference/permission-service/crates/permission-smoke-test/README.md): health, add/get/check/delete a policy, verify deny after delete. Chart hook when `smokeTest.enabled` (hooks `test,post-install,post-upgrade`), image `nvcr.io/nvidia/omniverse/permission-smoke-test`; outbound auth from the chart's `serviceIdentity`. It mutates policies; run it only where that is acceptable.

## Envoy auth extension

[`permissions/reference/envoy-auth-extension/test/run-integration-tests.sh`](../../../../permissions/reference/envoy-auth-extension/test/run-integration-tests.sh) (run from the extension's root): builds and starts `test/docker-compose-testing.yaml`, then uses a digest-pinned `grpcurl` to check gRPC health, reflection, and `envoy.service.auth.v3.Authorization/Check` allow responses and injected headers. Needs Docker; no cluster.

## Permission UI

`helm test <release>` runs a `busybox` connection check ([`templates/tests/test-connection.yaml`](../../../../permissions/reference/permission-ui/helm/permission-ui/templates/tests/test-connection.yaml)). It shows the Service answers, nothing more.

## User Info Service

**Conformance:** [`crates/userinfo-conformance-test/`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/userinfo-conformance-test/README.md). REST `/api/v1alpha` users, groups, memberships, filters, pagination, 404s; gRPC `nvidia.omniverse.userinfo.v1alpha.UserInfoService`. Samples fixtures from the live directory and fails fast if it cannot (needs a user with email, a group with a member within `--page-limit`).

```bash
cd user-identity/reference/microsoft-graph-userinfo-service
cargo run -p userinfo-conformance-test -- --rest-url http://localhost:8080 --grpc-url http://localhost:50051 \
  --access-token-file /path/to/token.json
```

In-cluster: chart [`helm/userinfo-conformance-test/`](../../../../user-identity/reference/microsoft-graph-userinfo-service/helm/userinfo-conformance-test/) (defaults `restUrl http://userinfo-service:8080`, `grpcUrl http://userinfo-service:50051`, `readyPath /readyz`). Prefer the token-file or client-credentials modes over `--bearer-token`. The README links an `AGENTS.md` not in this snapshot.

**Smoke:** [`crates/microsoft-graph-userinfo-smoke-test/`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/microsoft-graph-userinfo-smoke-test/README.md): service bearer (optional), `/healthz` and `/readyz`, optional Microsoft Graph check, REST and gRPC list/get; stops at first failure. Chart hook when `smokeTest.enabled`.

## Thumbnail Service

**Conformance:** [`crates/thumbnail-conformance-test/`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/README.md). REST `/v1alpha` associations, thumbnails, `CreateThumbnails` acceptance and rejection, health; gRPC `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService` with rejection parity; OpenAPI preflight of the 1000-specification bound. `--source-file` must already exist in storage; the run leaves generated thumbnails it cannot delete, and on a non-conformant deployment can make that source's metadata unreadable. Use a dedicated object.

```bash
cd thumbnails/reference/thumbnail-service
cargo run -p thumbnail-conformance-test -- --rest-url http://localhost:3000 --grpc-url http://localhost:50051 \
  --source-file "<existing disposable object address>" --access-token-file /path/to/token.json
```

In-cluster: chart [`helm/thumbnail-conformance-test/`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-conformance-test/), installed on demand (not a hook); rendering fails without `conformanceTest.restUrl` and `sourceFile`.

**Smoke:** [`crates/thumbnail-smoke-test/`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-smoke-test/README.md): health, associate, list, disassociate. Chart hook when `smokeTest.enabled`. Prefer `AUTH_TOKEN_FILE` over `AUTH_TOKEN`.

More: [deploy/thumbnails.md](../deploy/thumbnails.md).

---

## Event Aggregation Service

**Conformance:** [`conformance_tests/`](../../../../notifications/reference/event-aggregation-service/conformance_tests/README.md). gRPC `PublishEvent`, `BatchPublishEvents` including size and count limits; REST `GET /api/v1beta/health`, `POST /api/v1beta/events`, `/events/batch` with 422 cases. 401 cases only with `CONFORMANCE_AUTH_REQUIRED=true`. Needs a running service with RabbitMQ.

```bash
cd notifications/reference/event-aggregation-service/conformance_tests
uv sync
CONFORMANCE_GRPC_ENDPOINT=localhost:50051 CONFORMANCE_REST_ENDPOINT=http://localhost:8001 uv run run-conformance-tests
```

`uv run start` and the bundled compose run REST on `8001`; the chart's REST port is `8000`. The compose file sets `OMNI_EVENTS_PRINCIPAL_SOURCE: "disabled"`; a local `uv run start` needs the same for a tokenless run, because notification-common 7.0.0 refuses the `jwtUnverified` default without an opt-in. In-cluster: install the local chart [`helm/event-aggregation-conformance-tests/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-conformance-tests/) (defaults `event-aggregation-service:50051`, `http://event-aggregation-service:8000`). The README's `oci://nvcr.io/...` chart coordinates and versions are unverified here. **Security:** this chart takes the bearer token as a plain value (`auth.token`) rendered into the Job's environment; do not use it with a real token unless you accept that it lands in the release manifest. The consumer chart's secret-reference pattern is the safe one.

**Smoke:** [`helm/event-aggregation-service/files/tests/`](../../../../notifications/reference/event-aggregation-service/helm/event-aggregation-service/files/tests/): 4 tests (2 gRPC, 2 REST). Hook (`test,post-install,post-upgrade`) when `tests.enabled`; image `smokeTest.image` (default `smoke-test-base:1.0.1`). More: [deploy/notifications.md](../deploy/notifications.md).

## Event Consumer Service

**Conformance:** [`conformance_tests/`](../../../../notifications/reference/event-consumer-service/conformance_tests/README.md). gRPC durable-queue create/update/delete/consume (settlement protocol only), non-durable consume; REST queue CRUD and SSE streams. Out of scope: token semantics and authorization layers; ack/nack outcomes.

```bash
cd notifications/reference/event-consumer-service/conformance_tests
uv sync
CONFORMANCE_GRPC_ENDPOINT=localhost:50052 CONFORMANCE_REST_ENDPOINT=http://localhost:8000 uv run run-conformance-tests
```

In-cluster: local chart [`helm/event-consumer-conformance-tests/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-conformance-tests/) (defaults `event-consumer-service:50052`, `http://event-consumer-service:8000`); token from an existing Secret via `auth.existingSecretName` / `existingSecretKey`. The service chart's `service.grpc.port` defaults to `50051`, not `50052`; set `service.grpcEndpoint` to the port your release exposes.

**Smoke:** [`helm/event-consumer-service/files/tests/`](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/files/tests/): 6 tests (3 gRPC, 3 REST). Hook when `tests.enabled`; default image `smoke-test-base:1.0.1`.

---

## Smoke-test image versions

| Chart (snapshot) | `smokeTest.image` default |
|---|---|
| Storage Service | `smoke-test-base:1.0.5` |
| Legacy Discovery, Event Aggregation, Event Consumer | `smoke-test-base:1.0.1` |
| Permission, User Info | own image, tag defaults to chart `appVersion` |
| Thumbnail | defaults to the service image and tag |

The released inventory (Guide 1.3.0) pins `smoke-test-base` **1.0.9**; the previous `published` profile used 1.0.7 over chart defaults of 1.0.1. Override the tag to an image you can pull, and inspect rendered hook images: they are separate pull requirements from the service image.
