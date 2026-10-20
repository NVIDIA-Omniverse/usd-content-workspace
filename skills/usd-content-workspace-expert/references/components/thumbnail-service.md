# Component map: Thumbnail Service

> **Applies to:** `thumbnails/` (orchestrator, task-worker, conformance suite, smoke test, charts) · **Released:** chart and `thumbnail-service` image 0.1.0 (Developer Guide 1.3.0); worker image not listed separately · **This repo's snapshot:** Cargo `[workspace.package] version = "0.1.0"` (`SOURCE_VERSIONS.md`); Rust toolchain `1.94` (`rust-toolchain.toml`); charts at `0.0.0` placeholders · **Sources:** the files linked below; [published service docs 0.1.0](https://docs.omniverse.nvidia.com/ovstorage/thumbnail-service/0.1.0/index.html)

Apache-2.0 ([`LICENSE.txt`](../../../../thumbnails/reference/thumbnail-service/LICENSE.txt), [`ATTRIBUTIONS.md`](../../../../thumbnails/reference/thumbnail-service/ATTRIBUTIONS.md)). There is no in-repo skill and no `AGENTS.md` for this component. Task pages in this tree: [deploy](../deploy/thumbnails.md), [API](../apis/thumbnails.md), [custom implementation](../develop/custom-thumbnail-implementation.md), [monitoring](../operate/monitoring.md#thumbnail-service), [runbook](../operate/runbooks.md#thumbnails-stopped-appearing), [scaling](../operate/scalability.md#thumbnail-service), [upgrades](../operate/upgrades.md#thumbnail-service).

Root: `R = thumbnails/reference/thumbnail-service/`.

## Facts

| Item | Value | Where |
| --- | --- | --- |
| gRPC | `50051` (`service.grpcPort`, env `GRPC_PORT`) | `helm/thumbnail-service/values.yaml`, `README.md` |
| REST and health | `3000` (`service.restPort`, env `PORT`); `/health`, `/health/live`; OpenAPI `/api-docs/openapi.json`; Swagger `/swagger-ui` | same; `crates/service/src/http/mod.rs` |
| Worker probes and metrics | `8080` (`workerRuntime.healthPort`): `/healthz`, `/readyz`, `/metrics` | chart README "Task-worker" |
| Proto package | `nvidia.omniverse.thumbnail.v1alpha` (`ThumbnailService`: `CreateThumbnails`, `GetThumbnails`, `AssociateThumbnail`, `DisassociateThumbnail`) | `proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto`; Buf module `pb.api.nvidia.com/omniverse/thumbnail` |
| REST | `/v1alpha/thumbnails` (GET, POST), `/v1alpha/associations` (POST, DELETE) | `thumbnail.openapi.yaml` (`info.version: 0.1.0`) |
| Discovery | Service annotation `discovery.omniverse.nvidia.com/enabled: "true"` (chart default); ports named `rest` and `grpc`; `ListServices` type `thumbnail`, REST endpoint `thumbnail` `v1alpha` `/v1alpha/` | `templates/service.yaml`, `crates/service/src/grpc/capabilities.rs` |
| Events consumed | `omni.storage.created`, `omni.storage.deleted` (orchestrator); `omni.thumbnail.generate` (worker) | chart README |
| Events published | `omni.thumbnail.generate`, `omni.thumbnail.associated`, `omni.thumbnail.disassociated` | `docs/sdd.md` |
| Dependencies | Storage gRPC (`storage.endpoint`, default `storage-service:8011`), Event Consumer (`event-consumer-service:50051`), Event Aggregation (`event-aggregation-service:50051`); plaintext only | values, chart README "Outbound TLS posture" |
| Images | `nvcr.io/nvidia/omniverse/thumbnail-service`, `.../thumbnail-task-worker`, `.../thumbnail-conformance-test` | values files |

## Documents

| Need | File |
| --- | --- |
| Overview, env vars, path template, recursion guard | [`R/README.md`](../../../../thumbnails/reference/thumbnail-service/README.md) |
| Chart values (shared root, `thumbnailService`, `thumbnailWorker`) | [`R/helm/thumbnail-service/values.yaml`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/values.yaml) |
| Chart guide: auth modes, values migration, queues, worker failure model, metrics, sizing, tracing | [`R/helm/thumbnail-service/README.md`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md) |
| Templates (Deployments, HPAs, HTTPProxy, Certificate, smoke test) | [`R/helm/thumbnail-service/templates/`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/templates/) |
| Deployment architecture (first half current; second half describes the retired task queue) | [`R/helm/DEPLOYMENT_ARCHITECTURE.md`](../../../../thumbnails/reference/thumbnail-service/helm/DEPLOYMENT_ARCHITECTURE.md) |
| Design | [`R/docs/sdd.md`](../../../../thumbnails/reference/thumbnail-service/docs/sdd.md), [`R/docs/prd.md`](../../../../thumbnails/reference/thumbnail-service/docs/prd.md) |
| Decisions | [`R/docs/adr/0001-thumbnail-storage.md`](../../../../thumbnails/reference/thumbnail-service/docs/adr/0001-thumbnail-storage.md) (template-derived destinations), [`R/docs/adr/0002-resource-address-in-metadata.md`](../../../../thumbnails/reference/thumbnail-service/docs/adr/0002-resource-address-in-metadata.md) |
| Endpoint deep dives | [`R/docs/api/`](../../../../thumbnails/reference/thumbnail-service/docs/api/) (associate, disassociate, get) |
| Python task queue → Rust worker cutover | [`R/docs/cutover-runbook.md`](../../../../thumbnails/reference/thumbnail-service/docs/cutover-runbook.md) |
| Contracts | [`R/proto/`](../../../../thumbnails/reference/thumbnail-service/proto/), [`R/thumbnail.openapi.yaml`](../../../../thumbnails/reference/thumbnail-service/thumbnail.openapi.yaml); area index [`thumbnails/api/README.md`](../../../../thumbnails/api/README.md) |
| Conformance suite | [`R/crates/thumbnail-conformance-test/README.md`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/README.md), chart [`R/helm/thumbnail-conformance-test/`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-conformance-test/), image `R/Dockerfile.conformance-test`; area index [`thumbnails/conformance/README.md`](../../../../thumbnails/conformance/README.md) |
| Smoke test (Helm hook) | [`R/crates/thumbnail-smoke-test/README.md`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-smoke-test/README.md) |
| Local env template | [`R/.env.example`](../../../../thumbnails/reference/thumbnail-service/.env.example); `start-native-services.sh`, `start-local-dev.sh` |
| Make targets (`test`, `lint`, `helm-lint`, `openapi-update`, `build-local-image`, `worker-metrics`, `port-forward-*`) | [`R/Makefile`](../../../../thumbnails/reference/thumbnail-service/Makefile) |
| Scripts (proto regeneration, OpenAPI generation, env drift, backfill) | [`R/scripts/README.md`](../../../../thumbnails/reference/thumbnail-service/scripts/README.md) |
| Engineering, test and doc guides | `R/docs/engineering-best-practices.md`, `R/docs/test-writing-guide.md`, `R/docs/test-review-guide.md`, `R/docs/doc-writing-guide.md` |

## Crates

| Crate | Role |
| --- | --- |
| `crates/service` | `server` binary: Axum REST, Tonic gRPC, storage-event consumer, `generate_openapi` |
| `crates/task-worker` | `task-worker` binary: consume, convert, upload, associate |
| `crates/thumbnail-convert` | Bytes plus resolved spec → thumbnail bytes (`image` crate) |
| `crates/common` | Shared logic: auth, storage and association clients, work-event dispatch, notifications, provisioning |
| `crates/auth` | Interceptor wrapping `permission-auth` from `permissions/reference/permission-service/crates/permission-auth` |
| `crates/proto/{thumbnail,storage,notifications,discovery}` | Generated code (committed); `./scripts/buf-gen.sh` regenerates them from the in-repo protos (`proto/`, `storage/api/proto`, `notifications/api/protos`, `discovery/api/proto`) with locally installed plugins; no registry login |
| `crates/thumbnail-conformance-test`, `crates/thumbnail-smoke-test` | Test binaries |

## Build and test

Path dependencies reach outside the service directory (`common/observability` crates and `permissions/.../permission-auth`), so Docker builds use the repository root as context:

```bash
docker build -f thumbnails/reference/thumbnail-service/docker/Dockerfile --target server -t thumbnail-service:local .
docker build -f thumbnails/reference/thumbnail-service/docker/Dockerfile --target task-worker -t thumbnail-task-worker:local .
cd thumbnails/reference/thumbnail-service && cargo test --workspace && make helm-lint
```

CI here checks, tests and builds the images of this component ([`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md)). Details: [deploy/building-from-source.md](../deploy/building-from-source.md).

## Stale documents

Upstream inconsistencies in this snapshot; prefer the code, proto and `values.yaml`:

- `README.md` says there is no gRPC `GetThumbnails`; the proto and `crates/service/src/grpc/mod.rs` implement it.
- `helm/DEPLOYMENT_ARCHITECTURE.md` opens with "one Deployment with a single container" and documents the retired `thumbnail-task-queue` (FastAPI, Celery, PostgreSQL, RabbitMQ vhost).
- `docs/cutover-runbook.md` deploys a separate `helm/thumbnail-task-worker` chart and a chosen worker queue id; both are now in `helm/thumbnail-service` and the id is derived. It references `scripts-notifications/verify-generate-queue.sh`, not in this snapshot.
- Chart README examples nest `httpProxy` and `app.thumbnailPathTemplate` under `thumbnailService`; both are root keys now.
- Chart README "Running your own event distribution service" says there is no negative acknowledgement; the same README's redelivery section describes `nack_delivery_tags`.
- `values.yaml` comments disagree on whether the orchestrator may run more than one replica (see [scalability](../operate/scalability.md#thumbnail-service)).
- `DEPLOYMENT_ARCHITECTURE.md` says durable queue ids are namespaced per principal; Event Consumer 1.11.0 made ids deployment-wide.
- `crates/thumbnail-conformance-test/README.md` refers to an `AGENTS.md` that is not in the snapshot; `docs/sdd.md` lists old crate paths and routes (`crates/services/*`, task endpoints).
- `scripts/create-missing-thumbnails.sh` usage obtains a token from `scripts-notifications/get_jwt_token.sh`, not in this snapshot.

## Related areas

Storage ([components/storage-service.md](storage-service.md)), Notifications ([deploy/notifications.md](../deploy/notifications.md)), Permissions and identity ([deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md)), Discovery ([deploy/discovery.md](../deploy/discovery.md)), Navigator shows thumbnails from `omni.thumbnail.associated` events ([deploy/navigator.md](../deploy/navigator.md)).
