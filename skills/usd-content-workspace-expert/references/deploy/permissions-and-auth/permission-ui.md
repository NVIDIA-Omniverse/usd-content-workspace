# Deploy the Permission Web UI

> **Applies to:** Permission UI chart and image; embedding the Storage Permission UI panel · **Released:** chart and image `permission-ui` 0.0.22 (usd-content-workspace Developer Guide 1.3.0); `@omniverse/storage-permission-ui` not in the NGC inventory · **This repo's snapshot:** permission-ui 0.0.22 changelog / 0.0.23 `package.json`; storage-permission-ui 0.0.24 · **Sources:** `permissions/reference/permission-ui/helm/permission-ui/{values.yaml,templates/configmap.yaml,templates/_helpers.tpl}`, `permissions/reference/permission-ui/{README.md,Dockerfile}`, `permissions/reference/permission-service/docs/service/docs/web-ui.md`, `permissions/reference/storage-permission-ui/README.md`, the [source skill package](../../../NOTICE.md) production stack "Permission Web UI"

A static admin console for the Permission Service. Deploy it only after the Permission Service runs with authentication enabled and administrators hold meta-permissions ([cedar-policies.md](cedar-policies.md#administrators-meta-permissions)). It has no server-side logic; securing it means securing the browser OIDC client and the Permission Service API it calls.

## Identity provider setup

- Register a **public** client (single-page application platform on Entra; no client secret) for the UI, or reuse the stack's browser client.
- Redirect URI: exactly `https://<httpProxy.fqdn.host>.<httpProxy.fqdn.domain><httpProxy.path>/openid`, for example `https://web.permissions.<domain>/openid`. The chart derives it from `httpProxy` and uses `http://` when `httpProxy.tls.enabled` is false; do not register `http://` redirect URIs for production.
- Scopes: `openid email profile` plus `<api-client-id>/.default`, so the access token's `aud` is accepted by the Permission Service ([oidc-and-token-audiences.md](oidc-and-token-audiences.md#audience-design)).
- The chart picks the `clientRegistrations` entry named `permission-ui`, else `default`.

## Values

```yaml
image:
  pullSecrets:
    - name: ngcpull-secret

openId:
  openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"
  clientRegistrations:
    - name: "permission-ui"
      clientId: "<permission-ui-client-id>"
      scope: "openid email profile <api-client-id>/.default"

config:
  endpoints:
    backend: "https://permissions.<domain>"   # empty + httpProxy.enabled => https://permissions.<domain>
  csp: "default-src 'self'; connect-src 'self' https://permissions.<domain> https://userinfo.<domain> https://login.microsoftonline.com; frame-ancestors 'none'"

userService:                     # snapshot chart keys; not config.endpoints.userService
  enabled: true
  endpoint: "https://userinfo.<domain>"       # empty + httpProxy.enabled => https://userinfo.<domain>

httpProxy:
  enabled: true
  fqdn:
    host: "web.permissions"
    domain: "<domain>"
  tls:
    enabled: true
    secretName: "<tls-secret>"

serviceAccount:
  automount: false
podSecurityContext:
  runAsNonRoot: true
  seccompProfile: { type: RuntimeDefault }
securityContext:
  allowPrivilegeEscalation: false
  capabilities: { drop: ["ALL"] }
resources:
  requests: { cpu: 50m, memory: 64Mi }
  limits:   { cpu: 200m, memory: 128Mi }
```

Notes:

- The `csp` value is an example; tighten `connect-src` to the hosts your IdP and services actually use, and test sign-in after changing it. The nginx config adds `Content-Security-Policy` and `Strict-Transport-Security` **only** when `config.csp` is set; leaving it empty serves the console without either header.
- The pod-hardening values are not defaults in the snapshot chart (`podSecurityContext: {}`, `securityContext: {}`, `automount: true`, `resources: {}`). The snapshot `Dockerfile` now ends with `USER ${SERVE_USER}` (default `nvs`, from the default `simple-nginx` base). Kubernetes cannot check `runAsNonRoot: true` against a non-numeric image user and refuses to start such a container, so also set `podSecurityContext.runAsUser` to that user's numeric uid. The uid, whether the released 0.0.22 image sets a user, and whether nginx tolerates `readOnlyRootFilesystem` are not settled by the source here; inspect the image you pull (`docker image inspect … --format '{{.Config.User}}'`) and test before enforcing.
- The released 0.0.22 chart may differ from this snapshot on `userService`; the 0.0.22 changelog entry says it added "Helm `userService` section with enabled flag and default endpoint". Read the `values.yaml` in the chart you pulled.
- Narrow the Permission Service and User Info `httpProxy.corsPolicy.allowOrigin` to `https://web.permissions.<domain>` (their default is `*`).
- Do not put the UI's own `HTTPProxy` behind the Envoy auth extension; the SPA and `/config/main.json` must load before sign-in. The API hosts it calls must be behind it.

## Install and validate

```bash
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/permission-ui-0.0.22.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf permission-ui-0.0.22.tgz

helm upgrade --install permission-ui ./permission-ui -n <namespace> -f permission-ui-values.yaml
kubectl rollout status deploy/permission-ui -n <namespace>

# rendered runtime config: check authority, clientId, redirectUri, backend, userService
kubectl port-forward -n <namespace> svc/permission-ui 8080:8080 &
curl -s http://localhost:8080/config/main.json
```

`main.json` holds no secrets (client id, authority, redirect URI, scopes, URLs) and is public by design. Then sign in as an administrator and confirm Policies, Services, and Diagnostics load, and sign in as a non-administrator and confirm the API calls return `403`.

Failures and causes:

| Symptom | Likely cause |
|---|---|
| IdP error after sign-in (`redirect_uri` mismatch) | Registered redirect URI differs from the derived `…/openid` |
| Signed in, every list returns `401` | Access token `aud`/`iss` not accepted by the Permission Service or the Envoy edge; on the snapshot charts also a missing `aud`, or the UI's client id absent from a `requiredJwtClaims` allow-list |
| Signed in, `403` on Policies / Services / Diagnostics | User lacks `permissions:view` / `permissions:meta:read` / `permissions:diagnostics` |
| Users and Groups pages missing, principal is plain text | `userService.enabled` is false or the endpoint is empty |
| Browser blocks API calls | CORS `allowOrigin` on the API proxies, or `connect-src` in `config.csp` |
| Every write returns `501` | Permission Service runs in `config-file` mode |

Editing a policy in the UI creates a new policy and deletes the old one, so its numeric id changes; external references to policy ids break.

## Storage Permission UI panel

`@omniverse/storage-permission-ui` 0.0.24 is a React library, not a deployable service. [Navigator UI](../navigator.md) embeds it, installed in this snapshot from the in-repo source as a `file:` dependency rather than from a registry, to show "who can access this object or folder". The host app passes its own access token and id token, the Permission Service URL (`config.url`), and optionally the User Info URL (`config.userInfoUrl`). Same requirements as above: the access token must be accepted by both APIs, CORS must allow the host's origin, and the user needs `permissions:view` / `permissions:edit` on the subtree plus `permissions:meta:read`. It calls Permission Service policy and metadata management endpoints, so it does not work against a Permission API implementation that lacks them. Build and integration: [`storage-permission-ui/README.md`](../../../../../permissions/reference/storage-permission-ui/README.md).
