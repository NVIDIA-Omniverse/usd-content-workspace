# Envoy auth extension — component map

> **Applies to:** Envoy auth extension (`envoy-auth-ext`) · **Released:** chart `envoy-auth-extension` 2.4.0, image `envoy-auth-ext` 2.4.0 (usd-content-workspace Developer Guide 1.3.0 / NGC) · **This repo's snapshot:** 2.4.1 (`Cargo.toml` `[workspace.package] version`, top `CHANGELOG.md` entry `[2.4.1]`); `Chart.yaml` is the `0.0.0` placeholder and declares `kubeVersion: ">=1.27.0-0"` · **Sources:** `permissions/reference/envoy-auth-extension/` (README, CHANGELOG, helm, docker, crates, vendor, LICENSE), the [source skill package](../../NOTICE.md)

**Licence: Apache-2.0** ([`LICENSE`](../../../../permissions/reference/envoy-auth-extension/LICENSE); `Cargo.toml` `license = "Apache-2.0"`). The 2.4.1 snapshot builds from vendored pre-release copies of `permission-auth` and `permission-core` 0.0.54, `otel-tools`, and `tonic-observability` under [`vendor/`](../../../../permissions/reference/envoy-auth-extension/vendor/); the vendored `permission-auth` source matches the Permission Service's [`crates/permission-auth/`](../../../../permissions/reference/permission-service/crates/permission-auth/) at this snapshot (only `Cargo.toml` and added README/LICENSE files differ).

## What it is

A gRPC service implementing Envoy's external authorization API (`envoy.service.auth.v3.Authorization/Check`). The chart registers it with Contour as an `ExtensionService` (`projectcontour.io/v1alpha1`, protocol `h2c`). Any Contour `HTTPProxy` whose chart sets `httpProxy.authExtension.enabled: true` sends each request to `Check` before proxying it.

It **authenticates only**: it validates the bearer token (`jwt`, `opaque`, or `jwtuserinfo`) against the configured OpenID provider and returns OK, `401`, or `403`. It does not evaluate Cedar policies; authorization is the Permission Service's job, called by the upstream service. On success it sets three upstream headers (`user-id`, `username`, `username-source-claim` by default) with `OverwriteIfExistsOrAdd`, so client-supplied copies are replaced. The original `Authorization` header still reaches the upstream, which validates it again where it calls the Permission Service.

It is a separate Deployment, not a sidecar (the source skill package ports table calls it a "sidecar").

## Where things are

| Thing | Path in this repo |
|---|---|
| README (how it works, config file, required claims, rate limiting, build, local compose, production sizing draft) | [`README.md`](../../../../permissions/reference/envoy-auth-extension/README.md) |
| Changelog | [`CHANGELOG.md`](../../../../permissions/reference/envoy-auth-extension/CHANGELOG.md) |
| Helm chart | [`helm/`](../../../../permissions/reference/envoy-auth-extension/helm/) ([`values.yaml`](../../../../permissions/reference/envoy-auth-extension/helm/values.yaml), [`templates/configmap.yaml`](../../../../permissions/reference/envoy-auth-extension/helm/templates/configmap.yaml), [`templates/_helpers.tpl`](../../../../permissions/reference/envoy-auth-extension/helm/templates/_helpers.tpl), [`templates/extensionservice.yaml`](../../../../permissions/reference/envoy-auth-extension/helm/templates/extensionservice.yaml)) |
| Check handler | [`crates/envoy-auth-ext/src/handlers/grpc.rs`](../../../../permissions/reference/envoy-auth-extension/crates/envoy-auth-ext/src/handlers/grpc.rs) |
| Settings structs | [`crates/envoy-auth-ext/src/settings/`](../../../../permissions/reference/envoy-auth-extension/crates/envoy-auth-ext/src/settings/) |
| Token validation (vendored `permission-auth`) | [`vendor/permission-auth/`](../../../../permissions/reference/envoy-auth-extension/vendor/permission-auth/) (same source as [`permission-service/crates/permission-auth/`](../../../../permissions/reference/permission-service/crates/permission-auth/)) |
| Vendored Envoy protos | [`proto/vendor/envoy/service/auth/v3/`](../../../../permissions/reference/envoy-auth-extension/proto/vendor/envoy/service/auth/v3/) |
| Dockerfile, local Envoy compose stack, sample TOML | [`docker/`](../../../../permissions/reference/envoy-auth-extension/docker/) |
| Integration test harness | [`test/`](../../../../permissions/reference/envoy-auth-extension/test/) |
| gRPC health check helper | [`tools/check-grpc.sh`](../../../../permissions/reference/envoy-auth-extension/tools/check-grpc.sh) |
| Third-party attributions | [`attributions-2026-10-13.md`](../../../../permissions/reference/envoy-auth-extension/attributions-2026-10-13.md), [`license_links-2026-10-13.md`](../../../../permissions/reference/envoy-auth-extension/license_links-2026-10-13.md) |

