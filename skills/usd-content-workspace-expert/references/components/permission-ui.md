# Permission UI and Storage Permission UI — component map

> **Applies to:** Permission Web UI (`permission-ui`) and the `@omniverse/storage-permission-ui` React library · **Released:** chart and image `permission-ui` 0.0.22 (usd-content-workspace Developer Guide 1.3.0 / NGC, unchanged from the previous profile); `storage-permission-ui` is not in the released NGC inventory (npm package) · **This repo's snapshot:** permission-ui `CHANGELOG.md` top `[0.0.22]`, `package.json` 0.0.23, source tag `0.0.22` (`SOURCE_VERSIONS.md`), `Chart.yaml` `0.0.0` placeholder; storage-permission-ui `package.json` 0.0.24 (tag `0.0.24`) · **Sources:** `permissions/reference/permission-ui/`, `permissions/reference/storage-permission-ui/`, `permissions/reference/permission-service/docs/service/docs/web-ui.md`, `examples/navigator-ui/package.json`, the [source skill package](../../NOTICE.md) production stack "Permission Web UI"

Both are Apache-2.0 ([permission-ui `LICENSE.txt`](../../../../permissions/reference/permission-ui/LICENSE.txt), [storage-permission-ui `LICENSE.txt`](../../../../permissions/reference/storage-permission-ui/LICENSE.txt)).

## Permission UI (standalone admin console)

A React single-page app served by nginx. It lists, creates, and deletes Cedar policies, edits service metadata, and runs diagnostics. Every screen is a call to the Permission Service REST API with the signed-in user's access token; there is no backend of its own. Browser sign-in uses `react-oidc-context` against the same OpenID provider the Permission Service trusts. The user needs the meta-permissions `permissions:view`, `permissions:edit`, `permissions:meta:read`, `permissions:meta:write`, and `permissions:diagnostics` (see [../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md)).

| Thing | Path |
|---|---|
| README (local dev on port 3180, runtime `main.json` fields) | [`README.md`](../../../../permissions/reference/permission-ui/README.md) |
| User and deploy guide | [`permission-service/docs/service/docs/web-ui.md`](../../../../permissions/reference/permission-service/docs/service/docs/web-ui.md) |
| Changelog | [`CHANGELOG.md`](../../../../permissions/reference/permission-ui/CHANGELOG.md) |
| Helm chart | [`helm/permission-ui/`](../../../../permissions/reference/permission-ui/helm/permission-ui/) ([`values.yaml`](../../../../permissions/reference/permission-ui/helm/permission-ui/values.yaml), [`templates/configmap.yaml`](../../../../permissions/reference/permission-ui/helm/permission-ui/templates/configmap.yaml), [`templates/_helpers.tpl`](../../../../permissions/reference/permission-ui/helm/permission-ui/templates/_helpers.tpl)) |
| Dockerfile (reference build; `SERVE_IMAGE` and `SERVE_USER` build args) | [`Dockerfile`](../../../../permissions/reference/permission-ui/Dockerfile), README "Container image" |
| REST calls | `src/state/policies.ts`, `src/state/metadata.ts`, `src/state/diagnostics.ts` |

Facts verified in the snapshot chart:

| Fact | Value |
|---|---|
| Port | `service.port` 8080 (nginx); probes `GET /` |
| Runtime config | ConfigMap `main.json` mounted at `/var/www/html/config/`, served at `/config/main.json`; nginx.conf mounted at `/usr/local/nginx/conf/nginx.conf` |
| OIDC client | first `openId.clientRegistrations` entry named `permission-ui`, else `default`; `authority` = `openId.openIdConfigurationUri` with `/.well-known/openid-configuration` trimmed |
| Redirect URI | `<http|https>://<httpProxy.fqdn.host>.<httpProxy.fqdn.domain><httpProxy.path>/openid` — register exactly this in the IdP |
| Backend URL | `config.endpoints.backend`; if empty and `httpProxy.enabled`, derived as `<scheme>permissions.<domain>` |
| User Info URL | **`userService.enabled`** + **`userService.endpoint`**; if the endpoint is empty and `httpProxy.enabled`, derived as `<scheme>userinfo.<domain>`. `config.endpoints.userService` (shown in `web-ui.md` and in the source skill package) is **not read** by this snapshot chart. |
| Security headers | `Strict-Transport-Security` and `Content-Security-Policy` are added only when `config.csp` is non-empty |
| Pod hardening | `podSecurityContext: {}`, `securityContext: {}`, `serviceAccount.automount: true`, `resources: {}` by default — set them |
| Image build | Node 22 build stage; serve stage `ARG SERVE_IMAGE=nvcr.io/nvidia/omniverse/simple-nginx:1.1.0`, then `USER ${SERVE_USER}` (default `nvs`, the non-root user of the default base). Another nginx image must serve `/var/www/html` with the chart's nginx config and define a non-root user passed as `SERVE_USER`. The Dockerfile is a reference for how the published image is built. |
| Discovery | Not registered; it is a browser app |

Deployment: [../deploy/permissions-and-auth/permission-ui.md](../deploy/permissions-and-auth/permission-ui.md).

## Storage Permission UI (embeddable panel)

`@omniverse/storage-permission-ui` is a React component library (`PermissionForm`) that shows and edits the permissions for one storage `object` or `folder`. The host app supplies the tokens (`auth.accessToken`, `auth.idToken`), `config.url` (Permission Service base URL) and optional `config.userInfoUrl`. [Navigator UI](../../../../examples/navigator-ui/README.md) pins it at `0.0.24` (`examples/navigator-ui/package.json`); Navigator deployment is covered in [../deploy/navigator.md](../deploy/navigator.md).

| Thing | Path |
|---|---|
| README (props, peer dependencies, integration snippet) | [`README.md`](../../../../permissions/reference/storage-permission-ui/README.md) |
| Contribution policy (published as part of usd-content-workspace; external code contributions not accepted; bugs and feature requests as repository issues naming `storage-permission-ui`; security reports to NVIDIA PSIRT) | [`CONTRIBUTING.md`](../../../../permissions/reference/storage-permission-ui/CONTRIBUTING.md) |
| Runnable example (`npm run dev`) | [`src/examples/Example.tsx`](../../../../permissions/reference/storage-permission-ui/src/examples/Example.tsx) |
| API layer | [`src/api/`](../../../../permissions/reference/storage-permission-ui/src/api/) |

Endpoints it calls (from `src/api/`): Permission Service `/v1beta/authorization/batch/`, `/v1beta/policies/`, `/v1beta/policies/batch/`, `/v1beta/policies/{id}`, `/v1beta/services/storage-service/`; User Info `/api/v1alpha/users/me`, `/users`, `/users/{id}`, `/users/{id}/groups`, `/groups`, `/groups/{id}`. The policy and metadata routes are Permission Service management APIs, not part of the published contract, so the panel works only against the reference Permission Service or an adapter that implements them (see [../develop/custom-permissions-adapter.md](../develop/custom-permissions-adapter.md)).

The signed-in user needs `permissions:view` and `permissions:edit` scoped to the resources they manage, and `permissions:meta:read` to load the `storage-service` metadata.

## Known defects in this snapshot

Report only.

- permission-ui `CHANGELOG.md` has two `[0.0.22]` headings; `package.json` is already 0.0.23.
- `web-ui.md` and the source skill package document `config.endpoints.userService`; the chart reads `userService.*`.
- storage-permission-ui README says installation needs an authenticated `.npmrc` "see `.npmrc.ci`"; that file is not in the snapshot.
