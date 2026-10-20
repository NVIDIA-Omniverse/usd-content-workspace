# OIDC wiring, token audiences, and secrets

> **Applies to:** Envoy auth extension, Permission Service, Permission UI, Discovery `auth-config`, User Info Service, and every service that validates or presents a bearer token · **Released:** envoy-auth-extension 2.4.0, permission-service 0.0.53, permission-ui 0.0.22, microsoft-graph-userinfo-service 0.0.8, auto-discovery-service 1.1.1 (usd-content-workspace Developer Guide 1.3.0) · **This repo's snapshot:** envoy-auth-extension 2.4.1, permission-service 0.0.54, permission-ui 0.0.22/0.0.23, userinfo 0.0.9 (see [../../versions/source-snapshot.md](../../versions/source-snapshot.md)) · **Sources:** `permissions/reference/permission-service/helm/permission-service/{values.yaml,templates/_helpers.tpl,templates/deployment.yaml}`, `docs/service/docs/authentication.md`, `crates/permission-auth/src/{config.rs,lib.rs,claim_rules.rs}`, and `CHANGELOG.md`; `permissions/reference/envoy-auth-extension/helm/{values.yaml,templates/configmap.yaml,templates/_helpers.tpl}` and `CHANGELOG.md`; `permissions/reference/permission-ui/helm/permission-ui/templates/_helpers.tpl`; `user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/identity-provider.md` and `helm/.../values.yaml`; `discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml`; the [source skill package](../../../NOTICE.md) production stack "Authentication"

Every service in the stack trusts one OpenID Connect identity provider. Most failures after enabling auth come from four settings that must agree across charts: the **discovery URL**, the accepted **audiences**, the accepted **issuers**, and the **principal id claim**. Confirm chart versions first ([../../versions/capture-a-deployment.md](../../versions/capture-a-deployment.md)); key names differ between charts **and between the released and snapshot versions** of the Envoy auth extension and Permission Service.

## Who validates what

| Component | Validates inbound tokens? | Settings: released (2.4.0 / 0.0.53) | Settings: snapshot (2.4.1 / 0.0.54) |
|---|---|---|---|
| Envoy auth extension | Yes, at the Contour edge, for each `HTTPProxy` that enables `httpProxy.authExtension` | `openId.openIdConfigurationUri`, `openId.clientRegistrations[*].clientId` + `openId.additionalJwtAudience` (audience), `openId.jwtIssuers`, `openId.principalIdClaim`, `openId.tokenVerificationType` | `openId.openIdConfigurationUri`, `openId.allowedJwtAudience`, `openId.allowedJwtIssuers`, `openId.requiredJwtClaims`, `openId.principalIdClaim`, `openId.tokenVerificationType`; `config.jwtLeeway` |
| Permission Service | Yes, itself, on REST and gRPC (`openId.enabled: true` by default) | `clientRegistrations` + `additionalJwtAudience`, `allowedJwtIssuers`, `principalIdClaim`, `principalGroupsClaim`, `maxTokenLength` | `allowedJwtAudience` (required for `jwt`/`jwtuserinfo`), `allowedJwtIssuers`, `requiredJwtClaims`, `principalIdClaim`, `principalGroupsClaim`, `maxTokenLength` |
| Storage Service, notifications | Behind the Envoy edge; they forward the caller's token to the Permission Service. The notification charts keep their own `clientRegistrations` / `additionalJwtAudience` keys. | see [../production-stack.md](../production-stack.md), [../notifications.md](../notifications.md) | same |
| User Info Service | **No.** Requires a gateway; `/users/me` decodes `oid` unverified | gateway = Envoy auth extension on its `HTTPProxy` ([../userinfo.md](../userinfo.md)) | same |
| Discovery (Auto Discovery) | No; publishes the client bootstrap at `/api/v1/auth-config` with Contour auth disabled for that route | `openId.enabled`, `openId.openIdConfigurationUri`, `openId.clientRegistrations` ([../discovery.md](../discovery.md)) | same |
| Permission UI, Navigator UI | No; browser OIDC clients that obtain tokens | Permission UI: `openId.clientRegistrations` and `openId.openIdConfigurationUri` in its chart ([permission-ui.md](permission-ui.md)) | same |

