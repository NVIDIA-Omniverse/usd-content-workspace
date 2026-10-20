# Security across the stack

> **Applies to:** every component · **Released:** Developer Guide 1.3.0 inventory · **This repo's snapshot:** Storage Service 1.2.3 + unreleased, Envoy auth extension 2.4.1 (released 2.4.0), Permission Service 0.0.54 (released 0.0.53) (others in [../versions/source-snapshot.md](../versions/source-snapshot.md)) · **Sources:** [`SECURITY.md`](../../../../SECURITY.md); Storage Service [`storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst), [`storage-credentials.rst`](../../../../storage/reference/storage-service/docs/helm/storage-credentials.rst), [`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md); Envoy auth extension [`README.md`](../../../../permissions/reference/envoy-auth-extension/README.md), [`CHANGELOG.md`](../../../../permissions/reference/envoy-auth-extension/CHANGELOG.md), [`values.yaml`](../../../../permissions/reference/envoy-auth-extension/helm/values.yaml), and vendored [`permission-auth`](../../../../permissions/reference/envoy-auth-extension/vendor/permission-auth/src/lib.rs); Permission Service [`CHANGELOG.md`](../../../../permissions/reference/permission-service/CHANGELOG.md) and [`access-logs.md`](../../../../permissions/reference/permission-service/docs/service/docs/access-logs.md); Permission API [service principal guide](../../../../permissions/reference/permission-service/docs/api/docs/authorization-for-a-service-principal.md)

Use this as the review checklist for a cloud deployment. Component pages under [../deploy/](../deploy/) give the full settings.

## Trust boundaries

| Boundary | Control | Owner component |
| --- | --- | --- |
| Internet or corporate network → cluster | TLS at Contour; only intended hosts exposed | Contour `HTTPProxy` created by each chart |
| Ingress → service | Bearer token validated by the Envoy auth extension before proxying | Envoy auth extension |
| Service → authorization | Per-operation Cedar decision | Permission Service; Storage `config.permissions` |
| Service → service | Service principal token, usually from the client credentials grant | each calling service; Permission Service validates |
| Service → backend | Workload identity or least-privilege cloud credentials | Storage Service |
| Service → database or broker | Credentials in Kubernetes Secrets; TLS where offered | Storage metadata database, Permission Service PostgreSQL (`database.type: postgres` default), RabbitMQ |

## Authentication at the edge

The Envoy auth extension implements Envoy's external authorization `Check` RPC as a Contour `ExtensionService`. It reads `Authorization: Bearer <token>`, verifies it against the configured OpenID provider (`openId.tokenVerificationType`: `jwt`, `opaque`, or `jwtuserinfo`), and returns allow, 401, or 403.

The released chart and image are 2.4.0; this repo's source is 2.4.1, which changes key names and tightens validation. Key names below give both.

- **Set the issuer list.** 2.4.0: `openId.jwtIssuers`; 2.4.1: `openId.allowedJwtIssuers` (the old name fails the render). In **both** versions an empty list means the `iss` claim is not validated, and the discovery-document issuer is not added implicitly, so list every trusted issuer. When the list is set, 2.4.1 also rejects tokens that omit `iss`; per the 2.4.1 changelog, 2.4.0 accepted such tokens. The Permission Service uses `openId.allowedJwtIssuers` in both 0.0.53 and 0.0.54 with the same empty-means-unchecked rule.
- **Restrict audiences.** 2.4.0 accepts `openId.clientRegistrations[*].clientId` plus `openId.additionalJwtAudience`; 2.4.1 accepts only `openId.allowedJwtAudience`. Both ship the placeholder `some_client_id`; replace it with your API client id. 2.4.1 (and Permission Service 0.0.54) reject tokens that carry no `aud`; 2.4.0 (and 0.0.53) accepted them, per their successors' changelogs. Never accept any audience.
- **Restrict calling clients (2.4.1 / 0.0.54).** `openId.requiredJwtClaims` can require, for example, that `azp` or `appid` is one of your registered client ids. No equivalent exists in the released versions.
- **Clock checks.** 2.4.1 and Permission Service 0.0.54 enforce `nbf` and apply `config.jwtLeeway` (default 60 s, at most 300 s) to `exp` and `nbf`; the released versions do not check `nbf`.
- **Keep `openId.maxTokenLength`** (8192 bytes by default) and the token and negative caches bounded. Cache TTLs (`config.tokenTtl` 1200 s, `config.jwksTtl` 10800 s on the extension) delay the effect of revocation and key rotation; shorten them if your IdP revokes tokens you must honor quickly. 2.4.1 also bounds IdP calls with `config.idpConnectTimeout` / `idpRequestTimeout` (5 s / 15 s).
- **Treat 2.4.0 extension logs as secret.** Per the 2.4.1 changelog, 2.4.0 records the `Check` request, including bearer tokens and cookies, in logs and traces, and logs a principal hint from the unverified token on failure. Restrict access to those logs and traces and their retention, or move to 2.4.1 when it is released, which stops both and defaults to `info`.
- **The extension forwards identity headers upstream** (`user-id`, `username`, `username-source-claim` by default). Any service that trusts these headers must be reachable only through the Envoy path, never directly from clients.
- Prefer NVIDIA's released Envoy auth extension image; if you build it yourself (it is Apache-2.0), you own scanning, signing and patching it.

