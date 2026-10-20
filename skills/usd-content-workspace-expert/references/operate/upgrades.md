# Upgrade components from one version to another

> **Applies to:** every component in the stack, plus Live Edit (external) · **Released:** Developer Guide 1.3.0 inventory ([versions/released-inventory.md](../versions/released-inventory.md)) · **This repo's snapshot:** per-component versions in [versions/source-snapshot.md](../versions/source-snapshot.md); cited below where they matter · **Sources:** component `CHANGELOG.md` files, [`storage-service/docs/helm/migration.rst`](../../../../storage/reference/storage-service/docs/helm/migration.rst), [`url-normalizer.rst`](../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst), [`storage-metadata.rst`](../../../../storage/reference/storage-service/docs/helm/storage-metadata.rst), [Discovery `v1-to-v2-migration.md`](../../../../discovery/reference/discovery-service/operator/docs/v1-to-v2-migration.md), [thumbnail chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md) and [cutover runbook](../../../../thumbnails/reference/thumbnail-service/docs/cutover-runbook.md); the [source skill package](../../NOTICE.md)

Changing a version string is not an upgrade. Each crossed version can rename values, change defaults, migrate stored data, or change an API contract. This page gives the procedure and the known hazards per component; the component changelogs and migration guides are the authority.

## Procedure

### 1. Record what is installed

Capture chart versions, image tags and digests, effective values, advertised API versions and cluster facts with [capture-a-deployment](../versions/capture-a-deployment.md). Keep the output; it is the rollback target.

```bash
helm list -n <namespace> -o json
helm get values <release> -n <namespace> -o yaml > <release>.installed-values.yaml   # contains secret references; store securely
helm history <release> -n <namespace>
```

### 2. Choose the target and keep version sets apart

Pick either a released set (NGC inventory, [released-inventory](../versions/released-inventory.md)) or images you build from this repo's snapshot ([deploy/building-from-source.md](../deploy/building-from-source.md)). Do not mix a released chart with snapshot values, or the reverse; chart schemas differ. In this repo `Chart.yaml` files hold `0.0.0` placeholders, so read the real version from `CHANGELOG.md`, `Cargo.toml`, `pyproject.toml`, `package.json` or `SOURCE_VERSIONS.md`.

### 3. Read every crossed version

For each component, read the changelog entries strictly after the installed version up to and including the target, plus any migration guide. Look for: renamed or removed values (often silently ignored), new required values, default changes, stored-data migrations, new Kubernetes version requirements, API and status-code changes, metric renames.

### 4. Check API contracts

Clients and generated code must match the server's API version ([versions/api-contracts.md](../versions/api-contracts.md)). Confirm what the target advertises through Discovery (`/api/v2/services`, schema 2) and what clients use. Contract changes that need client work are listed per component below.

### 5. Diff and render

```bash
helm show values <new-chart> > new-defaults.yaml
diff <(yq -P 'sort_keys(..)' old-defaults.yaml) <(yq -P 'sort_keys(..)' new-defaults.yaml)
helm template <release> <new-chart> -n <namespace> -f <release>.values.yaml > new-rendered.yaml
helm get manifest <release> -n <namespace> > old-rendered.yaml
diff old-rendered.yaml new-rendered.yaml
```

Look at images, init containers, probes, security contexts, Service annotations (Discovery), and anything that disappeared. Keep values files in the original order; later files win.

### 6. Back up state that a rollback cannot restore

Storage metadata database (PostgreSQL or CosmosDB), Permission Service database, RabbitMQ definitions and durable queues, Valkey for Live Edit, and the thumbnail durable queues. An image rollback does not reverse a schema migration or make new-format data readable.

### 7. Upgrade in dependency order and verify outcomes

Dependencies first (databases, RabbitMQ, Valkey), then Storage, Permission Service and User Info, then Notifications, then Discovery, then Thumbnail and Live Edit, then UIs. After each step verify the originally failing or changed capability and the previously healthy ones, not just rollout status. Run the relevant smoke tests and conformance suites ([test/conformance.md](../test/conformance.md), [test/integration-tests.md](../test/integration-tests.md)).

### Roll back safely

Distinguish three failures: configuration rejected at startup, readiness or dependency failure, and an applied data migration. For the first two, restore the previous chart, image and values through the original mechanism (`helm rollback <release> <revision>` or the GitOps source; never a manual rollback that GitOps will undo). If data migrated, confirm the old version can read it; otherwise fix forward or restore the pre-upgrade backup and account for writes made since. Reconcile the final state into the deployment source.

## Per-component hazards

### Storage Service

Released 1.2.3; snapshot 1.2.3 plus Unreleased changes (`CHANGELOG.md`). Configuration reference: the in-repo skill `storage/reference/storage-service/skills/storage-service/SKILL.md` and [components/storage-service.md](../components/storage-service.md).