The original `Authorization` header passes through Envoy, so the Permission Service validates the **same** token the edge validated. Its `aud` and `iss` must be accepted by both.

## Audience design

A token is accepted only if its `aud` is in the validator's list.

| Chart version | How the list is built | Placeholder default | Token without `aud` |
|---|---|---|---|
| Envoy 2.4.0, Permission Service 0.0.53 | sorted, de-duplicated union of `openId.clientRegistrations[*].clientId` and `openId.additionalJwtAudience` (`jwt_audience` / `JWT_AUDIENCE`) | `clientRegistrations: [{name: default, clientId: "some_client_id", scope: "a b c"}]` | accepted (per the 0.0.54 / 2.4.1 changelog security notes) |
| Envoy 2.4.1 | sorted, de-duplicated `openId.allowedJwtAudience`; setting the old keys fails the render | `allowedJwtAudience: ["some_client_id"]`; an empty list rejects every token | rejected (`401`) |
| Permission Service 0.0.54 | sorted, de-duplicated `openId.allowedJwtAudience`; the old keys are ignored | `allowedJwtAudience: []`; with `openId.enabled: true` and `jwt`/`jwtuserinfo`, an empty list leaves `JWT_AUDIENCE` unset and the service exits at startup (`Failed to initialize authentication: JWT audience is not specified`) | rejected (`401`) |

