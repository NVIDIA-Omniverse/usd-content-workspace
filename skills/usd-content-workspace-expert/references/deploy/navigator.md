# Deploy Storage Navigator

> **Applies to:** Storage Navigator web UI (`storage-navigator` chart/image) · **Released:** `storage-navigator` chart and image 1.0.4 (Developer Guide 1.3.0) · **This repo's snapshot:** UI 1.0.4 in [`package.json`](../../../../examples/navigator-ui/package.json), with later source and an unreleased `# 1.0.5` section in [`CHANGELOG.md`](../../../../examples/navigator-ui/CHANGELOG.md) (later publishes are `0.0.0-manual` commits) · **Sources:** the [source skill package](../../NOTICE.md); [`examples/navigator-ui/`](../../../../examples/navigator-ui/README.md) chart and `src/services/`

Navigator is a client application of Discovery and the Storage REST API. It defines no service API of its own. Component map: [../components/navigator.md](../components/navigator.md).

## Storage Navigator UI

A React single-page app served by nginx. It reads a Discovery URL, fetches `auth-config`, signs the user in with OpenID Connect (authorization-code flow with PKCE, redirect to `/openid`, refresh-token grant; [`auth.ts`](../../../../examples/navigator-ui/src/services/auth.ts)), resolves the Storage REST URL from the catalog, and calls Storage REST directly from the browser. 1.0.4 probes Discovery v2 (`/api/v2/services`) first and falls back to v1 (`/api/v1/services`) on any v2 failure, logging the reason.

