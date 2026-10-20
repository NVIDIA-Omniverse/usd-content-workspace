# Storage Navigator — component map

> **Applies to:** Storage Navigator web UI · **Released:** `storage-navigator` chart and image 1.0.4 (Developer Guide 1.3.0) · **This repo's snapshot:** UI 1.0.4 in [`package.json`](../../../../examples/navigator-ui/package.json), with an unreleased `# 1.0.5` section at the top of [`CHANGELOG.md`](../../../../examples/navigator-ui/CHANGELOG.md); later publishes are commits named `Release storage-navigator 0.0.0-manual.<hash>` · **Sources:** [`examples/README.md`](../../../../examples/README.md) and the files below; no upstream component skill exists

Navigator is a reference client of Discovery, the Storage REST API, and (when advertised) an archive service; it defines no service API. Deployment and use: [../deploy/navigator.md](../deploy/navigator.md).

## Navigator UI — `examples/navigator-ui/`

| Need | Go to |
| --- | --- |
| Overview, scripts, building the optional permission panel from in-repo sources, k3d walkthrough | [`README.md`](../../../../examples/navigator-ui/README.md) |
| Service documentation (architecture, authentication, configuration, deployment, features, permissions, troubleshooting) | [`docs/app/docs/index.md`](../../../../examples/navigator-ui/docs/app/docs/index.md) |
| Source layout and conventions for contributors and agents | [`AGENTS.md`](../../../../examples/navigator-ui/AGENTS.md) |
| Changes per release | [`CHANGELOG.md`](../../../../examples/navigator-ui/CHANGELOG.md) |
| Helm chart | [`helm/storage-navigator/`](../../../../examples/navigator-ui/helm/storage-navigator/values.yaml) (`Chart.yaml` `0.0.0`) |
| Runtime `config.json` schema | [`src/services/appConfigService.ts`](../../../../examples/navigator-ui/src/services/appConfigService.ts) |
| Discovery v2 probe with v1 fallback; per-API version preference (`API_REGISTRY`) | [`src/services/discoveryService.ts`](../../../../examples/navigator-ui/src/services/discoveryService.ts), [`src/hooks/useServiceApi.ts`](../../../../examples/navigator-ui/src/hooks/useServiceApi.ts) |
| OIDC (authorization code + PKCE, `/openid` callback, client `navigator` then `default`) | [`src/services/auth.ts`](../../../../examples/navigator-ui/src/services/auth.ts), [`src/Router.tsx`](../../../../examples/navigator-ui/src/Router.tsx) |
| Storage REST calls | [`src/services/storage/`](../../../../examples/navigator-ui/src/services/storage/index.ts) |
| Metadata (tags): v1beta batch client with v1alpha fallback | [`src/services/storage/metadataService.ts`](../../../../examples/navigator-ui/src/services/storage/metadataService.ts) |
| ZIP download through an archive service | [`src/services/archiveService.ts`](../../../../examples/navigator-ui/src/services/archiveService.ts), [`src/hooks/useArchiveDownload.tsx`](../../../../examples/navigator-ui/src/hooks/useArchiveDownload.tsx), [`src/hooks/usePathContextMenu.tsx`](../../../../examples/navigator-ui/src/hooks/usePathContextMenu.tsx) |
| Notifications (SSE) and thumbnails | `src/services/notificationService.ts`, `src/services/thumbnailService.ts` |
| Optional permission panel shim | `src/stubs/` with the alias in [`vite.config.mjs`](../../../../examples/navigator-ui/vite.config.mjs); package source [`permissions/reference/storage-permission-ui/`](../../../../permissions/reference/storage-permission-ui/README.md) |
| Container | [`Dockerfile`](../../../../examples/navigator-ui/Dockerfile) |
| Third-party notices | [`ATTRIBUTIONS.md`](../../../../examples/navigator-ui/ATTRIBUTIONS.md) |

Facts: Vite dev server and chart Service on port 8080; chart probes `/health`; `config.json` keys `defaultDiscovery`, `references`, `requestTimeout`, `batchSize`; build `node:22-alpine`; the serve stage is `ARG SERVE_IMAGE`, default `public-nginx` (a stage on public `nginxinc/nginx-unprivileged:1.31.7-alpine-slim` laid out like simple-nginx; NVIDIA builds pass a simple-nginx image), and runs as `ARG SERVE_USER`, default `nvs` (any nginx image serving `/var/www/html` with the chart config works if you set both); `react-router-dom` `^7.18.4` (lock 7.18.4); tests with Vitest + MSW.

## Released 1.0.4 versus this snapshot

The `CHANGELOG.md` `# 1.0.4` section in this snapshot lists five entries that were added after the original pre-release import: v1beta metadata, archive ZIP download, `react-router-dom` 7.18.4, `simple-nginx` 1.1.7, and the service documentation. The code for the first two was not in that import. Whether the released 1.0.4 image contains them is not recorded here, so treat them as snapshot behavior until you confirm it on a deployment.

