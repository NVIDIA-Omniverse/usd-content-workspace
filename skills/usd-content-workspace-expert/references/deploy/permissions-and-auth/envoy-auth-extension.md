# Deploy the Envoy auth extension

> **Applies to:** Envoy auth extension chart and image · **Released:** chart `envoy-auth-extension` 2.4.0, image `nvcr.io/nvidia/omniverse/envoy-auth-ext:2.4.0` (usd-content-workspace Developer Guide 1.3.0) · **This repo's snapshot:** 2.4.1 (`Cargo.toml`, `CHANGELOG.md` top `[2.4.1]`) · **Sources:** `permissions/reference/envoy-auth-extension/{README.md,CHANGELOG.md,helm/Chart.yaml,helm/values.yaml,helm/templates/*}`, `crates/envoy-auth-ext/src/handlers/grpc.rs`, `vendor/permission-auth/src/lib.rs`, Storage/Discovery/Permission/User Info chart `httpProxy.authExtension` values, the [source skill package](../../../NOTICE.md) production stack "Ingress → Auth Mode" and "Authentication"

Apache-2.0 component ([`LICENSE`](../../../../../permissions/reference/envoy-auth-extension/LICENSE)); use the released NGC image unless you need to build it yourself.

**Two chart versions, two values layouts.** NGC publishes 2.4.0. This repo's source is 2.4.1, which renames and removes `openId` keys, and a 2.4.0 values file fails to render against it. Each settings block below says which version it is for. Check the version of the chart you pulled (`helm show chart ./envoy-auth-extension`) before writing values.

## What you get

Contour sends every request on an auth-enabled `HTTPProxy` to this service's `Check` RPC. A valid bearer token is let through with `user-id`, `username`, and `username-source-claim` headers set (any client-supplied values are overwritten); anything else gets `401`, or `403` when the IdP refuses the token. The service does **not** authorize — upstream services still call the Permission Service.

Prerequisites: Contour installed with its CRDs (the chart creates a `projectcontour.io/v1alpha1` `ExtensionService`; see [../production-stack.md](../production-stack.md)), DNS and TLS for the protected hosts, an OIDC provider, the `ngcpull-secret`, and the audience/issuer decisions in [oidc-and-token-audiences.md](oidc-and-token-audiences.md). The 2.4.1 chart also requires Kubernetes 1.27 or later (`Chart.yaml` `kubeVersion: ">=1.27.0-0"`; Helm refuses older clusters) because its probes use the gRPC handler.

## Install

Pull the chart (artifact access and credentials: [../artifact-access.md](../artifact-access.md)):

```bash
# NGC_API_KEY is supplied by your credential mechanism; do not echo it
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/envoy-auth-extension-2.4.0.tgz \
  --username='$oauthtoken' --password="${NGC_API_KEY}"
tar -xzf envoy-auth-extension-2.4.0.tgz   # unpacks to ./envoy-auth-extension (chart name)
```

`envoy-values.yaml`:

```yaml
image:
  tag: "2.4.0"              # values.yaml defaults to the placeholder "0.0.0"
  pullSecrets:
    - name: ngcpull-secret

replicaCount: 2              # it is in the path of every protected request

resources:
  requests: { cpu: 50m, memory: 64Mi }
  limits:   { cpu: 500m, memory: 128Mi }

# 2.4.0 chart:
config:
  otel:
    endpoint: null           # set to your OTLP collector to export traces/metrics

# 2.4.1 chart (replaces config.otel.endpoint; exactly one endpoint per signal when enabled):
# observability:
#   logging:
#     level: "info"
#     structured: { enabled: true }
#   tracing: { enabled: true, endpoint: { grpc: "<otlp-collector:4317>" } }
#   metrics: { enabled: true, endpoint: { grpc: "<otlp-collector:4317>" } }
```