There is no `docs/` tree, no OpenAPI, and no conformance suite; it is not a client-facing API and is not registered with Discovery.

Deployment guidance: [../deploy/permissions-and-auth/envoy-auth-extension.md](../deploy/permissions-and-auth/envoy-auth-extension.md). Token audience and issuer wiring shared with the other services: [../deploy/permissions-and-auth/oidc-and-token-audiences.md](../deploy/permissions-and-auth/oidc-and-token-audiences.md).

## Facts verified in the snapshot chart (2.4.1)

| Fact | Value |
|---|---|
| Service | `envoy-auth-extension-service` (`<chart>-service` unless `service.nameOverride`); Deployment `<fullname>-deployment` |
| Port | gRPC `service.ports.envoyAuthExt` 50051 (Service port name `envoy-auth-ext`). `service.ports.proxy` (8080) and `service.tls` are not used by any template. |
| ExtensionService name | `extensionService.name` default `auth-extension` — the default `httpProxy.authExtension.extensionRef.name` in the Storage, Discovery, Permission, and User Info charts |
| Probes | gRPC health (`grpc.port` = `service.ports.envoyAuthExt`) for liveness and readiness. `livenessProbe` / `readinessProbe` values carry timings only; the template adds the `grpc` handler. Needs Kubernetes 1.27 or later (`Chart.yaml` `kubeVersion`). |
| Config file | ConfigMap key `envoy-ext-config.toml` mounted at `/app/`; image runs `envoy-auth-ext -c /app/envoy-ext-config.toml` |
| Audience | `jwt_audience` = sorted, de-duplicated `openId.allowedJwtAudience`. Default is the placeholder `["some_client_id"]`; replace it. An empty list renders `jwt_audience = []`, and then every `jwt`/`jwtuserinfo` token is rejected. |
| `aud` claim | Required for `jwt` and `jwtuserinfo`: a token without `aud`, or with an `aud` that is not a string or list of strings, is rejected |
| Issuer | `openId.allowedJwtIssuers` empty ⇒ `jwt_issuers` is not rendered and `iss` is not validated. When set, a token without `iss` or with an unlisted `iss` is rejected. The discovery-document issuer is not added implicitly. |
| `nbf` | Enforced, with `config.jwtLeeway` (default 60 s, 0–300, render fails outside that range) applied to `exp` and `nbf` |
| Required claims | `openId.requiredJwtClaims` (default `[]`), rendered as `jwt_required_claims`. Operators `equals`, `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, `exists`; all rules must pass; invalid rules fail startup. Also applied to the `userinfo` claim set in `opaque` mode. |
| Removed keys fail the render | Setting `openId.clientRegistrations`, `openId.additionalJwtAudience`, or `openId.jwtIssuers` makes `helm template`/`install` fail with "`openId.<key>` was removed, use … instead" (`envoy-auth-extension.failOnRemovedOpenIdValues`). `openId.enabled` and `openId.principalGroupsClaim` were also removed but are **silently ignored** if set. |
| Token limits | `openId.maxTokenLength` 8192 bytes, checked in the `Check` handler before verification; `config.tokenTtl` 1200 s; `tokenCacheSize` 2048; negative cache `invalidTokenCacheSize` 256 / `invalidTokenCacheTtl` 60 s; `jwksTtl` 10800 s; `jwksFailureBackoff` 10 s |
| IdP client timeouts | `config.idpConnectTimeout` 5 s, `config.idpRequestTimeout` 15 s (discovery, JWKS, `userinfo`), `config.idpPoolIdleTimeout` 90 s |
| `openId.openIdConfigurationUri` | Default `""`; must be set. The service exits at startup if it cannot load the discovery document. |
| Image tag | `image.tag: "0.0.0"` (falls back to `latest` only when empty). Set it explicitly to the image you deploy. |
| Logging | `observability.logging.level` (default `info`) → `RUST_LOG`; `observability.logging.structured.enabled` (default `false`, pretty text) → `STRUCTURED_LOGGING`. The `Check` request (headers, bearer token, cookies) is not recorded in logs or spans (`#[tracing::instrument(skip_all)]`), and failed-authentication logs carry only the error. |
| Telemetry | `observability.tracing.*` and `observability.metrics.*` (each off by default; exactly one of `endpoint.grpc` / `endpoint.rest` when enabled), `observability.serviceName`, `observability.serviceScope`. `config.otel.endpoint` no longer exists. |
| Security context | `runAsNonRoot`, `runAsUser: 1000`, read-only root filesystem, all capabilities dropped; no ServiceAccount template |
| Autoscaling | `autoscaling.enabled: false` in values (`minReplicas: 2`, `maxReplicas: 5`, CPU 60%, memory 80% when enabled) |
| Image | Builder `rust:1.97` (`rust-toolchain.toml` 1.97); runtime `nvcr.io/nvidia/distroless/cc:v4.1.6`; build context is the component directory (`docker build -f docker/envoy-auth-ext.Dockerfile .`) |

