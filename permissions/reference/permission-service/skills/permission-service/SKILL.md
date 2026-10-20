---
name: permission-service
description: Deploy, configure, test or understand the Omniverse Storage Permission Service. Use when asked to stand this service up against someone's infrastructure ("deploy permission-service", "deploy ovstorage"), to map their Postgres / OIDC provider / Kubernetes cluster onto Helm values, to run its test suites, or to answer questions about Cedar policy authoring, policy and metadata management, authorization evaluation, OIDC authentication, diagnostics, notifications, access logs, observability or the web UI. Carries this repository's deployment prerequisites and known gaps, and routes to the authoritative docs rather than restating them.
version: 0.0.54
---

# Permission Service

**Routing principle: this file is a map, not a copy of the documentation.** It exists so an agent can
reach the authoritative page in one hop, and so deployment know-how that is *not* in the docs
accumulates in one place. Read the linked page before answering; do not answer policy or
configuration questions from this file alone.

Everything referenced below lives in this repository unless a URL is given.

## What this component is

A centralized authorization microservice: given a **principal** (user or service identity), an
**action** (service-scoped operation) and an optional **resource**, it answers `allow`, `deny` or
`skip`. Rules are [Cedar](https://www.cedarpolicy.com/) policies, stored alongside service metadata
(registered services, their actions, resource types and evaluation priorities).

This repository is the **reference implementation** of the published API — other teams validate
integrations against it and may ship compatible alternatives.

Ports: HTTP/REST `3000`, gRPC `3010`. Health: `GET /health` on `3000`.

---

## Start here: what are you being asked to do?

| Task | Do this |
|---|---|
| **Deploy the service** into a cluster | Read [What the service needs from an environment](#what-the-service-needs-from-an-environment) below, then `docs/service/docs/deployment.md` end to end. |
| **Deploy it as part of a larger stack** ("deploy ovstorage") | Read [Where it sits in the stack](#where-it-sits-in-the-stack) below for ordering, then `docs/service/docs/deployment.md`. |
| **Map someone's existing infrastructure** onto this chart | [Infrastructure → Helm value](#infrastructure--helm-value) below, then the per-topic docs it points at. |
| **Run it locally** without Kubernetes | `README.md` → *Run with Docker (PostgreSQL)* and *Run with Docker (Config File)*. |
| **Run the tests** | [Running the tests](#running-the-tests) below. |
| **Understand or call the API** | `docs/api/docs/index.md`, then `docs/api/docs/rest-api.md` or `docs/api/docs/grpc.md`. |
| **Write or debug a policy** | `docs/service/docs/policy-language.md`, then `docs/service/docs/policy-evaluation.md`, then `docs/service/docs/diagnostics.md`. |
| **Work on the code in this repo** | `README.md` → building, proto regeneration and the release procedure. |

---

## What the service needs from an environment

This is the checklist to run against "here is what my infrastructure looks like". Everything here is
verified against `helm/permission-service/values.yaml`,
`helm/permission-service/templates/`, and `docs/service/docs/deployment.md` on this branch.

### Hard requirements

| Requirement | Detail |
|---|---|
| **Kubernetes + Helm 3** | The chart is `helm/permission-service` (`apiVersion: v2`). Namespace is yours to pick; the docs use `storage-apis`. |
| **Container image access** | `image.repository` defaults to `nvcr.io/nvidia/omniverse/permission-service`. This repo *is* published to the [NGC `storage_apis` collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis) — the container and the Helm chart both exist there — but pulls are authenticated. You need an NGC API key and an image-pull secret referenced from `image.pullSecrets`. See [Gaps and caveats](#gaps-and-caveats). |
| **CPU and memory headroom** | `resources.requests` defaults to **cpu `4`, memory `4Gi`** (limits cpu `6` / memory `4Gi`). A small dev cluster will leave the pod `Pending`; lower these before blaming the chart. |

### A storage backend — and it does not have to be Postgres

`database.type` selects between two backends. **Choosing this first shapes the whole deployment.**

| `database.type` | What you need | What you lose |
|---|---|---|
| `"config-file"` | Nothing external. Policies and metadata come from `database.init.policies` / `database.init.services`, which the chart renders into a ConfigMap mounted at `/etc/permission-config/`. | **Read-only.** Every policy or metadata write returns `501 Not Implemented`. Change policies by `helm upgrade`; the config checksum annotation rolls the pods. |
| `"postgres"` (chart default) | A reachable PostgreSQL. Any PostgreSQL-compatible database works — in-cluster (the docs walk through [CloudNativePG](https://cloudnative-pg.io/)) or managed (RDS/Aurora, Azure Database for PostgreSQL, Cloud SQL). | Nothing; this is the full-featured mode. |

If the user's infrastructure has no database, do **not** tell them the service is unusable — route
them to config-file mode. Full reference: `docs/service/docs/database-configuration.md`.

### An identity provider, unless you deliberately turn auth off

`openId.enabled` defaults to `true`. When it is on, `openId.openIdConfigurationUri` is **required**:
the service fetches the OIDC discovery document at startup and **exits if it is unreachable**, so a
wrong or unreachable IdP URL shows up as `CrashLoopBackOff`, not as a warning. It recovers on its own
once the URL answers — no redeploy needed.

`openId.enabled: false` accepts every request as an anonymous principal and skips meta-permission
checks entirely. That is a local-development and smoke-test mode only; the repo's own CI uses it
(`ENABLE_AUTH=false`). Never hand it to a user as a production configuration.

Full reference including the three `tokenVerificationType` strategies (`jwt`, `opaque`,
`jwtuserinfo`) and a worked Microsoft Entra ID example: `docs/service/docs/authentication.md`.

### The one thing people forget: meta-permissions

With authentication enabled, the policy-management, metadata and diagnostics APIs are themselves
gated by Cedar actions on the reserved `permissions` service — `permissions:view`,
`permissions:edit`, `permissions:meta:read`, `permissions:meta:write`, `permissions:diagnostics`.
**If none of these are granted to anyone, the service starts up healthy and nobody can administer
it.** Seed them through `database.init.policies` on the first deployment. This applies in both
storage modes. See the *Granting Administrative Permissions* section of
`docs/service/docs/deployment.md`.

### Optional, only if the user asks for it

| Capability | Needs | Values |
|---|---|---|
| Ingress / external access | **Contour** (`projectcontour.io/v1` `HTTPProxy` CRDs) | `httpProxy.enabled`, `httpProxy.fqdn.host` / `.domain` |
| TLS certificates for that ingress | **cert-manager** (`cert-manager.io/v1` `Certificate`) | `httpProxy.tls.enabled` + `httpProxy.tls.certManager.enabled` + `issuerRef`; or bring your own `httpProxy.tls.secretName` |
| Envoy external authorization in front | an auth extension service | `httpProxy.authExtension.enabled`, `.extensionRef` |
| Publishing policy-change events | Event Aggregation Service (gRPC) | `notifications.publisher.enabled`, `.endpoint` |
| **More than one replica** | Event Consumer Service (gRPC) | `notifications.consumer.enabled`, `.endpoint` — see the warning below |
| Metrics / traces | an OpenTelemetry collector | `observability.metrics.*`, `observability.tracing.*` |
| Outbound service identity for the above | an OIDC client-credentials client, or a mounted token file | `serviceIdentity.clientCredentials.*` **or** `serviceIdentity.accessTokenFile.*` — enable exactly one |

> **Scaling trap.** `replicaCount` defaults to `1`. Scaling past 1 without
> `notifications.consumer.enabled: true` leaves replicas serving stale decisions from their local
> caches until TTL expiry, because cross-replica cache invalidation rides on the consumer
> subscription. If a user says "we run 3 replicas", they need the Event Consumer Service and a
> configured `serviceIdentity`. See `docs/service/docs/notifications.md`.

### Infrastructure → Helm value

When handed "this is what my infra looks like", map it with this table, then read the linked doc
before writing the values file.

| They have… | Set | Doc |
|---|---|---|
| A Postgres host/port/user/db | `database.postgres.host`, `.port`, `.user`, `.dbname`, `.sslMode` | `docs/service/docs/database-configuration.md` |
| A Postgres password | Either `database.postgres.password` with `secret.create: true` (chart makes the secret), **or** a pre-made secret holding `POSTGRES_PASSWORD` with `secret.name: <name>` and `secret.create: false` | `docs/service/docs/deployment.md` |
| Short-lived / rotated DB credentials | `database.postgres.tokenFile.enabled` + `.path` + `.pollInterval` (with a provisioner sidecar) | `docs/service/docs/database-configuration.md` |
| No database at all | `database.type: "config-file"` + `database.init.policies` + `database.init.services` | `docs/service/docs/database-configuration.md` |
| An OIDC provider | `openId.openIdConfigurationUri`, `openId.tokenVerificationType` | `docs/service/docs/authentication.md` |
| Registered client apps | `openId.allowedJwtAudience[]` — every accepted `aud` value (client ids or API identifiers); required when `openId.enabled` is true and `openId.tokenVerificationType` is `jwt` or `jwtuserinfo` | `docs/service/docs/authentication.md` |
| A non-standard user-id or groups claim | `openId.principalIdClaim`, `openId.principalGroupsClaim` | `docs/service/docs/authentication.md` |
| Only some clients, tenants, or groups may call it | `openId.requiredJwtClaims[]` — `claim`, `operator` (`equals`, `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, `exists`), `value`/`values` | `docs/service/docs/authentication.md` |
| Services to authorize (e.g. storage-service) | `database.init.services[]` — `name`, `actions`, `resourceTypes[].type`, `.evaluationPriority` | `docs/service/docs/metadata.md` |
| Legacy resource URLs that must be canonicalised | `config.urlRemapper.enabled` + `rewriteRules` (or `rewriteRulesRef`) — keep identical to Storage Service's `config.urlRemapper` | `docs/service/docs/deployment.md` |
| A DNS name to expose it on | `httpProxy.enabled`, `httpProxy.fqdn.*` | `docs/service/docs/deployment.md` |
| An OTel collector endpoint | `observability.metrics.endpoint.grpc`, `observability.tracing.endpoint.grpc` | `docs/service/docs/observability.md` |

Every chart value is surfaced to the container as an environment variable of the same meaning
(`DATABASE_TYPE`, `POSTGRES_HOST`, `ENABLE_AUTH`, `OPENID_CONFIGURATION`, `JWT_AUDIENCE`,
`PRINCIPAL_ID_CLAIM`, `ENABLE_NOTIFICATIONS`, …). When debugging a deployed pod, compare
`helm/permission-service/templates/deployment.yaml` against the pod's actual env. For a
non-Kubernetes run, `cargo run --bin permission-service -- --help` lists every variable.

---

## Where it sits in the stack

Deploy in this order:

1. **PostgreSQL** (skip in config-file mode) — must be reachable before the pod starts.
2. **Identity provider** — must serve its discovery document before the pod starts, or the pod
   crash-loops.
3. **permission-service** — seed meta-permissions and the service metadata for everything that will
   call it, in the same `helm install`.
4. **Callers.**

**Depends on:** PostgreSQL (optional), an OIDC identity provider (optional), Event Aggregation
Service and Event Consumer Service (both optional, for notifications and cross-replica cache
invalidation). None of these live in this repository.

**Depended on by:**

- **storage-service** calls it for every permission check. Its chart points at
  `http://permission-service:3010` (gRPC) and is gated by its own `permissionsEnabled` value. That
  value, and which storage endpoints consult this service, belong to the `storage-service` skill —
  not here.
- **envoy-auth-extension** can sit in front as an Envoy external-authorization gRPC service; wire it
  through `httpProxy.authExtension`.
- **permission-ui**, the web UI, is a *separate* chart and image (`permission-ui`, port `8080`), not
  part of `helm/permission-service`. Callers through it need the meta-permissions above.
  See `docs/service/docs/web-ui.md`.
- The chart's default `database.init.services` already seeds `storage-service`,
  `event-consumer-service` and `event-aggregation-service` — treat it as the reference list of what
  normally sits on top.

Cross-component questions beyond this (choosing a storage adapter, composing the whole stack, wiring
Discovery) belong to the `ovstorage` guide.

---

## Running the tests

| Suite | Command | Needs |
|---|---|---|
| Whole workspace | `cargo test` | **A running PostgreSQL** — see below. |
| Lint gate (blocking in CI) | `cargo fmt --all --check` then `cargo clippy --all-targets --all-features -- -D warnings` | — |
| Chart | `helm lint helm/permission-service` | — |
| Smoke test | container `permission-smoke-test` against a running service; `SERVICE_URL` env | see [Gaps and caveats](#gaps-and-caveats) |
| Conformance | `conformance-test-setup` binary seeds metadata+policy, then the conformance container runs. Flags and env vars are in `README.md`. | a running service |

**`cargo test` fails closed without Postgres.** 61 of the 69 tests in
`crates/permission-db/src/tests.rs` call `connect_database()`, which reads `POSTGRES_HOST`,
`POSTGRES_USER`, `POSTGRES_PASSWORD` and `POSTGRES_DBNAME` with `.expect(...)` and panics if any is
missing. CI starts `postgres:17.2-alpine` before `cargo test`; reproduce that locally:

```bash
docker run --name postgres -p 5432:5432 \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -d postgres:17.2-alpine
POSTGRES_HOST=localhost POSTGRES_USER=postgres POSTGRES_PASSWORD=postgres \
  POSTGRES_DBNAME=postgres cargo test
```

The remaining 8 `permission-db` tests (config-file storage and validation helpers) and the suites in
`permission-core`, `permission-auth` and `permission-service` do not need a database.

---

## Gaps and caveats

State these plainly rather than letting an agent discover them by failing.

- **Container vulnerability scans of the three images report HIGH/CRITICAL CVEs.** The scans are
  non-blocking, so a failing scan on an otherwise green pipeline **is not a regression you
  introduced.** Do not chase it when validating an unrelated change.
- **Build the container images from this directory.** All three Dockerfiles in `docker/` use this
  directory as the build context (`docker build -f docker/Dockerfile .`) and the public
  `nvcr.io/nvidia/distroless/cc` runtime base, so they build without NVIDIA-internal access.
- **`axum-observability` and `otel-tools` are vendored under `vendor/`.** They are NVIDIA's own
  crates, used as path dependencies; the unrelated `axum-observability` on crates.io is not used.
  Everything else comes from crates.io, so `cargo build` needs no registry configuration.
- **`permission-service` (container) and `permission-service` (Helm chart) are listed in the public
  NGC catalog; `permission-smoke-test` and `permission-service-conformance-test` are not.** So
  `smokeTest.enabled: true` (default `false`) and `helm/permission-service-conformance-test` will
  fail to pull outside NVIDIA. Do not put them in a user-facing deployment plan.
- **`helm/permission-service/Chart.yaml` carries `version`/`appVersion` `0.0.0`.** The
  release process rewrites them. A chart installed straight from a git checkout
  therefore resolves `image.tag` to `0.0.0`, which does not exist — set `image.tag` explicitly, or
  use the chart published to NGC.

---

## Route by task

Every path below exists on this branch.

### Service documentation (`docs/service/docs/`)

| If the question is about… | Go to |
|---|---|
| What the service is, and the doc index | `docs/service/docs/index.md` |
| **Deploying** — chart values, both storage modes, CloudNativePG, meta-permissions, URL remapper, ingress, clean-up | `docs/service/docs/deployment.md` |
| Storage backends, init file formats, scope inference | `docs/service/docs/database-configuration.md` |
| OIDC identity-provider setup, token validation, outbound service identity | `docs/service/docs/authentication.md` |
| **Writing** Cedar policies — syntax, entities, conditions, groups | `docs/service/docs/policy-language.md` |
| **Managing** policies over REST — create, update, delete | `docs/service/docs/policy-management.md` |
| How a request turns into allow / deny / skip, priorities | `docs/service/docs/policy-evaluation.md` |
| Registering services, actions, resource types (metadata) | `docs/service/docs/metadata.md` |
| Diagnostics API — inspecting why a decision came out as it did | `docs/service/docs/diagnostics.md` |
| Notifications — publishing policy changes, cross-replica cache invalidation | `docs/service/docs/notifications.md` |
| Metrics, tracing, structured logging, log levels | `docs/service/docs/observability.md` |
| API access-log records — what is logged, what is deliberately not | `docs/service/docs/access-logs.md` |
| The permission web UI (separate chart and image) | `docs/service/docs/web-ui.md` |

### API contract (`docs/api/docs/`)

| If the question is about… | Go to |
|---|---|
| The published API contract, and where to start | `docs/api/docs/index.md` |
| REST surface | `docs/api/docs/rest-api.md` |
| gRPC surface, including the discovery `CapabilitiesService` | `docs/api/docs/grpc.md` |
| Authorizing a **user** principal end to end | `docs/api/docs/authorization-for-a-user-principal.md` |
| Authorizing a **service** principal end to end | `docs/api/docs/authorization-for-a-service-principal.md` |
| External specifications the contract cites | `docs/api/docs/references.md` |

### In the repository root

| Thing | Where |
|---|---|
| Local Docker runs, proto regeneration, release procedure, attributions | `README.md` |
| Helm chart | `helm/permission-service/` |
| Conformance-test chart | `helm/permission-service-conformance-test/` |
| Release history | `CHANGELOG.md` |
| Published OpenAPI document | `openapi.json` |

---

## If you got stuck

A wrong route, a missing prerequisite or a command that did not work is a **bug in this skill or in
the doc it pointed at** — not something to work around silently. When a deployment or a question
goes sideways:

1. **Say what you actually hit** — the value you could not find, the step that failed, the page that
   did not cover the case.
2. **Decide where the fix belongs.** If the authoritative doc is wrong or incomplete, fix
   `docs/…` — that is the durable home, and doc updates ride along with chart and source changes
   anyway. If the doc is right but an agent could not get
   to it, or the knowledge is operational rather than documentary (a prerequisite, an ordering
   constraint, a known-failing job), fix **this file**.
3. **Prefer adding a row to an existing table** over adding prose. The value of this skill is that it
   stays scannable.
4. **Do not paste doc content in here to work around a routing miss.** If the answer belongs in the
   docs, put it in the docs and point at it.

Facts recorded here must be checked against the branch before they are written down — a confidently
wrong prerequisite costs more than a missing one.