The snapshot source adds two Discovery-driven behaviors that the released 1.0.4 import did not have; confirm on your deployment whether its image has them ([component map](../components/navigator.md#released-104-versus-this-snapshot)):

- **Metadata:** it uses the Storage `metadata` endpoint at `v1beta` when Discovery v2 advertises it, and `v1alpha` otherwise.
- **ZIP download:** when Discovery advertises an `archive` endpoint (`v1alpha`) on the same site as the Navigator origin, downloads go through that archive service as one ZIP. Otherwise Navigator uses the browser File System Access flow (Chromium-based browsers). This repository ships no archive service.

### Prerequisites

- Storage Service and Discovery running, with ingress and TLS ([production-stack.md](production-stack.md)). Navigator is a browser app; every URL in the external catalog must be reachable from users' browsers.
- An OIDC client registration that allows the redirect URI `https://navigator.<domain>/openid`. Navigator looks up the client named `navigator` in `auth-config`, then `default` (the operator chart copies `default` to `navigator` when no `navigator` registration exists; [discovery.md](discovery.md#authentication-bootstrap)).
- Backend CORS for the Navigator origin when objects are read or written through redirect URLs ([production-stack/storage-service.md](production-stack/storage-service.md#backend-permissions)), and Storage `httpProxy.corsPolicy` that admits the Navigator origin (the Storage default admits any subdomain of its `fqdn.domain`).
- Optional permission panel: the UI includes `@omniverse/storage-permission-ui` only if that package was installed and built at build time; otherwise a stub keeps the build working and the panel is absent. Released 1.0.4 declared version `0.0.24` from an `@omniverse` npm registry; the snapshot installs it from [`permissions/reference/storage-permission-ui`](../../../../permissions/reference/storage-permission-ui/README.md) (build its library with `npm run build:lib` first; see the UI README). Whether a given released image includes it is not recorded in this snapshot. The panel needs a Permission Service ([permissions-and-auth.md](permissions-and-auth.md)).
- Optional archive service (snapshot behavior): it must be advertised through Discovery and served on the same site as Navigator (same scheme and same registrable domain, approximated as the last two DNS labels), because its download relies on a `SameSite=Strict` cookie. A cross-site archive service is ignored with a console warning.

### Values

Snapshot chart: [`helm/storage-navigator/values.yaml`](../../../../examples/navigator-ui/helm/storage-navigator/values.yaml).

```yaml
image:
  repository: nvcr.io/nvidia/omniverse/storage-navigator
  tag: "1.0.4"
  pullSecrets:
    - name: ngcpull-secret
config:
  defaultDiscovery: "https://<domain>"   # auto-connect; hides "Add Connection"
  csp: ""                                # Content-Security-Policy; setting it also adds HSTS (max-age=3600)
  references: []                         # links shown next to the app menu: {label, url, icon}
service:
  port: 8080
httpProxy:
  enabled: true
  fqdn: { host: navigator, domain: "<domain>" }
  tls: { enabled: true, secretName: usd-content-workspace-cert }
```

`config.defaultDiscovery` accepts a hostname or full URL; a bare hostname gets `https://`. When it is empty, users add connections manually. Set `config.csp` for production; a policy that allows `connect-src` to Discovery, Storage, the IdP, and the backend redirect hosts is required, or the app breaks. With an archive service, also allow it in `connect-src` (the prepare call). The download itself is a form `POST` into a hidden iframe, so a policy that restricts `form-action` or `frame-src` must admit the archive host; test with a real download, because this snapshot documents no CSP for it. The default nginx route timeout is `response: 10s`, `idle: 3s`.

### Runtime configuration

The chart renders a ConfigMap with `nginx.conf` and `config.json`, mounted at `/usr/local/nginx/conf/nginx.conf` and `/var/www/html/config.json`. The app reads `config.json` on boot ([`appConfigService.ts`](../../../../examples/navigator-ui/src/services/appConfigService.ts)):

| Field | Default | Set by the chart |
| --- | --- | --- |
| `defaultDiscovery` | `""` | yes (`config.defaultDiscovery`) |
| `references` | `[]` (http(s) or root-relative URLs only) | yes (`config.references`) |
| `requestTimeout` | 30000 ms (max 300000) | no |
| `batchSize` | 25 (max 500) | no |

A missing `config.json` (404) yields defaults; invalid JSON or schema errors stop the app with an error. To change `requestTimeout` or `batchSize` with the snapshot chart you must override the mounted file (for example with your own ConfigMap in `volumes`/`volumeMounts`); the chart does not template them.

### Install

```bash
helm upgrade --install storage-navigator <chart> -n <ns> -f navigator-values.yaml -f ingress-values.yaml
kubectl -n <ns> rollout status deployment/storage-navigator
```

Add `navigator.<domain>` to the certificate ([production-stack/ingress-and-tls.md](production-stack/ingress-and-tls.md#certificates)).

### Validate

```bash
curl -fsS https://navigator.<domain>/ | grep -i '<title>'      # the Navigator shell, not an IdP page
curl -fsS https://navigator.<domain>/config.json | jq .
```

The liveness/readiness probes hit `/health`; the chart's `nginx.conf` has no `/health` location, so `try_files` returns `index.html` with 200. A passing probe only proves nginx serves files. Then sign in as a real user, browse the tree, upload a file, and confirm the Discovery route Navigator used (browser console logs the v2 fallback reason if it fell back to v1). If you deploy an archive service, download a folder and confirm you get one ZIP; a console warning that the archive service "is on a different origin than the app" means it failed the same-site check and Navigator fell back to the File System Access flow. Edit a tag on a file to exercise whichever metadata API version Discovery selected.

### Security notes

- The SPA holds the user's tokens in the browser; serve it only over TLS and set `config.csp`.
- The ServiceAccount has `automount: false`; keep it.
- `securityContext` and `podSecurityContext` are empty by default. The snapshot `Dockerfile` ends with `USER nvs` (`ARG SERVE_USER`, the user its default `public-nginx` serve stage creates, as simple-nginx does); the runtime user of the released 1.0.4 image is not recorded here. Test `runAsNonRoot: true` and `readOnlyRootFilesystem: true` before enabling them. If you build with another `SERVE_IMAGE`, set `SERVE_USER` to a non-root user that exists in it.
- Archive downloads (snapshot): the prepare call carries the bearer token, but the download `POST` carries only the `SameSite=Strict` cookie that the prepare response sets. Navigator's same-site check (last two DNS labels) only decides whether to try the archive path; it is not a security control. Cookie scope, lifetime, and per-item authorization belong to the archive service, which this repository does not define.