## Version notes

- **Released 2.4.0 (NGC) versus snapshot 2.4.1.** The NGC chart and image are 2.4.0; 2.4.1 exists only as source in this repo until it is published. Read the `values.yaml` of the chart you pull. In 2.4.0 the audience comes from `openId.clientRegistrations[*].clientId` plus `openId.additionalJwtAudience` (placeholder `some_client_id`), issuers from `openId.jwtIssuers` (empty = not validated), probes are `tcpSocket`, telemetry is `config.otel.endpoint`, and `openId.enabled`/`principalGroupsClaim` are present but unused. As described in the 2.4.1 `CHANGELOG.md` security notes, 2.4.0 accepts JWTs that omit `aud`, or omit `iss` when an issuer list is set, does not check `nbf`, records the `Check` request (including bearer tokens and cookies) in debug-level logs and traces, and logs an unverified principal hint on failed authentication.
- **2.4.1 migration** (from `CHANGELOG.md` `[2.4.1]`): move every accepted audience to `openId.allowedJwtAudience`; rename `openId.jwtIssuers` to `openId.allowedJwtIssuers`; delete `openId.enabled` and `openId.principalGroupsClaim`; replace `config.otel.endpoint` with `observability.tracing`/`observability.metrics`; remove any probe handler (`tcpSocket`, `httpGet`) from `livenessProbe`/`readinessProbe` overrides; make sure the IdP puts `aud` in every token; run Kubernetes 1.27 or later.
- 2.4.0 (relative to 2.3.3) exposed JWT issuers, principal id/groups claims, maximum token length, negative-token cache, and JWKS failure backoff in the chart; fixed the cache key so `token_cache_size` is applied (it was ignored as `jwt_cache_size`); added liveness/readiness probes and a hardened security context; removed `log_user_claim` in favour of `principal_id_claim`. Values files written for 2.3.3 that set the old keys silently lose them.
- Confirm the image you run ([../versions/capture-a-deployment.md](../versions/capture-a-deployment.md)).

## Known defects in this snapshot

Report only.

- `docker/envoy-ext-config.toml` embeds a concrete Entra tenant id in `openid_configuration`; replace it with your own before any local use.
- The image still sets `ENV LOG_LEVEL="debug"`, which the service does not read; the level comes from `RUST_LOG` (set by the chart) or the config file's `[logging] level` (default `info`).
- `CHANGELOG.md` `[2.4.1]` says `permission-auth` and `permission-core` were updated to 0.0.54; the vendored crates are pre-release builds of 0.0.54, not the release.
- `templates/_helpers.tpl` defines `envoy-auth-extension.serviceAccountName`, which reads `serviceAccount.*` values that `values.yaml` does not define; no template calls it.