Recommended pattern on Microsoft Entra ID (from [`authentication.md`](../../../../../permissions/reference/permission-service/docs/service/docs/authentication.md#configuring-microsoft-entra-id)):

1. One **API app registration** for the stack. Under *Expose an API* add a scope (for example `access_as_user`). Its Application (client) ID `<api-client-id>` is what Entra v2.0 puts in `aud` for tokens issued to this API.
2. One **client app registration per caller type**: browser apps (Permission UI, Navigator), CLI/desktop apps, and each service identity. Each is granted only the delegated or application permission it needs on the API registration.
3. Clients request `<api-client-id>/.default` (the bare GUID form, not `api://…`), plus `openid profile email` for interactive sign-in.
4. Every validator accepts `<api-client-id>`: `additionalJwtAudience` on the released charts, `allowedJwtAudience` on the snapshot charts. On the released charts the `clientRegistrations` client ids are also accepted; replace the `some_client_id` placeholder rather than leaving it in the list.
5. To accept tokens only from known client applications, add a `requiredJwtClaims` rule on the snapshot charts (Entra v2.0 access tokens carry the client id in `azp`, v1.0 in `appid`):

```yaml
openId:
  requiredJwtClaims:
    - claim: "azp"
      operator: "in"
      values: ["<browser-client-id>", "<cli-client-id>", "<service-client-id>"]
```

Rules apply after signature, `exp`, `nbf`, `aud`, and `iss` checks; every rule must pass; a missing or `null` claim fails every rule; values must match the claim's JSON type exactly (`"1"` is not `1`); invalid rules stop the service from starting. A rejected token is recorded in the negative cache. In `opaque` mode the rules run against the `userinfo` response; in `jwtuserinfo` mode they run against the JWT claims only. The released 2.4.0 / 0.0.53 charts have no equivalent.

The Discovery chart rejects its placeholders when `openId.enabled: true`.

Do not reuse the User Info Service's **Graph reader** registration as the API audience. It requests `https://graph.microsoft.com/.default`, and its tokens carry the Graph audience, which the gateway correctly rejects.

## Issuers

| Chart | Key | Empty means | Set means |
|---|---|---|---|
| Permission Service 0.0.53 (released) | `openId.allowedJwtIssuers` | `iss` is not validated | listed `iss` required if present; per the 0.0.54 changelog, a token that omits `iss` is still accepted |
| Permission Service 0.0.54 (snapshot) | `openId.allowedJwtIssuers` | `iss` is not validated | a token without `iss`, or with an unlisted `iss`, is rejected |
| Envoy auth extension 2.4.0 (released) | `openId.jwtIssuers` | `iss` is not validated (`jwt_issuers` not rendered) | as Permission Service 0.0.53 |
| Envoy auth extension 2.4.1 (snapshot) | `openId.allowedJwtIssuers` (old name fails the render) | `iss` is not validated (`jwt_issuers` not rendered) | as Permission Service 0.0.54 |

The discovery document's issuer is **not** added implicitly in any version. Set the list explicitly: an unchecked issuer lets any tenant or IdP whose keys appear in the configured JWKS satisfy the signature check, and it matters most for multi-tenant discovery URLs. Entra's v2.0 issuer form is `https://login.microsoftonline.com/<tenant-id>/v2.0`; v1-format tokens carry `https://sts.windows.net/<tenant-id>/` (both listed as examples in the Permission Service `values.yaml`). List only the form your tokens actually carry — decode a sample token and read `iss`.

On the released charts a shared overlay cannot set both issuer lists with one key, because the Envoy key is `jwtIssuers`. On the snapshot charts both use `openId.allowedJwtIssuers`.

## Principal id and groups claims

| Setting | Where | Default | Notes |
|---|---|---|---|
| `openId.principalIdClaim` | Permission Service | `sub` | Entra: set `oid`. `sub` is pairwise per application, so it differs between the UI and a service for the same user. |
| `openId.principalIdClaim` | Envoy auth extension | `sub` | Value forwarded upstream in the `user-id` header (`config.server.userIdHeader`) |
| `principal.idClaim` | Permission Service metadata, per service | falls back to `openId.principalIdClaim` (`PRINCIPAL_ID_CLAIM`), then `sub` | Lets one service identify users by `email` while another uses `oid`; policies for each service must use that service's id form |
| `openId.principalGroupsClaim` | Permission Service | `groups` | JSON array of group ids; feeds `principal in Group::"<id>"`. The Envoy 2.4.0 chart also has this key but nothing reads it; 2.4.1 removed it. |
| `openId.principalIdClaim` | Event Aggregation / Consumer | `sub` | Must match the other services; see [../notifications.md](../notifications.md) |

With `oid` everywhere, the principal ids in Cedar policies equal the User Info Service's user `id` (the Entra object id). That keeps policy authoring, the Permission UI's user autocomplete, and Nucleus ACL migration consistent.

Entra emits a `groups` claim only when group claims are enabled in the API registration's *Token configuration*. Entra also limits the number of groups placed in a token ("group overage"); a user above the limit gets no usable `groups` claim and silently fails group-scoped policies. Test with your largest-membership accounts, and prefer assigning a small set of app-specific groups. See Microsoft's [group claims documentation](https://learn.microsoft.com/en-us/entra/identity/hybrid/connect/how-to-connect-fed-group-claims).

## Token verification type

`openId.tokenVerificationType` (both charts): `jwt` (default; local signature, `exp`, `aud` check after JWKS fetch, plus `nbf` in the snapshot versions), `opaque` (token accepted if the IdP `userinfo` endpoint returns 200; cached for `tokenTtl`, so a revoked token keeps working until expiry; no `aud` check), `jwtuserinfo` (JWT check plus `userinfo` claims merged). Use `jwt` with Entra. With `opaque`, keep `tokenTtl` no longer than the IdP token lifetime.

Permission Service 0.0.54 fixes two startup problems in the released 0.0.53, per its changelog: 0.0.53 refused the documented value `jwtuserinfo` (only `jwt-with-user-info` worked; 0.0.54 accepts both), and 0.0.53 required `JWT_AUDIENCE` even for `opaque` (0.0.54 requires it only for `jwt` and `jwtuserinfo`).

JWT clock skew: `config.jwtLeeway` (default 60 s, 0–300) on the Permission Service; on Envoy it is a chart value only in 2.4.1 (2.4.0 uses the built-in 60 s).

## Shared values overlays

Layer a shared `oidc-config.yaml` with per-chart files (the source skill package "Layered Values Pattern"). Which keys can be shared depends on the chart versions.

**Released charts (Envoy 2.4.0, Permission Service 0.0.53).** `openIdConfigurationUri` and `clientRegistrations` are portable across the Envoy, Permission Service, Permission UI, and Discovery charts:

```yaml
# oidc-config.yaml — no secrets in this file
openId:
  openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"
  clientRegistrations:
    - name: "default"
      clientId: "<browser-client-id>"
      scope: "openid profile email offline_access <api-client-id>/.default"
```

```yaml
# envoy-auth-extension 2.4.0: oidc-edge.yaml
openId:
  additionalJwtAudience: ["<api-client-id>"]
  jwtIssuers: ["https://login.microsoftonline.com/<tenant-id>/v2.0"]
  principalIdClaim: "oid"
---
# permission-service 0.0.53: oidc-permission.yaml
openId:
  enabled: true
  additionalJwtAudience: ["<api-client-id>"]
  allowedJwtIssuers: ["https://login.microsoftonline.com/<tenant-id>/v2.0"]
  principalIdClaim: "oid"
```

**Snapshot charts (Envoy 2.4.1, Permission Service 0.0.54).** The Envoy 2.4.1 chart fails to render if `openId.clientRegistrations` is present, so keep client registrations out of the shared file. The validators now share the same key names:

```yaml
# oidc-config.yaml — shared by every chart
openId:
  openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"
  principalIdClaim: "oid"
---
# oidc-validators.yaml — envoy-auth-extension 2.4.1 and permission-service 0.0.54
openId:
  allowedJwtAudience: ["<api-client-id>"]
  allowedJwtIssuers: ["https://login.microsoftonline.com/<tenant-id>/v2.0"]
  # requiredJwtClaims: [...]   # optional client allow-list, see Audience design
---
# oidc-clients.yaml — permission-ui and auto-discovery-service only
openId:
  clientRegistrations:
    - name: "default"
      clientId: "<browser-client-id>"
      scope: "openid profile email offline_access <api-client-id>/.default"
```

Mixed releases (for example the released Permission UI and Discovery with a snapshot Envoy build) follow the snapshot layout. Add `openId.enabled: true` for the Permission Service and the Discovery chart in their own files.

The Permission UI chart prefers a registration named `permission-ui`; add one if the UI uses its own public client. Navigator and Kit clients read the client registration from Discovery `auth-config` ([../discovery.md](../discovery.md), [../navigator.md](../navigator.md)).

## Service identities (outbound tokens)

| Caller | Chart block | Needs a token for | Least-privilege notes |
|---|---|---|---|
| Permission Service → Event Aggregation / Consumer | `serviceIdentity.clientCredentials` or `serviceIdentity.accessTokenFile` (exactly one) | `event-aggregation-service:publish-event`, `event-consumer-service:consume-durable-queues` | Grant only those actions to the Permission Service principal ([cedar-policies.md](cedar-policies.md)) |
| Storage Service → Permission Service, notifications | Storage chart service identity | publishing events; any Permission Service call that names a principal other than the token's subject also needs `permissions:view` | [../production-stack.md](../production-stack.md), [`storage-credentials.rst`](../../../../../storage/reference/storage-service/docs/helm/storage-credentials.rst) |
| User Info smoke test → User Info | `serviceIdentity.*` (smoke-test hook only) | an API-audience token, not a Graph token | Test identity; do not reuse in production |
| User Info writer/reader → Microsoft Graph | `providers.entra.*` | Graph application permissions | Separate registration; see [../userinfo.md](../userinfo.md) |

Prefer `accessTokenFile` with a workload-identity token provisioner sidecar over a long-lived client secret. The chart mounts an in-memory `emptyDir` (`token-exchange`, `medium: Memory`); in 0.0.53 it is mounted at `/etc/secrets/access-token/`, and in 0.0.54 at the directory of `serviceIdentity.accessTokenFile.path`. The file holds JSON with `access_token` and an RFC 3339 `expires_on`. With `clientCredentials`, the secret comes from `clientSecretRef` (mounted at `/etc/secrets/client-credentials/<key>`), never from values.

Service principals are principals like users. In Entra, a client-credentials token's `oid` is the service principal's object id, so policies name it as `Principal::"<service-principal-object-id>"` when `principalIdClaim: oid`. Examples in the upstream docs use `Principal::"permission-service"`; that literal works only if your IdP puts that string in the configured claim. A `requiredJwtClaims` client allow-list must include each service identity's client id, or its calls get `401`.

## Secrets reference

Create every Secret out of band (`kubectl create secret … --from-file` or your secret manager's CSI or operator integration). Never place values in Helm values, Git, or CI logs; Helm release history keeps rendered values.

| Secret (default name) | Keys | Used by | Notes |
|---|---|---|---|
| `ngcpull-secret` | registry credentials | all charts | [../artifact-access.md](../artifact-access.md) |
| `permission-service-secret` | `POSTGRES_PASSWORD` (unless `database.postgres.tokenFile.enabled`), plus `POSTGRES_USER` when `database.postgres.user` is empty | Permission Service | Mounted read-only at `/etc/secrets/`. 0.0.53: named by `secret.name`; set `secret.create: false`. 0.0.54: named by `database.postgres.secretRef.name`; the chart never creates it, so it must exist before install. |
| `<clientSecretRef.name>` | `<clientSecretRef.key>` | Permission Service outbound client credentials | Prefer `accessTokenFile` |
| `microsoft-graph-userinfo-service-az-credentials` | `client-secret` | User Info (`authMode: client-secret`) | Omit with `workload-identity` |
| `microsoft-graph-userinfo-service-pg-credentials` | `password` | User Info Postgres | Or `tokenFile` rotation |
| TLS secret (`httpProxy.tls.secretName`) | `tls.crt`, `tls.key` | Contour `HTTPProxy` per chart | Or `httpProxy.tls.certManager` |

Rotate client secrets by creating a new Entra credential, updating the Secret, restarting the consuming pods, and only then revoking the old credential. Database credentials supplied through `tokenFile` are picked up without a restart (the Permission Service rebuilds its pool on change; User Info uses the new credential for new connections). A changed `permission-service-secret` is not tracked by the 0.0.54 chart; restart the Deployment after changing it.

## Checks

```bash
# the discovery document is reachable from the cluster (Permission Service exits if not)
kubectl run oidc-check --rm -i --restart=Never -n <namespace> --image=curlimages/curl -- \
  -sf "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"

# clients can bootstrap (route is intentionally unauthenticated)
curl -s https://<discovery-host>/api/v1/auth-config

# an unauthenticated call is refused at the edge
curl -s -o /dev/null -w '%{http_code}\n' https://<discovery-host>/api/v1/services   # expect 401
```

To diagnose a `401` from a valid-looking token, decode it locally (never paste a live token into a website) and compare `aud`, `iss`, `exp`, `nbf`, the principal claim, and any `requiredJwtClaims` claim with the lists above. On the snapshot versions, also confirm the token carries `aud` (and `iss` when issuers are listed) at all. The Envoy 2.4.0 extension logs `authorization failed: … for user <hint>` (the hint is read from the unverified token); 2.4.1 logs `authorization failed: <error>` only, and `authorization failed: token exceeds max_token_length` for oversized tokens. The Permission Service logs `Failed to initialize authentication` at startup when discovery fails, or, in 0.0.54, when no audience is configured for `jwt`/`jwtuserinfo`.
