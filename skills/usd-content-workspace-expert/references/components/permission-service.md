# Permission Service — component map

> **Applies to:** Permission Service (reference implementation of the Permission API) · **Released:** chart and image `permission-service` 0.0.53; API resource `permission-api` 1.0.0-beta.1 (usd-content-workspace Developer Guide 1.3.0 / NGC) · **This repo's snapshot:** 0.0.54 (`Cargo.toml` `[workspace.package] version`, top `CHANGELOG.md` entry `[0.0.54]`; `openapi.json` `info.version` still reads `0.0.53`; `Chart.yaml` is the `0.0.0` placeholder) · **Sources:** `permissions/reference/permission-service/` (README, docs/service/docs, docs/api/docs, helm, protos, openapi.json, crates), upstream permission-service skill pinned by the [source skill package](../../NOTICE.md)

A table of contents into the Permission Service source and docs in this repository. Read the linked file before answering a configuration or policy question; this page only routes and records what the upstream docs leave out.

## What it is

A centralized authorization service. Given a **principal** (user or service identity), an **action** (`<service>:<name>`), an optional **resource** (`type` + `id`) and optional **context**, it returns `allow`, `deny`, or (in batches) `skip`. Rules are [Cedar](https://www.cedarpolicy.com/) policies stored with service metadata (services, actions, resource types, evaluation priority, parent type). Storage, notifications, User Info, and your own services call it; it does not manage users or the identity provider.

Licence: Apache-2.0 ([`LICENSE.txt`](../../../../permissions/reference/permission-service/LICENSE.txt)).

## Where things are

| Thing | Path in this repo |
|---|---|
| Area index | [`permissions/README.md`](../../../../permissions/README.md) |
| Developer README (proto generation, Docker run, meta-permissions, conformance setup, release steps) | [`README.md`](../../../../permissions/reference/permission-service/README.md) |
| Security model | [`SECURITY.md`](../../../../permissions/reference/permission-service/SECURITY.md) |
| Changelog | [`CHANGELOG.md`](../../../../permissions/reference/permission-service/CHANGELOG.md) |
| gRPC contract (`nvidia.omniverse.permission.v1beta`) | [`protos/nvidia/omniverse/permission/v1beta/permission_service.proto`](../../../../permissions/reference/permission-service/protos/nvidia/omniverse/permission/v1beta/permission_service.proto) |
| OpenAPI (contract endpoints and `/health` only) | [`openapi.json`](../../../../permissions/reference/permission-service/openapi.json) |
| Postman collection | [`docs/service/postman-collection.json`](../../../../permissions/reference/permission-service/docs/service/postman-collection.json) |
| Helm chart | [`helm/permission-service/`](../../../../permissions/reference/permission-service/helm/permission-service/) ([`values.yaml`](../../../../permissions/reference/permission-service/helm/permission-service/values.yaml)) |
| Conformance-test chart | [`helm/permission-service-conformance-test/`](../../../../permissions/reference/permission-service/helm/permission-service-conformance-test/) |
| Conformance-test crate | [`crates/permission-service-conformance-test/`](../../../../permissions/reference/permission-service/crates/permission-service-conformance-test/README.md) |
| Smoke test (Helm test hook) | [`crates/permission-smoke-test/README.md`](../../../../permissions/reference/permission-service/crates/permission-smoke-test/README.md) |
| Dockerfiles (service, conformance, smoke) | [`docker/`](../../../../permissions/reference/permission-service/docker/) — build context is the repository root |
| Shared auth crate (the Envoy auth extension builds from a vendored copy) | [`crates/permission-auth/`](../../../../permissions/reference/permission-service/crates/permission-auth/) (token validation in `src/lib.rs`, settings in `src/config.rs`, `requiredJwtClaims` rules in `src/claim_rules.rs`) |
| URL remapper | [`crates/permission-service/src/url_remapper.rs`](../../../../permissions/reference/permission-service/crates/permission-service/src/url_remapper.rs) |
| API access logs (implementation) | [`crates/permission-service/src/access_log.rs`](../../../../permissions/reference/permission-service/crates/permission-service/src/access_log.rs) |

## Route by task

| If the question is about… | Read |
|---|---|
| Doc index, architecture, request flow | [`docs/service/docs/index.md`](../../../../permissions/reference/permission-service/docs/service/docs/index.md) |
| Chart values, config-file vs Postgres, CloudNativePG, URL remapper, ingress | [`deployment.md`](../../../../permissions/reference/permission-service/docs/service/docs/deployment.md) |
| `DATABASE_TYPE`, init files, scope inference | [`database-configuration.md`](../../../../permissions/reference/permission-service/docs/service/docs/database-configuration.md) |
| OIDC token validation (`openId`), outbound identity (`serviceIdentity`), Microsoft Entra ID | [`authentication.md`](../../../../permissions/reference/permission-service/docs/service/docs/authentication.md) |
| Writing Cedar policies, groups, resource hierarchies, meta-permissions | [`policy-language.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-language.md) |
| Policy CRUD API (`/v1beta/policies/`) | [`policy-management.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-management.md) |
| Service metadata API (`/v1beta/services/`) | [`metadata.md`](../../../../permissions/reference/permission-service/docs/service/docs/metadata.md) |
| How allow/deny/skip is reached, `order`, evaluation priority, batch short-circuit | [`policy-evaluation.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-evaluation.md) |
| Diagnostics API (which policies would be evaluated) | [`diagnostics.md`](../../../../permissions/reference/permission-service/docs/service/docs/diagnostics.md) |
| Change events and cross-replica cache invalidation | [`notifications.md`](../../../../permissions/reference/permission-service/docs/service/docs/notifications.md) |
| Metrics, traces, logs | [`observability.md`](../../../../permissions/reference/permission-service/docs/service/docs/observability.md) |
| API access logs (record contract, what is never logged, per-endpoint fields) | [`access-logs.md`](../../../../permissions/reference/permission-service/docs/service/docs/access-logs.md) |
| Permission Web UI | [`web-ui.md`](../../../../permissions/reference/permission-service/docs/service/docs/web-ui.md) (see the value-name caveat in [permission-ui.md](permission-ui.md)) |
| Published contract | [`docs/api/docs/index.md`](../../../../permissions/reference/permission-service/docs/api/docs/index.md), [`rest-api.md`](../../../../permissions/reference/permission-service/docs/api/docs/rest-api.md), [`grpc.md`](../../../../permissions/reference/permission-service/docs/api/docs/grpc.md) |
| User and service principal flows | [`authorization-for-a-user-principal.md`](../../../../permissions/reference/permission-service/docs/api/docs/authorization-for-a-user-principal.md), [`authorization-for-a-service-principal.md`](../../../../permissions/reference/permission-service/docs/api/docs/authorization-for-a-service-principal.md) |

Skill-tree pages built on these: [../deploy/permissions-and-auth/permission-service.md](../deploy/permissions-and-auth/permission-service.md), [../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md), [../apis/permissions.md](../apis/permissions.md), [../develop/custom-permissions-adapter.md](../develop/custom-permissions-adapter.md).

## Facts the docs scatter or omit

Verified against the snapshot chart and code; confirm against the chart you deploy.

| Fact | Value | Where verified |
|---|---|---|
| Ports | REST `service.ports.http` 3000; gRPC `service.ports.grpc` 3010 | `values.yaml`, `templates/service.yaml` |
| Services rendered | `permission-service` (ClusterIP) and `permission-service-headless` (`service.headless.enabled: true` by default) for client-side gRPC balancing | `templates/service-headless.yaml` |
| Health | REST `GET /health` (liveness and readiness probe); gRPC `grpc.health.v1.Health` | `values.yaml`, `crates/permission-service/src/grpc/mod.rs` |
| Unauthenticated routes | `GET /health`, `GET /v1beta/idp/keys/` (JWKS passthrough, not in the published docs), Swagger UI at `/` and `/openapi.json`; on gRPC, health, reflection (v1 and v1alpha) and `CapabilitiesService` are not behind the auth interceptor | `src/http/middlewares/auth.rs`, `src/http/mod.rs`, `src/grpc/mod.rs` |
| Discovery | Serves `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService` with service type `permission`, REST endpoint `permission` version `v1beta` path `/v1beta/`. The chart sets no Discovery annotations; add `discovery.omniverse.nvidia.com/enabled: "true"` through `service.annotations`. The port named `grpc` is selected automatically. `service.annotations` is applied to **both** Services, so the headless Service also opts in; check the catalog for a duplicate entry. | `src/grpc/capabilities.rs`, `templates/service*.yaml`, Discovery operator [`discovery.md`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/discovery.md) |
| Auth default | `openId.enabled: true`; startup fails (CrashLoopBackOff) if the discovery document cannot be fetched | `values.yaml`, `authentication.md` |
| Audience | `openId.allowedJwtAudience` (default `[]`) → `JWT_AUDIENCE`; the only audience source. Empty with `jwt`/`jwtuserinfo` ⇒ `JWT_AUDIENCE` omitted ⇒ startup fails. Not used with `opaque`. A JWT without `aud`, or with a non-string/non-list `aud`, is rejected. `openId.clientRegistrations` / `additionalJwtAudience` are no longer read. | `values.yaml`, `templates/_helpers.tpl` `permissions.jwtAudience`, `templates/deployment.yaml`, `crates/permission-auth/src/config.rs` `load`, `src/lib.rs` `decode_principal_from_key` |
| Issuer check | `openId.allowedJwtIssuers` empty ⇒ `iss` is **not** validated. When set, a JWT without `iss` is rejected. | `values.yaml`, `crates/permission-auth/src/lib.rs` |
| `nbf` and leeway | `nbf` enforced; `config.jwtLeeway` (60 s, max 300) applies to `exp` and `nbf` | `crates/permission-auth/src/lib.rs` |
| Required claims | `openId.requiredJwtClaims` (default `[]`) → `JWT_REQUIRED_CLAIMS` (JSON). Failing tokens get `401` and enter the negative cache. | `values.yaml`, `templates/deployment.yaml`, `crates/permission-auth/src/claim_rules.rs` |
| `tokenVerificationType` values | `jwt`, `opaque`, `jwtuserinfo` (`jwt-with-user-info` kept as alias) | `crates/permission-auth/src/config.rs` |
| Database default | `database.type: postgres`; `sslMode` accepts `""`/`disable`, `prefer`, `require` only | `values.yaml`, `crates/permission-db/src/postgres.rs` |
| Database Secret | Never created by the chart. Pre-create the Secret named by `database.postgres.secretRef.name` (default `permission-service-secret`) with `POSTGRES_PASSWORD` (unless `tokenFile.enabled`) and `POSTGRES_USER` (if `database.postgres.user` is empty); mounted at `/etc/secrets/`. Changes to it do not roll the pods. | `values.yaml`, `templates/deployment.yaml`, `database-configuration.md` |
| URL remapper | `.` / `..` segments (including `%2e%2e`) are resolved before rewrite rules match; a rule value may be a string or `{target: …}` | `src/url_remapper.rs`, `deployment.md` |
| YAML parsing | Policy, metadata, and URL remapper files are parsed with `serde_norway` (replaces `serde_yml`) | `Cargo.toml`, crate `Cargo.toml` files |
| Access logs | `observability.logging.access.enabled` (default `false`) → `ACCESS_LOG_ENABLED`; `observability.logging.access.headers` (`{}`) → `ACCESS_LOG_HEADERS`. Records go to tracing target `access`; `Authorization` and `Cookie` are never copied; records include `principal.sub` for authorization calls. | `values.yaml`, `templates/deployment.yaml`, `access-logs.md` |
| Autoscaling | Only `autoscaling.enabled` remains (`false`); the chart has no HPA template, so `true` just drops `replicas` from the Deployment for an external autoscaler | `values.yaml`, `templates/deployment.yaml` |
| Metadata seed | Chart ships `database.init.services` for `storage-service` (`read`, `write`; `object` with `parentType: folder`, `folder`, both `evaluationPriority: permit`), `event-consumer-service` (six actions), `event-aggregation-service` (`publish-event`, `EventType`) | `values.yaml` |
| Policy seed | `database.init.policies: []` — no policies, so with auth enabled nobody can administer until you seed meta-permissions | `values.yaml` |
| Metrics | OTLP push only (`observability.metrics.endpoint.grpc`); no Prometheus scrape port | `values.yaml`, `observability.md` |
| Toolchain | Rust `1.91.0` (`rust-toolchain.toml`); MSRV `rust-version = "1.88.0"` for the workspace crates, including the published `permission-core` and `permission-auth`; image `rust:1.91` builder, `gcr.io/distroless/cc-debian12` runtime, `USER 1000:1000` | `Cargo.toml`, `docker/Dockerfile` |

The component also ships an in-repo skill, [`skills/permission-service/SKILL.md`](../../../../permissions/reference/permission-service/skills/permission-service/SKILL.md) (`version: 0.0.54`), that routes to the docs above. Its "Gaps and caveats" section describes the development repository's build environment, not this snapshot; see the defects below.

## Version notes

- **Released 0.0.53 (NGC) versus snapshot 0.0.54.** NGC publishes 0.0.53; 0.0.54 is source only in this repo. Do not apply 0.0.54 values to a 0.0.53 chart or the reverse. What changes, per `CHANGELOG.md` `[0.0.54]` and the chart:
  - Audience: 0.0.53 accepts `openId.clientRegistrations[*].clientId` + `openId.additionalJwtAudience` (placeholder `some_client_id`); 0.0.54 reads only `openId.allowedJwtAudience` and refuses to start without it for `jwt`/`jwtuserinfo`. The old keys are silently ignored by 0.0.54, so an unmigrated values file produces a CrashLoopBackOff, not a render error.
  - Token checks (Security): 0.0.53 accepts JWTs that omit `aud`, or omit `iss` when `allowedJwtIssuers` is set, and does not check `nbf`; 0.0.54 rejects them. IdPs must put `aud` in every token.
  - New `openId.requiredJwtClaims`.
  - Database Secret: 0.0.53 has `secret.create` (default `true`), `secret.name`, and `database.postgres.password`, and its chart-created Secret is misnamed (fixed in the 0.0.54 changelog, then removed); 0.0.54 never creates the Secret and reads `database.postgres.secretRef.name`.
  - Removed: `autoscaling.minReplicas`, `maxReplicas`, `targetCPUUtilizationPercentage` (never used).
  - Startup fixes: `opaque` no longer needs `JWT_AUDIENCE`; `jwtuserinfo` is accepted (0.0.53 accepted only `jwt-with-user-info`).
  - URL remapper resolves dot segments before matching (0.0.53 could rewrite `…/NVIDIA/../Projects/…` with the `…/NVIDIA/` rule and evaluate it against the wrong location's policies); `serde_yml` replaced by `serde_norway`; dependency security updates; MSRV 1.84 → 1.88.
  - `serviceIdentity.accessTokenFile` volume mounted at the directory of `serviceIdentity.accessTokenFile.path` instead of the fixed `/etc/secrets/access-token`.
- **API access logs.** At this snapshot `CHANGELOG.md` lists access logs under `[0.0.53]`, but the pre-release 0.0.53 source previously imported here had no `access_log.rs` and no `observability.logging.access` values. Whether the released 0.0.53 image and chart include them is not verified; check the pulled chart's `values.yaml` for `observability.logging.access`.
- `[0.0.53]` also reduces metric cardinality (`authorization.request.allowed`/`denied` keyed by `service` only) and removes the custom millisecond `http.server.request.duration` histogram; dashboards built on 0.0.52 metrics need updating.
- `[0.0.52]` added the headless Service, `omni.permissions.meta.changed`, resource hierarchy scopes (`resource in <Type>::"<url>/"`), Postgres token-file auth, cross-replica invalidation, `config.parallelEvaluationThreshold`, and batch diagnostics.
- Confirm the version you run with [../versions/capture-a-deployment.md](../versions/capture-a-deployment.md); read the matching chart `values.yaml` and `CHANGELOG.md` before upgrading.

## Known defects in this snapshot's docs and chart

Report only; fix upstream.

- `SECURITY.md` links `security/token-caching.md`, which is not in the snapshot.
- The conformance-test README links `AGENTS.md`, which is not in the crate directory.
- `README.md` "License generation" runs `tools/generate_attributions.py`, which is not in the snapshot; the Docker example passes `BUNYAN=false`, which the service does not read.
- Released 0.0.53 only: with `secret.create: true`, `templates/secret.yaml` names the Secret after the release full name, but the Deployment mounts and reads `secret.name` (default `permission-service-secret`). Pre-create the Secret and set `secret.create: false` (also the secure choice). The 0.0.54 snapshot removes the chart-created Secret.
- `openapi.json` `info.version` is still `0.0.53` at the 0.0.54 snapshot, and it now carries a product-security tracking tag in `tags` that is not part of the API.
- `web-ui.md` documents `config.endpoints.userService`; the Permission UI chart in this snapshot reads `userService.enabled`/`userService.endpoint` instead.
- The in-repo `skills/permission-service/SKILL.md` says the Dockerfiles use an internal mirror of the distroless image and that `.cargo/config.toml` points at internal registries; in this snapshot the Dockerfiles use `gcr.io/distroless/cc-debian12`, there is no `.cargo/` directory, and `axum-observability` is a path dependency on `common/observability`. `CHANGELOG.md` `[0.0.54]` says the smoke and conformance images use distroless `cc:v4.1.4`; the snapshot Dockerfiles use `gcr.io/distroless/cc-debian12`.
- Storage Service [`storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst) shows the headless endpoint as `permissions-service-headless`; this chart renders `permission-service-headless`.