Details and migration steps: [../deploy/permissions-and-auth/envoy-auth-extension.md](../deploy/permissions-and-auth/envoy-auth-extension.md), [../deploy/permissions-and-auth/oidc-and-token-audiences.md](../deploy/permissions-and-auth/oidc-and-token-audiences.md).

## Authorization

- Storage Service authorizes through the Permission Service only when `config.permissions.enabled: true`; the default is `false`. A production deployment with users must enable it.
- The per-endpoint `permissionsEnabled` flag (default `true`) bypasses authorization for one endpoint. Use it only for intentionally public content. Storage Service 1.2.1 renamed it from `enablePermissions`; values that still use the old name silently lose the setting.
- Storage Service 1.2.1 fixed responses from the workspace manifest cache bypassing permission checks, and filters `ListRoutes` and URL-remapper aliases by caller permission. Deployments older than 1.2.1 should upgrade.
- Permission Service 0.0.54 (snapshot) resolves `.` and `..` segments, including `%2e%2e`, before applying `config.urlRemapper` rules. Per its changelog, 0.0.53 could rewrite an id such as `omniverse://host/NVIDIA/../Projects/scene.usd` with the `omniverse://host/NVIDIA/` rule and evaluate it against that location's policies. On 0.0.53 with a URL remapper, do not rely on prefix rules to separate locations whose ids callers can craft.
- Write least-privilege Cedar policies: grant actions on resource prefixes to groups, not to individuals or `*`. Grant the Permission Service meta-permissions (needed to manage policies, including through the Permission UI) to a small administrator group. Policy authoring and diagnostics: [../deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md) and [../apis/permissions.md](../apis/permissions.md).
- Service-to-service calls carry a service principal token, typically from the OAuth 2.0 client credentials grant. The callee asks the Permission Service to validate the token and authorize the action. Register each calling service separately in the IdP so its permissions can be scoped and revoked.

## Credentials and secrets

- Prefer cloud workload identity (EKS IRSA or Pod Identity, AKS Workload Identity, GKE Workload Identity) for backend access. Scope the role to the specific buckets or containers and the operations used.
- For Storage Service service identity, use the access token file provider (`serviceIdentity.accessTokenFile`, a sidecar writing a short-lived token to an in-memory volume). The client-secret provider (`serviceIdentity.clientCredentials`) is deprecated; if used, reference the secret with `clientSecretRef`, never inline it.
- Permission Service database credentials: on 0.0.53 set `secret.create: false` and pre-create the Secret (the default `true` copies `database.postgres.password` from values into Helm release history); 0.0.54 never creates it and reads `database.postgres.secretRef.name`.
- Keep every secret in a Kubernetes Secret or an external secret manager. Never put tokens, NGC API keys, client secrets, or database passwords in values files committed to Git, in chat, or in logs.
- An NGC API key is only for pulling charts and images. It is not a runtime identity for any service.
- `helm get values` output and support bundles can contain secret references and hostnames; redact before sharing.

## Workload hardening

- Storage Service 1.2.3 charts run as non-root uid 1000, drop all capabilities, and forbid privilege escalation; the Envoy auth extension chart also sets a read-only root filesystem. Keep these defaults, and check each other chart's `podSecurityContext` and `securityContext` in the version you deploy.
- No chart in this snapshot creates a `NetworkPolicy`. Add your own: allow ingress to each service only from Contour and from the services that call it; allow egress only to the backend, database, broker, IdP, and Discovery.
- Each service has its own `ServiceAccount`. The Auto Discovery chart grants its operator a `ClusterRole` and `ClusterRoleBinding` (see [`rbac.yaml`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/templates/rbac.yaml)); review the granted verbs and resources, and run one operator per cluster or tenant boundary you intend it to see.
- Pin images by digest in production and track base-image security updates in component changelogs (for example the Storage Service distroless base updates).
- Turn off what you do not use: unused routes and endpoints, and OpenAPI document serving (Storage Service 1.2.3 and later: `serveOpenapi`, default `true`; see `docs/helm/misc.rst`). The Envoy auth extension always serves gRPC health and reflection; keep its port cluster-internal.

## Network exposure

- Expose services only through Contour `HTTPProxy` with TLS. Internal ports (metrics, health, gRPC between services) stay on `ClusterIP`.
- Contour TLS secrets come from cert-manager (`httpProxy.tls.certManager` in the Storage chart) or your certificate process. Do not disable TLS validation in clients to work around certificate problems; fix the chain or the hostname.
- Notification subscriptions are long-lived streams; set ingress and route timeouts deliberately rather than removing them.

## Data protection and audit

- Enable backend encryption at rest (S3 SSE-KMS, Azure Storage encryption) and database encryption; the services do not add their own.
- Metadata migrations are not reversed by an image rollback; back up the metadata database before upgrades ([../operate/upgrades.md](../operate/upgrades.md)).
- Ship logs and traces through the observability settings ([../operate/monitoring.md](../operate/monitoring.md)). Do not log tokens or request bodies containing user content.
- Permission Service 0.0.54 and User Info 0.0.9 can emit API access logs (`observability.logging.access.enabled`, off by default). They never copy `Authorization` or `Cookie`, and they record rejected calls, which is useful for audit; but they do contain principal ids and resource ids (and, for policy writes, the policy text), so restrict and retain them like other audit data.

## Reporting vulnerabilities

Report suspected vulnerabilities privately to NVIDIA PSIRT at psirt@nvidia.com, as described in [`SECURITY.md`](../../../../SECURITY.md). Do not open public issues for them.
