# Deploy the Microsoft Graph User Info Service

> **Applies to:** `microsoft-graph-userinfo-service` chart and image (writer and reader) · **Released:** chart and image 0.0.8 (usd-content-workspace Developer Guide 1.3.0); API resource `userinfo-api` 1.0.0-alpha.1 · **This repo's snapshot:** 0.0.9 (`Cargo.toml`, `CHANGELOG.md`) · **Sources:** `user-identity/reference/microsoft-graph-userinfo-service/{README.md,SECURITY.md,CHANGELOG.md}`, `docs/service/docs/{deployment,identity-provider,database,directory-synchronization,directory-scoping,static-directory,authorization,scaling,observability,access-logs,troubleshooting}.md`, `helm/microsoft-graph-userinfo-service/{values.yaml,templates/service.yaml}`, `src/http.rs`, the [source skill package](../../NOTICE.md)

The User Info API is the provider-agnostic contract ([../apis/userinfo.md](../apis/userinfo.md)); this page deploys NVIDIA's Entra ID implementation. Ports: REST 8080, gRPC 50051; `/healthz` and `/readyz` on REST. Doc map: [../components/userinfo-service.md](../components/userinfo-service.md). Read the `values.yaml` of the chart you install; values below are from the 0.0.9 snapshot. Keys added after 0.0.8 are marked.

## Architecture in one paragraph

A single **writer** synchronizes users, groups and (optionally) memberships from Microsoft Graph into PostgreSQL: one full sync, then `delta` increments every `providers.cache.syncInterval` (default `5m`). One or more **readers** load that snapshot into memory, reload it every `reader.reloadInterval` (default `60s`), and serve REST and gRPC. The Kubernetes Service (`userinfo-service`) routes only to readers. The result is eventually consistent. **Live mode** (writer, cache and Postgres off) sends every request to Graph; it is for initial testing only.

## Prerequisites

- Kubernetes with Helm 3, `kubectl`, `ngcpull-secret` in the namespace ([artifact-access.md](artifact-access.md)).
- A Microsoft Entra identity for Graph reads — app registration (client secret or federated credential) or user-assigned managed identity (workload identity) — with **application** permissions and tenant-admin consent.
- PostgreSQL for the synchronized deployment.
- An authenticating gateway in front of the API: the Envoy auth extension ([permissions-and-auth/envoy-auth-extension.md](permissions-and-auth/envoy-auth-extension.md)). The service does not validate caller tokens itself.
- Optional: the Permission Service, for per-endpoint and per-item authorization.

## Entra: grant the minimum

| Permission (application) | Grant when |
|---|---|
| `User.Read.All` | always |
| `Group.Read.All` | always |
| `GroupMember.Read.All` | only if `providers.cache.memberships.enabled`, membership endpoints are used, or directory scoping is on |
| `Application.Read.All` | only if `appRoleAssignments.servicePrincipalObjectId` points at a **different** application — avoid this; leave it unset to read the service's own assignments |

The service requests `https://graph.microsoft.com/.default` with the client-credentials grant. Use a **dedicated** registration or managed identity for this; it is not the stack's API audience ([permissions-and-auth/oidc-and-token-audiences.md](permissions-and-auth/oidc-and-token-audiences.md#audience-design)).

Prefer `providers.entra.authMode: workload-identity`. The chart then labels pods `azure.workload.identity/use: "true"` and annotates the chart-created ServiceAccount with `azure.workload.identity/client-id`; you create the federated credential in Entra. If you bring your own ServiceAccount (`serviceAccount.create: false`), annotate it yourself. With `client-secret`, create the Secret out of band:

```bash
# AZURE_CLIENT_SECRET comes from your secret store; it never goes in values
kubectl create secret generic microsoft-graph-userinfo-service-az-credentials -n <namespace> \
  --from-literal=client-secret="${AZURE_CLIENT_SECRET}"
```

**Limit what the directory exposes.** By default every user and group in the tenant is served to anyone who passes the gateway. To serve only the groups (and their direct members) assigned to the service's own enterprise application, enable directory scoping. It requires the cache with groups and memberships on, and the writer refuses to start rather than fall back to the whole tenant if the assignment read fails:

