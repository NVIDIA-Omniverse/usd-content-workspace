# Add authentication and authorization to a deployment

> **Applies to:** Envoy auth extension, Permission Service, Permission UI, and the OIDC settings of every service that takes bearer tokens · **Released:** envoy-auth-extension 2.4.0 (image `envoy-auth-ext` 2.4.0), permission-service 0.0.53, permission-ui 0.0.22; API resource `permission-api` 1.0.0-beta.1 (usd-content-workspace Developer Guide 1.3.0 / NGC) · **This repo's snapshot:** envoy-auth-extension 2.4.1, permission-service 0.0.54, permission-ui 0.0.22 changelog / 0.0.23 package (see [../versions/source-snapshot.md](../versions/source-snapshot.md)) · **Sources:** `permissions/README.md`; `permissions/reference/{envoy-auth-extension,permission-service,permission-ui,storage-permission-ui}/`; the [source skill package](../../NOTICE.md)

The snapshot Envoy auth extension (2.4.1) and Permission Service (0.0.54) use different `openId` keys from the released 2.4.0 and 0.0.53, and 0.0.54 no longer creates its database Secret. The sub-pages give values for both; pick the one that matches the chart you pulled.

Authentication and authorization are separate layers here:

- **Authentication** — the [Envoy auth extension](permissions-and-auth/envoy-auth-extension.md) validates the bearer token at the Contour edge for every protected host. The Permission Service also validates tokens itself.
- **Authorization** — the [Permission Service](permissions-and-auth/permission-service.md) evaluates Cedar policies when Storage, notifications, User Info, or your service asks "may this principal do this?".
- **Administration** — the [Permission UI](permissions-and-auth/permission-ui.md) and the Storage Permission UI panel edit policies through the Permission Service management API.

Explain by default; install only when asked, in the user's namespace and with their credentials. Confirm the versions you are working with first ([../versions/capture-a-deployment.md](../versions/capture-a-deployment.md)) and read each chart's `values.yaml` and `CHANGELOG.md`.

## Request path once everything is on

```text
client ──TLS──▶ Contour Envoy ──h2c Check──▶ Envoy auth extension ──JWKS (cached)──▶ IdP
                   │ allow (adds user-id / username headers; keeps Authorization)
                   ▼
            Storage / Discovery / notifications / User Info / your service
                   │ CheckPermission(principal?, action, resource)  + caller's or service's token
                   ▼
            Permission Service ──▶ PostgreSQL (policies, metadata)
                   └──▶ Event Aggregation (omni.permissions.changed), Event Consumer (cache invalidation)
```

## Order of work

Each step assumes the previous ones are healthy. Storage, Discovery, Contour, DNS, and TLS come from [production-stack.md](production-stack.md).