| Crossing | Change | Action |
| --- | --- | --- |
| → 1.2.0 | S3, Azure Blob and Azure Table metadata backends removed; DynamoDB and CosmosDB temporarily disabled at 1.2.0 (batch operations missing) | Migrate metadata to PostgreSQL before upgrading from 1.0.x with those backends; plan data conversion |
| → 1.2.0 | Flat S3 credential keys (`accessKeyId`, `secretAccessKey`, `sessionToken`, `assumedRole*`, `externalId`) no longer supported | Move to `credentials.accessKey.*` / `credentials.assumeRole.*` ([migration.rst](../../../../storage/reference/storage-service/docs/helm/migration.rst)) |
| → 1.2.0 | PostgreSQL migrations run in a bootstrap init container; token provisioner is a native sidecar | Kubernetes **1.29+** |
| → 1.2.0 | Requests to unconfigured S3 buckets or Azure accounts rejected at routing (`allowUnconfiguredEndpoints`) | List every endpoint clients use |
| → 1.2.1 | `enablePermissions` renamed `permissionsEnabled` (S3 and Azure endpoints); old key silently ignored | Rename before upgrading |
| → 1.2.1 | `serviceIdentity.clientCredentials` deprecated (1.2.2 restored the chart settings) | Prefer the access-token-file provisioner |
| → 1.2.1 | Cross-backend `Copy`/`Move` return `UNIMPLEMENTED` (REST 501), not `INVALID_ARGUMENT` | Update client error handling |
| → 1.2.1 | Discovery v2alpha advertises the REST version mount base (`/v1alpha`) | Clients join it with the endpoint name and OpenAPI paths |
| → 1.2.2 | Deprecated and unused chart properties removed; PostgreSQL continuation-token format changed | Diff values; do not reuse continuation tokens across the upgrade |
| → 1.2.3 | Pod runs as non-root uid 1000, capabilities dropped; Metadata API `v1beta` added beside `v1alpha` | Check volume permissions and custom sidecars |
| → snapshot Unreleased | URL normalizer enabled by default: collapses repeated `/` and resolves `.`/`..` after percent-decoding | Audit stored keys for empty segments and dot segments first, or set `config.urlNormalizer.enabled: false` ([url-normalizer.rst](../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst)) |
| → snapshot Unreleased | Header forwarding setting at `observability.headers.forward` | Move any earlier key there |

The 1.2.0 metadata restriction does not apply to 1.2.3, whose docs cover PostgreSQL and CosmosDB. Rollback limit: PostgreSQL migrations are applied forward only by the bootstrap container; rolling the image back does not undo them. Back up the database.

### Discovery

Released Auto Discovery operator 1.1.1 (and legacy chart 2.3.11 in the inventory); snapshot operator 1.2.0 (Cargo, unreleased), legacy chart 2.4.1 (`VERSION.toml`, unreleased).

- Legacy static chart → operator is a replacement, not a values rename: services opt in with Service annotations and are interrogated through `CapabilitiesService/ListServices` ([deploy/discovery.md](../deploy/discovery.md)).
- Clients move from `/api/v1/services` to `/api/v2/services`: require `"schema-version": 2`, select by endpoint name and version or by fully qualified gRPC service, not by v1 `type` ([v1-to-v2-migration.md](../../../../discovery/reference/discovery-service/operator/docs/v1-to-v2-migration.md)). The operator serves v1 compatibility entries from `config.v1Endpoints` during the transition; Storage Navigator 1.0.4 prefers v2 with v1 fallback.
- Snapshot operator 1.2.0 (unreleased): the primary catalog route is `/api/v2beta/services`, with `/api/v2/services` kept as an alias returning the same `schema-version: 2` body, so existing v2 clients keep working. Auth bootstrap is served at `/api/{v1,v1alpha,v2,v2beta}/auth-config`, and the HTTPProxy exempts all four. The chart probes move to `/api/v2beta/services`. The operator queries targets with capabilities **v2beta** `ListServices`, follows `next_page_token` (at most 100 pages), and falls back to the deprecated v2alpha on `UNIMPLEMENTED` with a deprecation warning, so targets that serve only v2alpha (every service in this snapshot) still register. The operator's conformance runner checks gRPC capabilities at v2beta only and fails v2alpha-only targets.
- The operator does not roll its pod when only the v1 mapping ConfigMap changes; restart the Deployment after such an upgrade (operator `docs/service/docs/operations.md`).

### Notifications

Released Aggregation 1.6.0, Consumer 1.11.0; snapshot Aggregation 1.7.0, Consumer 1.12.0 (`pyproject.toml`, unreleased), both on notification-common 7.0.0. Full guidance: [operate/notifications.md](notifications.md), [deploy/notifications.md](../deploy/notifications.md).