```yaml
appRoleAssignments:
  enabled: true
  servicePrincipalObjectId: null     # own application; no Application.Read.All needed
  includeServicePrincipals: true     # assigned service principals appear as users
providers:
  cache:
    enabled: true
    groups: { enabled: true }
    memberships: { enabled: true }
```

Users are still synchronized tenant-wide; scoping restricts groups and memberships ([`directory-scoping.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/directory-scoping.md)). To restrict who may read which users, use the Permission Service integration below.

## PostgreSQL

- Dedicated role (default user `microsoft_graph_userinfo_service`) owning only the `graph-userinfo` database. CloudNativePG and Azure Database for PostgreSQL flexible server are documented in [`deployment.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/deployment.md#setting-up-postgresql).
- `providers.cache.postgres.sslMode` defaults to `disable`. Use `require` for any database outside the cluster network (mandatory on Azure Database for PostgreSQL); the server certificate is verified against the container's root store.
- Password from a Secret (`password.secretRef`, default `microsoft-graph-userinfo-service-pg-credentials` / key `password`), or better a rotating token: `providers.cache.postgres.tokenFile.enabled: true` with a `provisionerSidecar.container` (for example Entra authentication to Azure Database). The token file takes precedence over the static password; empty or expired credentials are rejected.
- Connection budget: (readers + 1 writer) × `connections` (default 4) + headroom. Keep the writer's budget ≥ 2, because a full sync holds one connection for the advisory lock.

```bash
kubectl create secret generic microsoft-graph-userinfo-service-pg-credentials -n <namespace> \
  --from-literal=password="${POSTGRES_PASSWORD}"
```

## Values for a production deployment

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

providers:
  entra:
    tenantId: "<tenant-id>"
    clientId: "<graph-reader-client-id>"
    authMode: "workload-identity"
  cache:
    enabled: true
    users: { enabled: true }
    groups: { enabled: true }
    memberships: { enabled: false }   # turn on only if membership endpoints are needed; see sizing
    syncInterval: "5m"
    postgres:
      enabled: true
      host: "<server>.postgres.database.azure.com"
      port: 5432
      dbname: "graph-userinfo"
      user: "microsoft_graph_userinfo_service"
      password:
        secretRef: { name: "microsoft-graph-userinfo-service-pg-credentials", key: "password" }
      sslMode: "require"

writer:
  enabled: true
reader:
  enabled: true
  replicaCount: 2
  reloadInterval: "60s"

permission:                      # optional; see below
  enabled: true
  url: "http://permission-service.<namespace>.svc.cluster.local:3000/"
  serviceName: "userinfo"
  cacheTtl: "5m"
  cacheMaxCapacity: 10000

httpProxy:
  enabled: true
  fqdn: { host: "userinfo", domain: "<domain>" }   # REST userinfo.<domain>, gRPC grpc.userinfo.<domain>
  tls: { enabled: true, secretName: "<tls-secret>" }
  corsPolicy:
    allowOrigin: ["https://web.permissions.<domain>", "https://navigator.<domain>"]   # default "*"
  authExtension:
    enabled: true                # required: the service trusts the gateway for authentication

service:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"

observability:
  logging:
    structured: { enabled: true }
    access: { enabled: true }    # 0.0.9+; not in the released 0.0.8 chart
```

Install:

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/microsoft-graph-userinfo-service-0.0.8.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf microsoft-graph-userinfo-service-0.0.8.tgz
helm upgrade --install userinfo-service ./microsoft-graph-userinfo-service -n <namespace> -f userinfo-values.yaml
kubectl get deploy,pods,svc,httpproxy -n <namespace>
```

Remove keys your chart version does not define (for example `observability.logging.access` on 0.0.8); Helm silently ignores unknown keys, so a setting you believe is on may be off.

## Authentication and network exposure

The reader trusts the gateway. It reads the caller's `oid` from the bearer token **without verifying it** to serve `/users/me`, and with `permission.enabled: false` it serves every request that reaches it. Therefore:

- Enable `httpProxy.authExtension` on the User Info proxy (REST and gRPC hosts). The chart default is `false`.
- Restrict the ClusterIP Service with a NetworkPolicy that admits only Contour Envoy and the named in-cluster callers (Permission UI traffic arrives via Envoy; Navigator and the migration tool via Envoy or explicitly allowed pods).
- Turn on the Permission Service integration if callers must not all see the whole directory. It forwards the caller's token to the Permission Service, which **does** validate it, and fails closed (`500`) if the Permission Service is unreachable. Register the `userinfo` service metadata and policies first ([permissions-and-auth/cedar-policies.md](permissions-and-auth/cedar-policies.md#user-info-access)); otherwise every request is refused. Exempt: `/healthz`, `/readyz`, `/users/me`.

## Static directory overlay

`staticDirectory.users|groups|memberships` (with `staticDirectory.enabled: true`) adds entries that are not in Entra — typically CI and service accounts. Overlay entries **override** synchronized records with the same `id`, are never written to PostgreSQL, and are matched first for `/users/me`. Review overlay ids so they cannot shadow real users, and keep the overlay under change control ([`static-directory.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/static-directory.md)).

## Discovery

The reader serves `CapabilitiesService` with service type `userinfo`; the Service port named `grpc` (50051) is used automatically. Add `discovery.omniverse.nvidia.com/enabled: "true"` (and `visibility` if needed) through `service.annotations`, then confirm the `userinfo` entry and its client-reachable endpoints in the Discovery catalog ([discovery.md](discovery.md)). The source skill package referred to "Auto Discovery 1.1.1" for this; use whichever Discovery the deployment runs.

## Scaling and availability

- Readers scale horizontally (`reader.replicaCount`, or `autoscaling.enabled: true` to let an external HPA own the count; the chart creates no HPA). Each reader holds the full directory in memory.
- The writer is always one replica and is off the request path; a writer outage pauses freshness only.
- The chart sets no anti-affinity and no PodDisruptionBudget; add them through `affinity` and your own PDB for HA.
- Sizing: defaults are requests 500m / 1Gi, limits 1 CPU / 2Gi per container. With memberships on, a directory of ~100,000 users and ~170,000 groups can need ~2.5 GB for the membership index alone, and the first full sync can take hours. Readers become ready immediately and serve an empty or partial directory until the writer has saved a complete one.

## Validate

```bash
kubectl run userinfo-health --rm -i --restart=Never -n <namespace> --image=curlimages/curl -- \
  -sf http://userinfo-service.<namespace>.svc.cluster.local:8080/healthz
kubectl run userinfo-ready --rm -i --restart=Never -n <namespace> --image=curlimages/curl -- \
  -sf http://userinfo-service.<namespace>.svc.cluster.local:8080/readyz
```

`/readyz` means a reader is serving, **not** that the directory is populated. Also check that the writer pod is ready and that `GET /api/v1alpha/users?limit=5` through the gateway returns real users, and that the same call without a token returns `401` at the edge. Optional Helm smoke test: `smokeTest.enabled: true`, then `helm test userinfo-service -n <namespace> --logs`. Through an auth gateway it needs `serviceIdentity` credentials for the **User Info audience** (a Graph token is rejected). Contract conformance: [../test/conformance.md](../test/conformance.md).

## Troubleshooting pointers

| Symptom | Look at |
|---|---|
| Writer never ready; Graph `403` | Missing application permission or admin consent; scoping pointed at another app without `Application.Read.All` |
| Empty or partial results after install | First full sync still running (writer not ready), or reader not yet reloaded |
| Stale names or memberships | `syncInterval` + `reloadInterval`; delta link expiry forces a full resync (`410 Gone`) |
| `429` / `502` | Graph throttling or errors; `providers.entra.maxRetries` (default 3) |
| `500` on every call with authorization on | Permission Service unreachable, or `userinfo` metadata/policies missing |
| Readers crash-loop on start | PostgreSQL unreachable at startup (readers exit until they can connect) |

Full guide: [`troubleshooting.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/troubleshooting.md); metrics: [`observability.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/observability.md); operations runbooks: [../operate/runbooks.md](../operate/runbooks.md).

## Released 0.0.8 versus snapshot 0.0.9

Snapshot-only: API access logs (`observability.logging.access.*`), header forwarding to the Permission Service (`observability.headers.forward`), `LOG_STRUCTURED` renamed to `STRUCTURED_LOGGING`, `nvcr.io/nvidia/distroless/cc:v4.1.4` base images, hardened conformance Job. The ports, the `providers`/`writer`/`reader`/`permission` keys, and the deployment shape above are the same as in the source skill package's 0.0.8 description. Check the pulled chart before relying on any 0.0.9 key.