The `# 1.0.5` section is **unreleased**: `package.json` still says 1.0.4, the chart holds `0.0.0` placeholders, the released inventory has no 1.0.5, and the publishes since are `0.0.0-manual` commits. Its two fixes are in the snapshot source.

| Area | Released 1.0.4 (as originally imported) | This snapshot |
| --- | --- | --- |
| Metadata (tags) API | v1alpha only (`POST metadata/{uri}`, `PUT metadata/{uri}/tags` with `If-Match`) | Prefers the `metadata` endpoint at `v1beta` from Discovery v2, falling back to `v1alpha`; Discovery v1 always gets v1alpha. v1beta reads page through `POST metadata/query` and writes with `PUT metadata` carrying `expected_etag` in the entry. The batch route answers 200 with a per-entry `failed_precondition` on an ETag conflict, which the client turns into an error |
| Download | Browser File System Access API (`showDirectoryPicker`), file by file | ZIP archive through an archive service when Discovery advertises one on the same site as the app; otherwise the File System Access flow |
| Folder rename in the Path Tree | Pasted into the current folder | Stays in its parent folder (`getParentPath`); expanded nodes refresh (1.0.5) |
| File names containing `%` | "URI malformed" error when listing | Fixed (1.0.5; `getFileExtension` no longer calls `decodeURI`) |
| Optional permission panel | `@omniverse/storage-permission-ui` `0.0.24` from an `@omniverse` npm registry | `file:../../permissions/reference/storage-permission-ui`; `.npmrc` sets `install-links=true`. Build its library first (`npm run build:lib`) or the build substitutes the stub and warns |
| Serve image and user | Not recorded for the released image | `SERVE_IMAGE` default `public-nginx` (public nginx-unprivileged base, listed under the unreleased `# 1.0.5`), `USER nvs` |

### Archive service contract (client side only)

This repository ships no archive service. What Navigator expects, from [`useServiceApi.ts`](../../../../examples/navigator-ui/src/hooks/useServiceApi.ts) and [`archiveService.ts`](../../../../examples/navigator-ui/src/services/archiveService.ts):

| Item | Value |
| --- | --- |
| Discovery v2 | A service whose REST `endpoints` include `name: "archive"` with a `v1alpha` version `path` |
| Discovery v1 fallback | A service with `type: "archive"`; Navigator appends `/v1alpha/` to its `rest` URL |
| Prepare | `POST archive/prepare` under the versioned URL, JSON `{items: [{type: "file"|"folder", address}], archive_name}`, bearer token, `credentials: "include"`; response `{download_action, expires_at}`, problem `detail`/`title` on error |
| Download | A hidden HTML form `POST` (`application/x-www-form-urlencoded`, fields `items[<i>][type]`, `items[<i>][address]`, `archive_name`) to the service base URL joined with `download_action`, targeted at a hidden iframe |
| Cookie | The prepare response is expected to set a `SameSite=Strict` cookie that authorizes the form POST; no bearer token is sent with the download |
| Same-site check | Same scheme, and the same host, both hosts localhost or IP literals, or the same last two DNS labels. Multi-part public suffixes such as `co.uk` are a documented limitation. Cross-site: archive download is disabled, a warning is logged, and the File System Access flow is used |

The service side (validation, cookie lifetime, size limits, authorization of each item) is not defined in this repository.

## Known documentation defects in this snapshot

- UI README: the `docker build -t ...` command omits the build context (`.`); lists a `prettier:check` script that is `prettier` in `package.json`; says Node.JS 22+ while `.nvmrc` is 20.17.0 and the Dockerfile uses Node 20.
- UI `AGENTS.md`: says Mantine 7 and Allotment; `package.json` uses Mantine 9 and the 1.0.3 changelog replaced Allotment with Mantine's Splitter.
- UI chart: probes `/health`, but the chart's `nginx.conf` has no `/health` location; requests fall through to `index.html` (200). `requestTimeout` and `batchSize` cannot be set through chart values. The new `docs/app/docs/deployment.md` repeats the `/health` claim ("the `/health` endpoint").
- `CHANGELOG.md` `# 1.0.4` says "Update simple-nginx to 1.1.7"; the snapshot `Dockerfile` names no simple-nginx tag and defaults `SERVE_IMAGE` to its public `public-nginx` stage.
- `CHANGELOG.md` `# 0.0.1` names a non-public registry.
- `docs/app/docs/features.md` and `architecture.md` do not mention the archive download or the archive service.
- The `Dockerfile` build stage copies only this folder, so it cannot see `../../permissions/reference/storage-permission-ui` or the `.npmrc`; an image built from it gets the permission-panel stub. Whether `npm install` tolerates the missing optional `file:` dependency was not verified (no build was run).
