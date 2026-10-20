# Deploy the Permission Service

> **Applies to:** Permission Service chart and image · **Released:** chart `permission-service` 0.0.53, image `nvcr.io/nvidia/omniverse/permission-service:0.0.53` (usd-content-workspace Developer Guide 1.3.0) · **This repo's snapshot:** 0.0.54 (`Cargo.toml`, `CHANGELOG.md`; `openapi.json` still says 0.0.53) · **Sources:** `permissions/reference/permission-service/helm/permission-service/{values.yaml,templates/*}`, `docs/service/docs/{deployment,database-configuration,authentication,notifications,observability,access-logs}.md`, `CHANGELOG.md`, `README.md`, `SECURITY.md`, the [source skill package](../../../NOTICE.md) production stack "Permission Service"

Default ports: REST 3000, gRPC 3010. The upstream [`deployment.md`](../../../../../permissions/reference/permission-service/docs/service/docs/deployment.md) is the step-by-step guide; this page is the production checklist with the security decisions made explicit. Read the `values.yaml` of the chart version you install; the defaults below are from the 0.0.54 snapshot unless marked 0.0.53.

**Released 0.0.53 versus snapshot 0.0.54.** NGC publishes 0.0.53. The 0.0.54 source in this repo changes the token-audience keys and the database Secret handling, and an unmigrated values file is silently accepted by Helm but leaves the pod in CrashLoopBackOff (no audience) or unable to mount its Secret. The sections below give both forms; the full list is in [../../components/permission-service.md](../../components/permission-service.md#version-notes).

## Decide first

| Decision | Options | Recommendation |
|---|---|---|
| Policy store | `database.type: postgres` (default; runtime CRUD through the API and the Permission UI) or `config-file` (read-only YAML from a ConfigMap; writes return `501`; files reload on change) | `postgres` for anything users administer; `config-file` for static, GitOps-reviewed policy sets |
| Inbound auth | `openId.enabled` (default `true`) | Keep `true`. With `false`, every request is anonymous and the meta-permission gates are skipped, so anyone who can reach the port can rewrite policy. |
| Exposure | ClusterIP only, or Contour `HTTPProxy` (`httpProxy.enabled`) | Expose externally only if a browser UI or an out-of-cluster service needs it; then enable TLS and the [Envoy auth extension](envoy-auth-extension.md) on the proxy |
| Accepted token audience | 0.0.53: `openId.clientRegistrations[*].clientId` + `openId.additionalJwtAudience`; 0.0.54: `openId.allowedJwtAudience` (required for `jwt`/`jwtuserinfo`) | Only your API client id; never leave the `some_client_id` placeholder ([oidc-and-token-audiences.md](oidc-and-token-audiences.md#audience-design)) |
| Replicas | `replicaCount`; `autoscaling.enabled: true` only drops `replicas` for an autoscaler you supply (the chart renders no HPA) | ≥ 2 for availability, **with** `notifications.consumer.enabled: true` so replicas invalidate each other's caches |
| Outbound identity | `serviceIdentity.clientCredentials` or `.accessTokenFile` (exactly one, only if notifications are on) | `accessTokenFile` with a workload-identity sidecar |

## Database credentials

PostgreSQL settings live under `database.postgres` (`host`, `port` 5432, `user`, `dbname` default `postgres`, `sslMode`, `connections` 16, `workerThreads` 32). The password reaches the pod as a **file** (`POSTGRES_PASSWORD_FILE=/etc/secrets/POSTGRES_PASSWORD`) from a Secret: named by `secret.name` in 0.0.53, by `database.postgres.secretRef.name` (default `permission-service-secret`) in 0.0.54.

1. Pre-create the Secret.
   - **0.0.53:** also set `secret.create: false`. With the default `secret.create: true` the chart renders `database.postgres.password` from values into a Secret (it then lives in Helm release history), and that Secret is named after the release, not `secret.name`, so the mount can fail ([../../components/permission-service.md](../../components/permission-service.md#known-defects-in-this-snapshots-docs-and-chart)).
   - **0.0.54:** the chart never creates the Secret; `secret.create`, `secret.name`, and `database.postgres.password` no longer exist and are ignored if set. The Secret must exist before `helm install`, or the pod cannot start. When a release that used `secret.create: true` is upgraded to 0.0.54, Helm deletes the chart-created Secret (named after the release, not `permission-service-secret`) because it is no longer rendered; create the `secretRef` Secret before upgrading. The chart does not watch the Secret, so restart the Deployment after rotating it.
2. Use a dedicated database role that owns only the Permission Service database (CloudNativePG `bootstrap.initdb.owner`, or a managed-service role). Do not use the `postgres` superuser shown in the README's local Docker example.
3. Set `sslMode: require` for any database reached over a network you do not fully control (managed RDS / Azure Database / Cloud SQL). Accepted values in this snapshot are `""`/`disable` (default), `prefer`, and `require`; there is no `verify-full`. Whether `require` verifies the server certificate is unverified in this snapshot, so combine it with private networking.
4. Prefer short-lived credentials: `database.postgres.tokenFile.enabled: true` with a `provisionerSidecar.container` that writes the token JSON (`access_token`, RFC 3339 `expires_on`) to an in-memory volume. The pool is rebuilt when the token rotates; the pod waits `credentialWaitTimeout` (30s) for the file at startup and then fails closed.

```bash
# password from a CloudNativePG cluster named permission-pg (owner role: permission-service)
PGPASSWORD=$(kubectl get secret permission-pg-app -n <namespace> -o jsonpath='{.data.password}' | base64 -d)
kubectl create secret generic permission-service-secret -n <namespace> \
  --from-literal=POSTGRES_PASSWORD="$PGPASSWORD"
unset PGPASSWORD
```

If `database.postgres.user` is empty, the pod reads `POSTGRES_USER` from the same Secret.

## Production values

`permission-values.yaml` (combine with the OIDC overlays from [oidc-and-token-audiences.md](oidc-and-token-audiences.md#shared-values-overlays)):

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

replicaCount: 2

database:
  type: "postgres"
  init:
    policies: []            # seed least-privilege policies; see cedar-policies.md
    # services: keep the chart's default seed, or replace it with the services you deploy
  postgres:
    host: "<postgres-host>"
    port: 5432
    user: "permission-service"
    dbname: "permissions"
    sslMode: "require"
    secretRef:
      name: permission-service-secret   # 0.0.54; on 0.0.53 use the secret block below instead

# 0.0.53 only:
# secret:
#   name: permission-service-secret
#   create: false

config:
  enableDenyReason: true     # deny reason "Explicit deny." is returned to callers; set false if callers must not learn that a forbid matched
  cacheControlMaxAge: 0      # >0 lets clients cache decisions; that delays revocations by up to this many seconds

notifications:
  publisher:
    enabled: true
    endpoint: "http://event-aggregation-service:50051"
  consumer:
    enabled: true            # required with more than one replica
    endpoint: "http://event-consumer-service:50051"

serviceIdentity:
  accessTokenFile:
    enabled: true
    provisionerSidecar:
      container:
        name: token-provisioner
        image: "<your-token-provisioner-image>"
        # writes serviceIdentity.accessTokenFile.path (default /etc/secrets/access-token/token);
        # the chart adds the token-exchange mount (0.0.54: at that path's directory)

observability:
  logging:
    structured:
      enabled: true
    access:                  # 0.0.54 chart; see "API access logs" below before enabling
      enabled: false
      headers: {}
  metrics:
    enabled: true
    endpoint:
      grpc: "<otlp-collector:4317>"
```

Leave `httpProxy.enabled: false` unless it must be reachable from outside the cluster. When enabled:

```yaml
httpProxy:
  enabled: true
  fqdn: { host: "permissions", domain: "<dns-domain>" }   # REST permissions.<domain>, gRPC grpc.permissions.<domain>
  tls: { enabled: true, secretName: "<tls-secret>" }
  corsPolicy:
    allowOrigin: ["https://web.permissions.<dns-domain>"] # default is "*"; narrow it to the UIs that call this API
  authExtension:
    enabled: true
```

Notification endpoints and the event services they reach: [../notifications.md](../notifications.md). Turn on `notifications.*.tls.enabled` (with `caCertificateSecretRef`) when the hop to the Notification Service leaves a trusted network, because the service bearer token travels on it.

## Seed policies before you need them

With auth on, an empty policy store means nobody can list, create, or delete policies, edit metadata, or run diagnostics (`403`), and every authorization check is an implicit `deny`. Seed through `database.init.policies` on first install: in Postgres mode they load as **system policies** that the API refuses to delete. Least-privilege patterns for administrators, service principals, storage, notifications, and User Info: [cedar-policies.md](cedar-policies.md).

Do not ship `permit(principal, action, resource);`. The upstream quick start uses it for testing; it grants every action, including all five meta-permissions, to every authenticated caller.

## Install and upgrade

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/permission-service-0.0.53.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf permission-service-0.0.53.tgz

helm upgrade --install permission-service ./permission-service -n <namespace> \
  -f permission-values.yaml -f oidc-config.yaml -f oidc-permission.yaml
kubectl rollout status deploy/permission-service -n <namespace>
```

The overlay file names are those of the released-chart layout in [oidc-and-token-audiences.md](oidc-and-token-audiences.md#shared-values-overlays); for 0.0.54 use `-f oidc-config.yaml -f oidc-validators.yaml` plus a file with `openId.enabled: true`.

Changes to `database.init.*` or the URL remapper roll pods automatically (config checksum annotation). Environment-backed settings are read at startup. Before upgrading, diff `values.yaml` and read [`CHANGELOG.md`](../../../../../permissions/reference/permission-service/CHANGELOG.md); 0.0.53 changed metric attributes, so update dashboards ([../../operate/monitoring.md](../../operate/monitoring.md), [../../operate/upgrades.md](../../operate/upgrades.md)).

Moving from 0.0.53 to 0.0.54 (when it is published):

1. Copy every accepted audience (the `clientRegistrations` client ids that tokens really carry, plus `additionalJwtAudience`) into `openId.allowedJwtAudience`, then delete the old keys. 0.0.54 ignores them; with an empty `allowedJwtAudience` and `jwt`/`jwtuserinfo`, the pod logs `Failed to initialize authentication: JWT audience is not specified` and stays in CrashLoopBackOff.
2. Create the database Secret and set `database.postgres.secretRef.name`; delete `secret.*` and `database.postgres.password`.
3. Delete `autoscaling.minReplicas`, `maxReplicas`, and `targetCPUUtilizationPercentage` (they were never used).
4. Confirm every token your callers send carries `aud`, and `iss` if `allowedJwtIssuers` is set; 0.0.54 rejects tokens without them and enforces `nbf` within `config.jwtLeeway`.
5. If you use the URL remapper, re-check rules against ids that contain `.` or `..` segments: 0.0.54 resolves them before matching, so such ids may now map to a different prefix (the intended one).
6. If `serviceIdentity.accessTokenFile.path` is not under `/etc/secrets/access-token/`, update the token provisioner sidecar to write where the new mount is (the path's directory).

## Register with Discovery

The chart adds no Discovery annotations. The service implements `CapabilitiesService` (service type `permission`), and the Service port named `grpc` is picked automatically:

```yaml
service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
    discovery.omniverse.nvidia.com/visibility: "internal"   # "external" or "both" only if clients outside the cluster call it
```

`service.annotations` is applied to both the regular and the headless Service, so both opt in; check the catalog for two entries and set `service.headless.enabled: false` if that is a problem and you do not use client-side gRPC balancing. Operator behaviour: [../discovery.md](../discovery.md).

## Validate

```bash
kubectl port-forward -n <namespace> svc/permission-service 3000:3000 &

curl -sf http://localhost:3000/health

# token from your IdP for a principal with a known policy; do not paste tokens into tickets or chat
curl -s -X POST http://localhost:3000/v1beta/authorization/ \
  -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/json' \
  -d '{"action":{"service":"storage-service","name":"read"},
       "resource":{"type":"object","id":"https://<bucket-host>/Projects/Scene.usd"}}'
```

Check both an `allow` you expect and a `deny` you expect. For an unexpected result, call `POST /v1beta/diagnostics/authorize/` (needs `permissions:diagnostics`) to see the candidate policies in evaluation order. The optional Helm smoke test (`smokeTest.enabled: true`, then `helm test permission-service -n <namespace> --logs`) creates, reads, checks, and deletes a policy (Postgres mode only). It authenticates with the release's own `serviceIdentity`, so with auth on that identity needs `permissions:edit` and `permissions:view` — rights the Permission Service principal should not hold in production. Run the smoke test in a non-production release, or remove the grant afterwards. Run the conformance suite against the deployment as described in [../../test/conformance.md](../../test/conformance.md).

## Operational limits and signals

- Startup fails (CrashLoopBackOff) while the OIDC discovery URL is unreachable; it recovers by itself. In 0.0.54 it also fails, and does not recover, when `openId.allowedJwtAudience` is empty for `jwt`/`jwtuserinfo`, or when `openId.requiredJwtClaims` contains an invalid rule.
- Caches: evaluation 10000 entries / 15 min, policy sets 1000 / 1 h, metadata 100 / 1 h, tokens 2048 / 30 min, invalid tokens 256 / 60 s, JWKS 3 h. Without the cross-replica consumer, a replica that did not handle a write serves stale decisions for up to `evaluationCacheTtl`.
- `config.jwtLeeway` (60 s) is capped at 300 s; larger values are rejected at startup.
- Default resources: requests 4 CPU / 4 Gi, limits 6 CPU / 4 Gi; `config.workerThreads` 32 plus `database.postgres.workerThreads` 32.
- Metrics are OTLP push only. Key series: `authorization.request.allowed` / `.denied` (by `service`, `deny_reason`), `policy.mutation.count`, `notification.publish.count`, `notification.consume.connected`. See [`observability.md`](../../../../../permissions/reference/permission-service/docs/service/docs/observability.md).
- Public surfaces even with auth on: `/health`, `/v1beta/idp/keys/`, Swagger UI at `/`, `/openapi.json`, gRPC health, reflection, and `CapabilitiesService`. None returns policy data, but they reveal the API shape; keep the Service off the public internet unless required.

## API access logs

The 0.0.54 chart can emit one structured access record per business REST or gRPC call ([`access-logs.md`](../../../../../permissions/reference/permission-service/docs/service/docs/access-logs.md)). It is off by default (`observability.logging.access.enabled: false` → `ACCESS_LOG_ENABLED`). Records are `info` events on the tracing target `access`, so `observability.logging.level` must allow that target (`info` does; `error,access=info` keeps only access records); they are pretty text or nested JSON according to `observability.logging.structured.enabled`.

- Authentication failures, unknown routes and methods, and malformed payloads are logged as call-level records without request parameters, which makes rejected access auditable. Health, `/openapi.json`, Swagger UI, gRPC health, reflection, and capability discovery are never logged.
- `Authorization` and `Cookie` are never copied. Other request headers are copied only if you map them in `observability.logging.access.headers` (for example `{ip: "x-forwarded-for", ua: "user-agent"}`); there is no built-in request id, so map your gateway's correlation header if you need to group records of one batch.
- Records carry `principal.sub`, actions, and resource ids for authorization calls, and the submitted policy text for policy writes. Treat access logs as personal and security-sensitive data: restrict readers and set retention to your audit policy.
- Whether the released 0.0.53 chart has these values is not verified (the changelog lists the feature under 0.0.53, but the earlier 0.0.53 release-candidate source did not include it); check the pulled `values.yaml`.