Then layer the shared and edge OIDC overlays from [oidc-and-token-audiences.md](oidc-and-token-audiences.md#shared-values-overlays) for your chart version:

```bash
helm upgrade --install envoy-auth-extension ./envoy-auth-extension \
  -f envoy-values.yaml -f oidc-config.yaml -f oidc-edge.yaml \
  --namespace <namespace>
kubectl get pods,svc,extensionservice -n <namespace>
```

The `ExtensionService` is named `auth-extension` (`extensionService.name`) in the release namespace and points at the Service port 50051 over `h2c`.

## Protect services with it

Each service chart enables it on its own `HTTPProxy`. The key names are identical in the Storage, Auto Discovery, Permission, and User Info charts:

```yaml
httpProxy:
  enabled: true
  fqdn:
    domain: "<dns-domain>"
  tls:
    enabled: true
    secretName: "<tls-secret>"
  authExtension:
    enabled: true
    responseTimeout: 2s
    extensionRef:
      name: auth-extension
      namespace: ""          # empty = release namespace; set it if the extension lives elsewhere
```

Behaviour per chart:

| Chart | What is protected | Exempt |
|---|---|---|
| Auto Discovery | all routes | `/api/v1/auth-config` and `/api/v2/auth-config` (`authPolicy.disabled: true`), and the optional root redirect, so clients can bootstrap |
| Storage Service | REST and gRPC virtual hosts | see [../production-stack.md](../production-stack.md) |
| Permission Service | REST (`permissions.<domain>`) and gRPC (`grpc.permissions.<domain>`) | none at the edge; `/health` is public in-service |
| User Info | REST (`userinfo.<domain>`) and gRPC (`grpc.userinfo.<domain>`) | none; **required**, because the service does not validate tokens itself |
| Permission UI | Do not enable; it is a static SPA that signs users in itself | — |

Contour applies the extension per virtual host. Every external host that serves data must enable it; a single host without it is an unauthenticated path into the stack. In-cluster traffic to a ClusterIP Service never passes through Envoy, so pair the edge with NetworkPolicies that admit only the Contour Envoy pods and the intended in-cluster callers.

The extension does not rate-limit. To bound checks per Envoy pod, add a Contour local rate limit policy on the `HTTPProxy`; Contour runs it before external authorization, whereas its global rate limit runs after and does not reduce checks ([`README.md`](../../../../../permissions/reference/envoy-auth-extension/README.md) "Rate Limiting").

## Settings reference

Token-validation keys differ by version. Defaults are from each chart's `values.yaml`.

| Setting | 2.4.0 (released) | 2.4.1 (snapshot) | Effect |
|---|---|---|---|
| Verification type | `openId.tokenVerificationType` `jwt` | same | `jwt` \| `opaque` \| `jwtuserinfo` |
| Discovery URL | `openId.openIdConfigurationUri` `"https://..."` | same, default `""` | IdP discovery document; startup exits if it cannot be loaded |
| Accepted `aud` | `openId.clientRegistrations[*].clientId` (placeholder `some_client_id`) + `openId.additionalJwtAudience` | `openId.allowedJwtAudience` (placeholder `["some_client_id"]`); old keys fail the render | `aud` must match one entry. In 2.4.1 a token without `aud` is rejected, and an empty list rejects every token. |
| Accepted `iss` | `openId.jwtIssuers` `[]` | `openId.allowedJwtIssuers` `[]`; old key fails the render | Empty = `iss` not validated in both. When set, 2.4.1 also rejects tokens that omit `iss`. |
| Extra claim rules | — | `openId.requiredJwtClaims` `[]` | All rules must pass, else `401`; see the [README "Required Claims"](../../../../../permissions/reference/envoy-auth-extension/README.md) |
| Principal id | `openId.principalIdClaim` `sub` | same | Forwarded as `user-id` |
| Groups claim | `openId.principalGroupsClaim` `groups` (not forwarded) | removed; ignored if set | — |
| Token size cap | `openId.maxTokenLength` 8192 | same; enforced in the `Check` handler before verification | Larger tokens get `401` |
| JWT clock skew | not in chart (built-in 60 s) | `config.jwtLeeway` 60 (0–300; render fails outside) | Applied to `exp`; 2.4.1 also enforces `nbf` |
| IdP timeouts | not in chart | `config.idpConnectTimeout` 5, `idpRequestTimeout` 15, `idpPoolIdleTimeout` 90 (s) | Bounds discovery, JWKS, and `userinfo` calls |
| `openId.enabled` | present, read by no template | removed; ignored if set | Cannot disable authentication in either version |
| Logging | not in chart; the 2.4.0 pre-release source previously imported hard-codes the filter `info,envoy_auth_ext=debug,client=trace` | `observability.logging.level` `info`, `observability.logging.structured.enabled` `false` | — |
| Telemetry | `config.otel.endpoint` `null` | `observability.tracing.*`, `observability.metrics.*` (off) | OTLP export |
| Probes | `tcpSocket` on `envoy-auth-ext` | gRPC health on `service.ports.envoyAuthExt`; values hold timings only | Do not add a handler to the probe values in 2.4.1 |

Unchanged in both: `config.server.usernameClaims` `["preferred_username"]` (first present claim forwarded as `username`); header names `user-id` / `username` / `username-source-claim`; `config.tokenTtl` 1200 s (capped by token `exp` for JWTs); `config.tokenCacheSize` 2048; `config.invalidTokenCacheSize` / `invalidTokenCacheTtl` 256 / 60 s (TTL clamped to `tokenTtl`); `config.jwksTtl` / `jwksFailureBackoff` 10800 s / 10 s; `extensionService.name` `auth-extension`; `autoscaling.enabled` `false` (`minReplicas: 2`, `maxReplicas: 5`, CPU 60%). `service.ports.proxy` and `service.tls` exist in `values.yaml` but no template reads them.

To take a route out of authentication, remove `authExtension` from that `HTTPProxy`; no extension setting turns validation off (do not do that on a shared cluster).

## Upgrading

From 2.4.0 to 2.4.1 (when it is published), per `CHANGELOG.md` `[2.4.1]`:

1. Replace `openId.clientRegistrations` and `openId.additionalJwtAudience` with one `openId.allowedJwtAudience` list. Leaving either old key makes the install fail, which also means a shared overlay that carries `clientRegistrations` for the UI and Discovery can no longer be passed to this chart ([oidc-and-token-audiences.md](oidc-and-token-audiences.md#shared-values-overlays)).
2. Rename `openId.jwtIssuers` to `openId.allowedJwtIssuers` (the old name fails the render).
3. Delete `openId.enabled` and `openId.principalGroupsClaim` (ignored if left).
4. Move `config.otel.endpoint` to `observability.tracing` / `observability.metrics` (the old key is ignored, so export silently stops).
5. Strip `tcpSocket`/`httpGet` from any `livenessProbe`/`readinessProbe` override.
6. Confirm every IdP token carries `aud` (and `iss`, if you list issuers) and that clocks are within `config.jwtLeeway`; tokens that 2.4.0 accepted can now get `401`.
7. Confirm the cluster is Kubernetes 1.27 or later.

From 2.3.3 to 2.4.0: 2.4.0 renamed or added keys (issuers, groups claim, maximum token length, negative cache, JWKS backoff) and fixed `token_cache_size` being ignored. Diff your values against the 2.4.0 `values.yaml`; read [`CHANGELOG.md`](../../../../../permissions/reference/envoy-auth-extension/CHANGELOG.md) and [../../operate/upgrades.md](../../operate/upgrades.md).

## Security notes

- Upstream services must trust `user-id`/`username` only on traffic that arrived through an auth-enabled Envoy route. The extension overwrites those headers on authenticated requests; a request that reaches a pod directly can carry any header.
- The Envoy → extension hop is cleartext `h2c` inside the cluster and carries the bearer token. Keep the extension's Service ClusterIP-only and restrict it with a NetworkPolicy that admits only the Contour Envoy pods.
- **2.4.0 logs credentials at its default filter.** Per the 2.4.1 changelog, 2.4.0 records the `Check` request, including bearer tokens and cookies, in logs and traces, and logs a principal hint read from the **unverified** token on failed authentication. Treat 2.4.0 extension logs and traces as containing secrets: restrict who can read them, keep trace export off or to a trusted collector, and set retention accordingly. 2.4.1 stops recording the request, drops the hint, and defaults to `info`.
- In 2.4.1 keep `observability.logging.level` at `info` or above outside short troubleshooting windows.
- `responseTimeout` (default `2s`) bounds the edge wait; in 2.4.1 `config.idpRequestTimeout` also bounds each IdP call. JWKS caching and the negative cache keep IdP load bounded under bad-token floods.
- Set `allowedJwtIssuers` (2.4.1) or `jwtIssuers` (2.4.0); see [oidc-and-token-audiences.md](oidc-and-token-audiences.md#issuers).

## Validate

```bash
# extension is serving gRPC health (from inside the cluster)
kubectl run grpc-health --rm -i --restart=Never -n <namespace> --image=fullstorydev/grpcurl -- \
  -plaintext envoy-auth-extension-service.<namespace>.svc.cluster.local:50051 grpc.health.v1.Health/Check

# edge refuses anonymous requests and accepts a valid token
curl -s -o /dev/null -w '%{http_code}\n' https://<protected-host>/<path>                       # 401, server: envoy
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer ${TOKEN}" https://<protected-host>/<path>
```

The Service is named `<chart name>-service` (`envoy-auth-extension-service`) unless `service.nameOverride` is set. A `401` with a token you believe is valid is almost always `aud`, `iss`, `nbf`/`exp` clock skew, or a failed `requiredJwtClaims` rule; see [oidc-and-token-audiences.md](oidc-and-token-audiences.md#checks). For a broader triage path see [../../operate/runbooks.md](../../operate/runbooks.md).
