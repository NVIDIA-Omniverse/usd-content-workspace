# Microsoft Graph User Info Service — component map

> **Applies to:** Microsoft Graph User Info Service (reference implementation of the User Info API) · **Released:** chart and image `microsoft-graph-userinfo-service` 0.0.8; API resource `userinfo-api` 1.0.0-alpha.1 (usd-content-workspace Developer Guide 1.3.0 / NGC) · **This repo's snapshot:** 0.0.9 (`Cargo.toml` `[workspace.package] version`, top `CHANGELOG.md` entry `[0.0.9]`); `Chart.yaml` is the `0.0.0` placeholder · **Sources:** `user-identity/reference/microsoft-graph-userinfo-service/` (skill, README, SECURITY, CHANGELOG, docs, helm, protos, openapi.yaml, crates), upstream userinfo-service skill pinned by the [source skill package](../../NOTICE.md)

## Start with the in-repo skill

The component ships its own documentation map: [`skills/userinfo-service-docs-map/SKILL.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/skills/userinfo-service-docs-map/SKILL.md). It routes every service and API doc page by task. It is the same map as the upstream `userinfo-service` skill, with links relative to the component directory. This page adds what that map lacks.

Licence: Apache-2.0 ([`LICENSE.txt`](../../../../user-identity/reference/microsoft-graph-userinfo-service/LICENSE.txt)).

## What the in-repo skill does not list

| Thing | Path in this repo |
|---|---|
| API access logs (new in 0.0.9) | [`docs/service/docs/access-logs.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/access-logs.md) |
| API-level freshness contract | [`docs/api/docs/directory-synchronization.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/directory-synchronization.md) |
| gRPC contract (`nvidia.omniverse.userinfo.v1alpha`) | [`protos/nvidia/omniverse/userinfo/v1alpha/user_info_service.proto`](../../../../user-identity/reference/microsoft-graph-userinfo-service/protos/nvidia/omniverse/userinfo/v1alpha/user_info_service.proto) |
| OpenAPI 3.0.3, `info.version: v1alpha` | [`openapi.yaml`](../../../../user-identity/reference/microsoft-graph-userinfo-service/openapi.yaml) |
| Postman collection | [`postman_collection.json`](../../../../user-identity/reference/microsoft-graph-userinfo-service/postman_collection.json) |
| Helm chart | [`helm/microsoft-graph-userinfo-service/`](../../../../user-identity/reference/microsoft-graph-userinfo-service/helm/microsoft-graph-userinfo-service/) ([`values.yaml`](../../../../user-identity/reference/microsoft-graph-userinfo-service/helm/microsoft-graph-userinfo-service/values.yaml)) |
| Conformance test crate and chart | [`crates/userinfo-conformance-test/README.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/userinfo-conformance-test/README.md), [`helm/userinfo-conformance-test/`](../../../../user-identity/reference/microsoft-graph-userinfo-service/helm/userinfo-conformance-test/) |
| Smoke test | [`crates/microsoft-graph-userinfo-smoke-test/README.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/microsoft-graph-userinfo-smoke-test/README.md) |
| Security model | [`SECURITY.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/SECURITY.md) |
| Changelog | [`CHANGELOG.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/CHANGELOG.md) |
| Generated protobuf code (committed) | `crates/microsoft-graph-userinfo-proto/` and `crates/microsoft-graph-userinfo-capabilities-proto/`; their `buf.gen.yaml` regenerate from this component's `protos/` and from [`discovery/api/proto`](../../../../discovery/api/proto) in this repo (`buf generate` in the crate directory), with well-known types from `buf.build/protocolbuffers/wellknowntypes` |
| Dockerfiles (service, smoke, conformance) | [`Dockerfile`](../../../../user-identity/reference/microsoft-graph-userinfo-service/Dockerfile), `Dockerfile.smoke-test`, `Dockerfile.conformance-test` |
| Area index | [`user-identity/README.md`](../../../../user-identity/README.md) |

Skill-tree pages: deployment [../deploy/userinfo.md](../deploy/userinfo.md); API [../apis/userinfo.md](../apis/userinfo.md); Permission Service actions it uses [../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md).

## Facts verified in the snapshot

| Fact | Value | Where verified |
|---|---|---|
| Ports | REST `service.ports.rest` 8080 (port name `http`); gRPC `service.ports.grpc` 50051 (port name `grpc`) | `values.yaml`, `templates/service.yaml` |
| Service name | `userinfo-service` (`service.nameOverride`); the Service selects reader pods only and is rendered only when `reader.enabled` | `templates/service.yaml` |
| Health | REST `/healthz` (liveness), `/readyz` (readiness, `failureThreshold: 180`); gRPC `grpc.health.v1.Health`; gRPC reflection v1 and v1alpha | `values.yaml`, `crates/microsoft-graph-userinfo-service/src/main.rs` |
| Discovery | Serves `CapabilitiesService` (`v2alpha`) with service type `userinfo`, REST `userinfo` `v1alpha` `/api/v1alpha/`. Chart adds no Discovery annotations; set `service.annotations`. The `grpc` port is selected automatically. | `src/grpc/capabilities.rs`, Discovery operator docs |
| Inbound token validation | **None in the service.** `/users/me` reads `oid` with an unverified JWT decode (`jsonwebtoken::dangerous::insecure_decode`). An authenticating gateway in front of every path is required. | `src/http.rs`, `docs/service/docs/identity-provider.md` |
| Ingress auth default | `httpProxy.authExtension.enabled: false` | `values.yaml` |
| Writer | Always one replica; advisory-lock serialized; not behind the Service | `values.yaml` |
| Postgres TLS default | `providers.cache.postgres.sslMode: "disable"` (`prefer`, `require` available) | `values.yaml` |
| Permission integration default | `permission.enabled: false`; `url: http://permission-service:3000/` (REST) | `values.yaml` |
| Toolchain | Rust `rust:1.91-bookworm` builder; runtime `nvcr.io/nvidia/distroless/cc:v4.1.4` | `Dockerfile` |

## Snapshot 0.0.9 versus released 0.0.8

From `CHANGELOG.md` `[0.0.9]`; none of this is in the released 0.0.8 chart:

- `observability.logging.access.enabled` / `.headers` (env `ACCESS_LOG_ENABLED`, `ACCESS_LOG_HEADERS`) — API access logs.
- `observability.headers.forward` — forward chosen request headers to the Permission Service.
- Environment variable `LOG_STRUCTURED` renamed to `STRUCTURED_LOGGING` (affects anyone setting it through `extraVars`).
- Distroless base `nvcr.io/nvidia/distroless/cc:v4.1.4` for service, smoke, and conformance images; hardened conformance Job; conformance chart defaults `restUrl`/`grpcUrl` to the in-cluster Service.
- API contract docs: `username` is required on `User` examples; implementation-specific requirements removed.

The in-repo skill's front matter still says `version: 0.0.8`. Confirm what you run with [../versions/capture-a-deployment.md](../versions/capture-a-deployment.md).

## Known defects in this snapshot

Report only.

- `SECURITY.md` links `security/docker.md` and `security/entra.md`; neither is in the snapshot.
- The conformance-test README links `AGENTS.md`, which is not in the crate directory.
- `openapi.yaml` lists only `200`, `401`, and `404` responses; [`rest-api.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/rest-api.md) also specifies `400`, `403`, `429`, `502`, and `500`. Generate clients from the proto or treat undeclared statuses as possible.
- `values.yaml` describes `providers.cache.syncJitterMax` for "multi-replica writer deployments", but the same file states the writer is always one replica.
- The `values.yaml` comment on `providers.cache.memberships.enabled` recommends raising the readiness probe `failureThreshold` (to about 5 hours) for large directories; [`scaling.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/scaling.md) says a long first sync needs no probe change because readiness failures never restart the container.
- `values.yaml` comment for `permission.url` gives the example `http://permission-service:8080`; the Permission Service REST port is 3000 (the default value is correct).