| # | Step | Done when | Page |
|---|---|---|---|
| 1 | Register the API, browser, and service clients in the IdP; decide audience, issuer, and principal claim (`oid` on Entra) | You can obtain a user token and a service token whose `aud` is `<api-client-id>` | [oidc-and-token-audiences.md](permissions-and-auth/oidc-and-token-audiences.md) |
| 2 | Create Secrets out of band (pull secret, TLS, database, client credentials) | `kubectl get secret` shows them; no secret values in any values file | [oidc-and-token-audiences.md](permissions-and-auth/oidc-and-token-audiences.md#secrets-reference) |
| 3 | Install the Envoy auth extension | `ExtensionService auth-extension` exists; pods ready | [envoy-auth-extension.md](permissions-and-auth/envoy-auth-extension.md) |
| 4 | Enable Discovery `auth-config` and the auth extension on every data-serving `HTTPProxy`; redeploy those charts | Anonymous call returns `401`; `/api/v1/auth-config` returns the client bootstrap | [discovery.md](discovery.md), [production-stack.md](production-stack.md) |
| 5 | Provision PostgreSQL and install the Permission Service with seeded admin and service policies | `/health` 200; a known allow and a known deny behave as expected | [permission-service.md](permissions-and-auth/permission-service.md), [cedar-policies.md](permissions-and-auth/cedar-policies.md) |
| 6 | Turn on enforcement in callers: Storage `config.permissions.enabled`, notification `permissions.enabled`, User Info `permission.enabled` | A user without a grant is refused; one with a grant succeeds | [production-stack.md](production-stack.md), [notifications.md](notifications.md), [userinfo.md](userinfo.md) |
| 7 | Optional: Permission UI; Navigator with the Storage Permission UI panel | Admin can list policies; non-admin gets `403` | [permission-ui.md](permissions-and-auth/permission-ui.md), [navigator.md](navigator.md) |
| 8 | Register the Permission Service (and User Info) with Discovery if clients must find them | Catalog lists `permission` / `userinfo` entries | [permission-service.md](permissions-and-auth/permission-service.md#register-with-discovery), [discovery.md](discovery.md) |

Enable enforcement in callers (step 6) only after the policies they need exist; with auth on, an empty policy store denies everything.

## Ports and names (snapshot charts)

| Component | Kubernetes Service | gRPC | REST / HTTP | Health | External host (when `httpProxy.enabled`) |
|---|---|---|---|---|---|
| Envoy auth extension | `envoy-auth-extension-service` | 50051 (`Check`) | — | gRPC health; probes TCP (2.4.0) or gRPC (2.4.1, Kubernetes 1.27+) | none (Contour calls it in-cluster) |
| Permission Service | `permission-service`, `permission-service-headless` | 3010 | 3000 | `GET /health` | `permissions.<domain>`, `grpc.permissions.<domain>` |
| Permission UI | `permission-ui` | — | 8080 (nginx) | `GET /` | `web.permissions.<domain>` |

Ports are chart values; read them from the chart you deploy. TLS certificates for the hosts above need `permissions.<domain>`, `grpc.permissions.<domain>`, and `web.permissions.<domain>` in addition to the hosts listed in [production-stack.md](production-stack.md) (a `*.permissions.<domain>` wildcard covers the `grpc.` and `web.` hosts). With User Info exposed, add `userinfo.<domain>` and `grpc.userinfo.<domain>`.

## Security baseline

Hold these regardless of environment size; a reviewer should be able to check each one.

- **Authentication stays on.** `openId.enabled: true` on the Permission Service; auth extension on every data-serving `HTTPProxy`, including both REST and gRPC hosts. The only unauthenticated edge routes are Discovery `auth-config` and the static Permission UI.
- **Tokens are bound to this deployment.** Audience lists contain your API client id, not the `some_client_id` placeholder (released charts: `clientRegistrations` + `additionalJwtAudience`; snapshot charts: `allowedJwtAudience`). Issuer lists are set (empty means `iss` is not checked in every version) on both the Envoy extension (`openId.jwtIssuers` in 2.4.0, `openId.allowedJwtIssuers` in 2.4.1) and the Permission Service (`openId.allowedJwtIssuers`). On the snapshot charts, consider a `requiredJwtClaims` client allow-list.
- **Least privilege in Cedar.** No `permit(principal, action, resource);`. Meta-permissions only for named admin groups. Storage grants per group and subtree. Service principals limited to the actions their code calls. Forbids ordered before permits where the resource type uses `evaluationPriority: permit` (the chart default for storage `object`/`folder`).
- **Users and services are distinct principals.** Users come from interactive flows (PKCE); services use client credentials or, preferably, workload identity through an access-token file. A service that asks about a user needs `permissions:view`, narrowed by `context.action.service`.
- **Secrets never in values.** Permission Service 0.0.53 `secret.create: false` (0.0.54 never creates it; `database.postgres.secretRef`), `clientSecretRef`, and `secretRef` everywhere. Short-lived database tokens (`tokenFile`) where the platform supports them. `sslMode: require` for databases outside the cluster network.
- **Network exposure is minimal.** Permission Service ClusterIP unless an external caller needs it. NetworkPolicies admit only Contour Envoy and named in-cluster callers to the Permission Service, User Info, and the auth extension. CORS `allowOrigin` narrowed from `*` to the UI origins.
- **Changes are observable.** Structured logs and OTLP metrics on; `policy.mutation.count` and `authorization.request.denied` dashboards; Permission Service publishes change events; on 0.0.54, API access logs where your audit policy needs them.
- **Logs do not hold credentials.** The released Envoy auth extension 2.4.0 records bearer tokens and cookies from `Check` requests in logs and traces (fixed in 2.4.1); restrict those logs ([permissions-and-auth/envoy-auth-extension.md](permissions-and-auth/envoy-auth-extension.md#security-notes)).

Stack-wide review points beyond this area: [../stack/security.md](../stack/security.md).

## Validation checklist

```bash
# 1. edge: anonymous refused, bootstrap public
curl -s -o /dev/null -w '%{http_code}\n' https://<domain>/api/v1/services          # 401
curl -s https://<domain>/api/v1/auth-config                                        # JSON bootstrap

# 2. authorization service healthy and enforcing (from a pod or port-forward)
curl -sf http://permission-service.<namespace>.svc.cluster.local:3000/health
#    then one expected allow and one expected deny — see permission-service.md#validate

# 3. end to end: a user without a storage grant is refused by Storage; one with a grant succeeds
```

Record each result (status code, decision, versions) with the deployment record. Integration tests that exercise permissions: [../test/integration-tests.md](../test/integration-tests.md). Runbooks for `401`/`403` storms and Permission Service outages: [../operate/runbooks.md](../operate/runbooks.md).

## Differences from the source skill package

| Topic | Source skill package said | This repo shows |
|---|---|---|
| Meta-permission names | `permissions:view`, `:edit`, `:meta`, `:diagnostics` | `permissions:view`, `:edit`, `:meta:read`, `:meta:write`, `:diagnostics` |
| Permission Service config-file example | `openId.enabled: false` for a first smoke test | Chart default `true`; keep auth on and seed admin policies instead |
| Permission UI User Info URL | `config.endpoints.userService` | snapshot chart reads `userService.enabled` + `userService.endpoint` |
| Envoy auth extension | "Envoy sidecar" with no ports | separate Deployment; gRPC 50051; `ExtensionService auth-extension` |
| Envoy values | `image.pullSecrets` only | also set `image.tag` (`0.0.0` placeholder), audience, issuers; 2.4.1 renames them to `allowedJwtAudience` / `allowedJwtIssuers` and fails on the old keys |
| `database.init` metadata key | `database.init.metadata.services` (API reference) | `database.init.services` |
| Permission API errors | `413`, `422`, explicit `DECISION_DENY` for malformed input | `400`/`INVALID_ARGUMENT` for malformed input; no `413`/`422` in the contract |