- Consumer 1.11.0: durable queue broker names no longer embed the caller identity (`event-queue:<queue_id>`). **Queues created by earlier versions are unreachable after upgrading; clients must recreate them.** Queue ids now share one deployment-wide namespace. Durable queues expire after 24 h without a consumer by default; clients must keep their `queue_id` and filter groups.
- Consumer 1.11.0: a token whose principal claim is present but empty is rejected; refusals move to `UNAUTHENTICATED`/`401` (dashboards keyed on `UNKNOWN` must move); a render-time grace-period invariant can fail existing overrides.
- Aggregation 1.6.0: authentication runs before request validation (some `400`/`422` become `401`); `terminationGracePeriodSeconds` now 60 and render-checked; REST body limit `413` at 4 MiB.
- Aggregation 1.7.0 (snapshot, potentially breaking): per-event limits on both APIs (≤ 1000 events per batch, `message` ≤ 65536 bytes as JSON, `event_type` ≤ 256, `resource_id` ≤ 2048); over-limit requests get `422`/`INVALID_ARGUMENT`.
- notification-common 7.0.0, Aggregation 1.7.0, Consumer 1.12.0 (snapshot, potentially breaking; from each `CHANGELOG.md` `[Unreleased]` and the library's `[7.0.0]`):
  - **Authentication default.** Chart `principalSource` defaults to `""` instead of `jwtUnverified`. With OpenID configured (`openId.openIdConfigurationUri` plus an audience from the `default` client registration or `openId.additionalJwtAudience`) and no `principalSource`, the release moves to `jwt` with no values change. Before upgrading, check that callers' tokens pass: expired tokens, which `jwtUnverified` accepted, now get `401`/`UNAUTHENTICATED`; a token whose `aud` is not accepted is refused; the pods need egress to the IdP discovery and JWKS endpoints. A successful render does not prove these.
  - **No OpenID and no `principalSource`:** the render fails. Add OpenID or set `principalSource` explicitly (`disabled` only for a local or test run).
  - **`jwtUnverified`** renders only with `principalSource: jwtUnverified` and `allowUnverifiedJwt: true`, which sets `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`; the service refuses to start in that mode otherwise. Use it only when every path to the pods verifies tokens first, including a NetworkPolicy (or equivalent) that keeps the Service unreachable except through the verifying proxy.
  - **Render guards** now refuse, at `helm template`/`helm upgrade`, auth combinations the service would refuse at startup ([deploy/notifications.md](../deploy/notifications.md#authentication-and-authorization)).
  - **REST CORS.** The services answer CORS only for `rest.corsAllowOrigins` (aggregation) / `service.rest.corsAllowOrigins` (consumer), default empty; `*` and `null` fail the render. Browser clients that call either REST API directly, not through the HTTPProxy's `httpProxy.corsPolicy`, need their origins listed.
  - **Local runs** need `OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start` for a tokenless run on your own machine; never use `disabled` for a shared deployment.
  - **Rollback:** `helm rollback` to the previous release, or keep the new release and set `principalSource: jwtUnverified` with `allowUnverifiedJwt: true`, but only where every path to the pods verifies tokens first. Rolling back to 1.6.0 / 1.11.0 brings back their defaults (`jwtUnverified`, CORS for every origin with credentials), so set `principalSource` explicitly in the restored values.
- Both services prefer `observability.*` over the deprecated `telemetry` block.

### Permission Service, Envoy auth extension, User Info

Released Permission Service 0.0.53, envoy-auth-extension 2.4.0, User Info 0.0.8; snapshot 0.0.54, 2.4.1, 0.0.9. Details: [deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md), [deploy/userinfo.md](../deploy/userinfo.md).

- Permission Service 0.0.52 (breaking): `permissions:meta` split into `permissions:meta:read` and `permissions:meta:write`; seed policies must grant the new actions. HTTP metrics renamed to `http.server.*`.
- Permission Service 0.0.53: authorization metrics lose `principal`, `action`, `resource` attributes; the custom millisecond `http.server.request.duration` histogram is removed in favor of the seconds-based one.
- Envoy auth extension 2.4.0: `token_cache_size` is now honored (the old `jwt_cache_size` key was ignored); new JWT issuer, claim, token-length, negative-cache and JWKS backoff settings in the chart; `log_user_claim` removed in favor of `principal_id_claim`.
- Permission Service 0.0.54 (snapshot, breaking): accepted audiences move to `openId.allowedJwtAudience`; `clientRegistrations` and `additionalJwtAudience` are ignored, and with `jwt` or `jwtuserinfo` verification an empty list stops startup. The chart no longer creates the Postgres Secret: create it first and set `database.postgres.secretRef.name`, because Helm deletes a Secret the old chart created. JWTs without `aud`, without `iss` when issuers are listed, or before `nbf` are rejected. The URL remapper resolves dot segments before matching. Unused `autoscaling.*` keys are removed. Details: [deploy/permissions-and-auth/permission-service.md](../deploy/permissions-and-auth/permission-service.md).
- Envoy auth extension 2.4.1 (snapshot, breaking): `openId.allowedJwtAudience` replaces `clientRegistrations` and `additionalJwtAudience`, and `jwtIssuers` is renamed `allowedJwtIssuers`; the old keys fail the render, so remove `clientRegistrations` from any overlay shared with this chart. `config.otel.endpoint` moves to `observability.*`. Probes are gRPC and need Kubernetes 1.27+. `aud` is required and `nbf` enforced. `Check` requests, which 2.4.0 logged with their bearer tokens and cookies, are no longer logged. Details: [deploy/permissions-and-auth/envoy-auth-extension.md](../deploy/permissions-and-auth/envoy-auth-extension.md).
- User Info 0.0.9: `LOG_STRUCTURED` renamed `STRUCTURED_LOGGING`; access logs and header forwarding added.

### Thumbnail Service

Released 0.1.0; snapshot 0.1.0.

- Values layout changed from the flat orchestrator plus `worker:` shape to root runtime keys plus `thumbnailService.*` and `thumbnailWorker.*`. Use the migration table in the [chart README](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/README.md#values-migration-post-182--current). `observability.logging.structured.enabled` is gone (JSON always); `app.generatedThumbnailPrincipalIdentity` is gone.
- Legacy env vars (`SERVICE_ACCOUNT_CLIENT_ID`, `SERVICE_ACCOUNT_CLIENT_SECRET[_FILE]`, `TENANT_ID`, `TOKEN_URI`, `TOKEN_*` tuning) were removed with no fallback; old names fail startup. Use `CLIENT_ID`, `CLIENT_SECRET[_FILE]`, `CLIENT_CREDENTIALS_OPENID_CONFIGURATION`.
- Queue ids are derived from `resourcePrefixes`; `NOTIFICATIONS_QUEUE_ID` no longer exists. Changing prefixes moves both components to new queues.
- Combined with Event Consumer 1.11.0, durable queues created before the consumer upgrade are unreachable; the thumbnail binaries recreate theirs on startup, but undelivered work on the old queues is not carried over. Drain first.
- From the retired Python task queue: follow the [cutover runbook](../../../../thumbnails/reference/thumbnail-service/docs/cutover-runbook.md) (no dual dispatch; worker queue verified before the orchestrator publishes; backfill with `scripts/create-missing-thumbnails.sh`). That runbook still names the pre-merge `helm/thumbnail-task-worker` chart and a chosen worker queue id; in this snapshot both Deployments come from `helm/thumbnail-service` and the id is derived.
- Startup now aborts if no bearer arrives within 60 s; use a native sidecar for the token provisioner.

Rollback: restore orchestrator and worker together. Stop new work first (`helm rollback` of the release, or disable StorageEventQueue), let the worker drain until `thumbnail_worker_unacknowledged_deliveries` is zero, then scale it down. Never delete WorkerEventQueue during a rollback; recreating it does not recover its messages.

### Live Edit (external)

Released 0.1.1; no source here. Earlier preparation used 0.1.0. Re-read the 0.1.1 chart's `values.yaml`: the default checkpoint settings refuse to start, so choose durability explicitly ([deploy/live-edit.md](../deploy/live-edit.md)). Helm rollback restores configuration, not Valkey data or checkpoints. After any restore from checkpoint, clients reload layers from empty state.

### Contracts and libraries

- Storage API: released resource `storage-api` 1.0.0-beta.4; snapshot `storage/api` unreleased (after 1.0.0-beta.4) (adds native metadata, `IsVersioned`/`IsReadOnly`, stricter versioned-identity conformance). Generated clients and custom adapters must match what the deployment serves ([apis/storage.md](../apis/storage.md), [develop/custom-storage-adapter/](../develop/custom-storage-adapter/README.md)).
- Metadata API: Storage 1.2.3 serves `nvidia.omniverse.metadata.v1beta` alongside `nvidia.omniverse.storage.metadata.v1alpha` ([apis/metadata.md](../apis/metadata.md)).
- Storage Core: pre-1.0 alpha crates with unreleased changes; they may break between minor versions; read `storage/reference/storage-core/*/CHANGELOG.md`.
- Observability libraries: Unreleased removes the deprecated OTel gRPC server metric layer and changes A66 size reporting; dashboards may shift ([monitoring.md](monitoring.md)).
- Client Library: release baseline 2.72, examples 2.74 ([components/client-library.md](../components/client-library.md)); verify the Kit runtime's actual client version before relying on 2.74 behavior.
