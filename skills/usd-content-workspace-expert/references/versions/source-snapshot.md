# Source-snapshot versions in this repository

> **Applies to:** every component in this repository · **Captured from:** repository files at the commit you have checked out (values below were re-read on 2026-10-16 at `189896e`; the Storage Core row was last checked at `367f7ed`) · **Sources:** component `CHANGELOG.md`, `Cargo.toml`, `pyproject.toml`, `package.json`, git history

Components arrive as snapshot pull requests titled `Release <component> <version>`. The values below go stale with every snapshot; re-read them with the commands at the end before relying on them.

## Where each version lives

Most `Chart.yaml` files in the snapshots carry `version: "0.0.0"` and `appVersion: "0.0.0"`. That is a placeholder the release pipeline replaces when it publishes the chart; it is not the component version. Read the sources in the table instead.

| Component | Snapshot version | Read from |
| --- | --- | --- |
| Storage Service | 1.2.3, plus `[Unreleased]` changes | [`Cargo.toml`](../../../../storage/reference/storage-service/Cargo.toml) `[workspace.package]`, [`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md) |
| Storage API (protos, OpenAPI, conformance, filesystem example) | unreleased, after 1.0.0-beta.4; read the exact build from `pyproject.toml` | [`CHANGELOG.md`](../../../../storage/api/CHANGELOG.md), OpenAPI `info.version`, [`filesystem_example/pyproject.toml`](../../../../storage/api/filesystem_example/pyproject.toml) |
| Storage Core crates | pre-1.0 alpha libraries with no release of their own; `Cargo.toml` keeps the versions Storage Service 1.2.3 shipped with, and the code has unreleased changes | [`storage-core/CHANGELOG.md`](../../../../storage/reference/storage-core/storage-core/CHANGELOG.md), [`storage-core-protos/CHANGELOG.md`](../../../../storage/reference/storage-core/storage-core-protos/CHANGELOG.md) |
| Metadata API | 0.3.0 (OpenAPI reports `metadata-protos-0.1.0-14-g41e6e34`) | [`CHANGELOG.md`](../../../../metadata/api/CHANGELOG.md) |
| Auto Discovery operator | 1.2.0, unreleased | [`Cargo.toml`](../../../../discovery/reference/discovery-service/operator/Cargo.toml), release commit |
| Legacy discovery chart | 2.4.1, unreleased | [`VERSION.toml`](../../../../discovery/reference/discovery-service/chart/VERSION.toml), [`CHANGELOG.md`](../../../../discovery/reference/discovery-service/chart/CHANGELOG.md) |
| Discovery protos | untagged main snapshot | [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md) |
| Event Aggregation Service | 1.7.0 | [`pyproject.toml`](../../../../notifications/reference/event-aggregation-service/pyproject.toml), chart `version: 1.7.0` |
| Event Consumer Service | 1.12.0 | [`pyproject.toml`](../../../../notifications/reference/event-consumer-service/pyproject.toml), chart `version: "1.12.0"` |
| notification-common | 7.0.0 (both services pin `notification-common==7.0.0`, build against the in-tree path source, and lock 7.0.0; see [../components/notification-common.md](../components/notification-common.md)) | [`pyproject.toml`](../../../../notifications/reference/common/pyproject.toml) |
| Notification protos | `notification-protos 0.0.0-manual.*` snapshots | release commits |
| Permission Service | 0.0.54 (`openapi.json` `info.version` still says 0.0.53) | [`Cargo.toml`](../../../../permissions/reference/permission-service/Cargo.toml), [`CHANGELOG.md`](../../../../permissions/reference/permission-service/CHANGELOG.md), `openapi.json` `info.version` |
| Envoy auth extension | 2.4.1 | [`Cargo.toml`](../../../../permissions/reference/envoy-auth-extension/Cargo.toml), [`CHANGELOG.md`](../../../../permissions/reference/envoy-auth-extension/CHANGELOG.md) |
| Permission UI | 0.0.22 changelog; `package.json` 0.0.23 | [`CHANGELOG.md`](../../../../permissions/reference/permission-ui/CHANGELOG.md), [`package.json`](../../../../permissions/reference/permission-ui/package.json) |
| Storage permission UI | 0.0.24 | [`package.json`](../../../../permissions/reference/storage-permission-ui/package.json) |
| Microsoft Graph User Info Service | 0.0.9 | [`Cargo.toml`](../../../../user-identity/reference/microsoft-graph-userinfo-service/Cargo.toml), [`CHANGELOG.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/CHANGELOG.md) |
| Thumbnail Service | 0.1.0 | [`Cargo.toml`](../../../../thumbnails/reference/thumbnail-service/Cargo.toml), `thumbnail.openapi.yaml` `info.version` |
| Storage Navigator UI | 1.0.4 (`package.json`); the changelog's `# 1.0.5` section is unreleased and also adds entries under the released `# 1.0.4` heading | [`package.json`](../../../../examples/navigator-ui/package.json), [`CHANGELOG.md`](../../../../examples/navigator-ui/CHANGELOG.md) |
| Observability libraries | 0.14.0 | [`Cargo.toml`](../../../../common/observability/Cargo.toml) |
| URL normalizer | 0.1.0 | [`Cargo.toml`](../../../../common/url-normalizer/Cargo.toml) |

Changelogs can lag the package version. The notification services are 1.7.0 and 1.12.0 by `pyproject.toml`, chart, and release commit, while their changelogs still top out at 1.6.0 and 1.11.0 with the newer changes under `[Unreleased]`.

`[Unreleased]` changelog sections describe changes in this snapshot that are not in any published release. Treat them as source behavior only. For example, the Storage Service snapshot enables URL normalization by default (`config.urlNormalizer.enabled`), which changes which object some addresses resolve to; released Storage Service 1.2.3 does not include that change.

[`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md) records each component's declared version, source tag and source commit for the current snapshot, and [`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md) summarizes import scope and CI build coverage.

## Re-read the versions

Run from the repository root:

```bash
# Release history: one line per snapshot pull request
git log --format='%h %ad %s' --date=short | grep -i 'release '

# Rust workspace versions
for f in $(git ls-files '*Cargo.toml'); do
  awk -v f="$f" '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p&&/^version/{print f": "$3}' "$f"
done

# Python and Node packages
git ls-files '*pyproject.toml' | xargs grep -H -m1 '^version'
for f in examples/*/package.json permissions/reference/*/package.json; do
  echo "$f: $(jq -r .version "$f")"
done

# Top changelog entry per component
git ls-files '*CHANGELOG.md' | grep -v externals | while read f; do
  echo "$f: $(grep -m1 -E '^##? \[?[0-9U]' "$f")"
done
```
