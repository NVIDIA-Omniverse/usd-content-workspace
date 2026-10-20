# Deploy Discovery

> **Applies to:** Auto Discovery operator (`auto-discovery-service`) and the legacy static Discovery chart (`discovery-service`) · **Released:** `auto-discovery-service` chart/image 1.1.1 (NGC; the Developer Guide 1.3.0 changelog says 1.1.0); legacy `discovery-service` chart 2.3.11 with image `simple-nginx` 1.1.0 (previous `published` profile) · **This repo's snapshot:** operator 1.2.0 ([`Cargo.toml`](../../../../discovery/reference/discovery-service/operator/Cargo.toml); unreleased; the most recent operator release commit adds capabilities v2beta with v2alpha fallback, the `/api/v2beta/services` route, and v2beta chart probes; no operator `CHANGELOG.md`); legacy chart 2.4.1, unreleased ([`VERSION.toml`](../../../../discovery/reference/discovery-service/chart/VERSION.toml)) · **Sources:** the [source skill package](../../NOTICE.md); operator [`README.md`](../../../../discovery/reference/discovery-service/operator/README.md), [`docs/service/docs/`](../../../../discovery/reference/discovery-service/operator/docs/service/docs/index.md), [`helm/auto-discovery-service/`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml); legacy [`chart/`](../../../../discovery/reference/discovery-service/chart/README.md)

Discovery tells clients which deployed service implements which API version and where to reach it. Clients (Kit/Client Library, Navigator) are configured with one Discovery base URL. API contracts are in [../apis/discovery.md](../apis/discovery.md); the component map is [../components/discovery.md](../components/discovery.md).

## Choose the implementation

| | Auto Discovery operator | Legacy static chart |
| --- | --- | --- |
| How it learns services | Watches annotated Kubernetes Services and calls each one's gRPC `CapabilitiesService/ListServices`: v2alpha on released 1.1.1; the snapshot calls v2beta and falls back to v2alpha on `UNIMPLEMENTED` | A static list in Helm values, served by nginx |
| Catalog | `GET /api/v2/services` (`schema-version: 2`); the snapshot adds the primary route `/api/v2beta/services` and keeps `/api/v2/services` as an alias; v1 compatibility at `/api/v1/services` and `/api/v1alpha/services` | `GET /api/v1/services` and `/api/v1alpha/services` (`schema-version: 1`) |
| Release | 1.1.1 in the 1.3.0 inventory | 2.3.11 in the previous inventory |
| Use for | Current deployments | Existing static deployments; targets that implement neither discovery capabilities version (for example this snapshot's Python filesystem example) |

The two charts have unrelated values schemas. Do not carry `discovery.services` into the operator, or operator keys (`config.kubeNamespace`, `rbac`, annotations) into the static chart.

**Released versus snapshot.** This repository has operator 1.2.0 source, newer than the released 1.1.1. Features such as multi-namespace `config.kubeNamespace` lists, `httpProxy.endpoints[].additionalRoutes`, and `httpProxy.rootRedirect` may not exist in 1.1.1; there is no operator changelog to confirm. Before reusing any key below on a 1.1.1 install, check `helm show values auto-discovery-service-1.1.1.tgz`.

## Install the operator

Minimum values, after creating the namespace and pull secret ([artifact-access.md](artifact-access.md)):

```yaml
# discovery-values.yaml
image:
  repository: nvcr.io/nvidia/omniverse/auto-discovery-service
  tag: "1.1.1"                 # the image you deploy; empty means chart appVersion
  pullSecrets:
    - name: ngcpull-secret     # the snapshot default is "regcred"
config:
  kubeNamespace: ""            # empty = the release namespace; or "ns-a,ns-b" or a YAML list
  reconcileInterval: 30
service:
  ports:
    internal: 8080             # cluster-internal catalog
    external: 8081             # catalog with client-facing URLs
podSecurityContext:
  runAsNonRoot: true
  seccompProfile: { type: RuntimeDefault }
securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities: { drop: [ALL] }
```

The image already runs as UID/GID 1000; the security contexts above are from the operator's [operations doc](../../../../discovery/reference/discovery-service/operator/docs/service/docs/operations.md).

```bash
helm upgrade --install auto-discovery-service <chart> -n <ns> -f discovery-values.yaml
kubectl -n <ns> rollout status deployment/auto-discovery-service
```

Environment-backed settings are read at startup. The generated v1-mapping ConfigMap has no checksum annotation, so after a mapping-only change run `kubectl -n <ns> rollout restart deployment/auto-discovery-service`. The `autoscaling` values are not rendered into an HPA; use `replicaCount`.

Resources default to 25m/32Mi requests and 250m/128Mi limits, sized for one namespace with modest service counts.

### RBAC and least privilege

With `rbac.create: true` (default) the chart creates a **ClusterRole and ClusterRoleBinding** with `get`, `list`, `watch` on `services` and `projectcontour.io/httpproxies`, even when only one namespace is watched. When `config.kubeNamespace` (or the release namespace default) is set, the operator uses namespaced API clients ([`src/k8s/discovery.rs`](../../../../discovery/reference/discovery-service/operator/src/k8s/discovery.rs)), so a namespaced alternative is possible: set `rbac.create: false` and create, in each watched namespace, a Role with the same rules and a RoleBinding to the operator's ServiceAccount. Verify the operator still logs successful reconciliation.

`--use-kube-portforward` mode additionally needs `pods` and `pods/portforward` permissions, which the chart never grants. Do not enable it in production.

`serviceAccount.automount: true` is required: the operator talks to the Kubernetes API.

## Register services

On each target **Service** (not the Pod):

| Annotation | Required | Meaning |
| --- | --- | --- |
| `discovery.omniverse.nvidia.com/enabled` | yes | exactly `"true"` |
| `discovery.omniverse.nvidia.com/grpc-port` | when no Service port is named `grpc` | port for the capabilities query and the advertised gRPC URL |
| `discovery.omniverse.nvidia.com/visibility` | no | `internal`, `external`, or `both` (default; unknown values also mean `both`) |
| `discovery.omniverse.nvidia.com/external-rest-endpoint` | no | client-facing REST base URL; overrides HTTPProxy |
| `discovery.omniverse.nvidia.com/external-grpc-endpoint` | no | client-facing gRPC URL; overrides HTTPProxy |
| `discovery.omniverse.nvidia.com/base-path` | no, **on the HTTPProxy** | appended to the HTTPProxy-derived REST URL |

Requirements the operator enforces:

- The target must serve discovery `ListServices` on the resolved gRPC port: v2alpha works with both the released and snapshot operators (the snapshot logs a deprecation warning for it); v2beta works only with the snapshot. Every service in this snapshot serves v2alpha only. A target that has never answered is omitted; one that stops answering keeps its last entry until it is removed. The snapshot follows `next_page_token` for up to 100 pages and fails the query, keeping the last entry, on a repeated token or a failed page.
- REST is published only if a Service port is named `http` or `rest` and the target advertises REST capabilities. No default port is assumed.
- Services are keyed by name; with several watched namespaces, the first-listed namespace wins a name collision (logged).

Annotation key names are configurable under `config.annotations.*`; keep the defaults unless every chart is changed with them.

How the snapshot charts annotate their Services:

| Component | How | Notes |
| --- | --- | --- |
| Storage Service | `service.annotations` | Also rendered on `storage-service-headless`; expect a second entry ([production-stack/storage-service.md](production-stack/storage-service.md#register-with-discovery)) |
| Thumbnail Service | annotated `enabled: "true"` by default | [thumbnails.md](thumbnails.md) |
| Event Aggregation / Event Consumer | `service.annotations` | Consumer can also render external endpoint annotations; [notifications.md](notifications.md) |
| Permission Service, User Info Service | `service.annotations` | Permission Service also annotates its headless Service; [permissions-and-auth.md](permissions-and-auth.md), [userinfo.md](userinfo.md) |
| Live Edit (released chart) | `service.annotations` with `grpc-port: "50001"` | [live-edit.md](live-edit.md) |
| Python filesystem example | not registrable in this snapshot | no discovery capabilities service (v2alpha or v2beta); [local-development.md](local-development.md#discovery-choice-for-the-example-stack) |

Discoverability does not grant API permission and does not create an ingress route.

### External addresses

For the external catalog, each transport needs an explicit annotation or a Contour HTTPProxy route in the target's namespace that references the target Service: a route whose port matches the gRPC port (or protocol `h2`) gives the gRPC URL; a non-`h2` route on the REST port gives the REST URL. The scheme is `https` when the virtual host has TLS. Never advertise cluster DNS names to external clients. Without Contour (CRD absent), external URLs come only from annotations.

## Authentication bootstrap

```yaml
# oidc-config.yaml
openId:
  enabled: true
  description: "OpenID Authentication"
  openIdConfigurationUri: "https://login.microsoftonline.com/<tenant-id>/v2.0/.well-known/openid-configuration"
  clientRegistrations:
    - name: default
      clientId: "<client-id>"
      scope: "openid profile email offline_access <client-id>/.default"
```

The chart refuses placeholder or empty values. It renders `AUTH_CONFIG_JSON`, served at `/api/v1/auth-config` and `/api/v2/auth-config` (the snapshot also serves `/api/v1alpha/auth-config` and `/api/v2beta/auth-config`) as `{"_comment", "openid_configuration", "clients": {<name>: {"client_id", "scope"}}}`. If there is a `default` registration and no `client_library` or `navigator` registration, the chart copies `default` to those names. The HTTPProxy always exempts these routes from external authorization (the snapshot regex covers all four). This endpoint only advertises how to authenticate; token validation is the auth extension's job ([permissions-and-auth.md](permissions-and-auth.md)). Without `openId.enabled` the routes return 404, which clients treat as "no auth".

## Ingress

```yaml
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.example.com
      port: 8081               # serve the external catalog to clients
  tls: { enabled: true, secretName: discovery-tls }
  authExtension:
    enabled: true              # protects every route except auth-config
```

Without `endpoints`, one HTTPProxy is rendered for `httpProxy.fqdn.host`.`httpProxy.fqdn.domain` (or just the domain when `host` is empty) on the external port. Each additional `endpoints[]` entry gets its own HTTPProxy. `httpProxy.rootRedirect` makes the operator answer `/` with a redirect (Contour's own redirect policy is not used because it ignores `authPolicy.disabled`).

### One FQDN for v1 and v2 clients

During migration from the static chart, keep one FQDN by letting the operator own the root HTTPProxy and routing legacy paths to the legacy Service:

```yaml
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.example.com
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          service: { name: discovery-service, port: 8080 }
        - exact: /api/v1alpha/services
          service: { name: discovery-service, port: 8080 }
```

Disable the legacy chart's own root HTTPProxy. Each route takes exactly one of `exact`, `prefix`, `regex`. Alternatively drop the legacy chart and use the operator's built-in v1 compatibility below.

## v1 compatibility

`config.v1Endpoints.enabled` (default `true`) serves `/api/v1/services` and `/api/v1alpha/services` by mapping advertised capabilities to legacy `{id, name, type, rest, grpc}` entries. Each mapping has `name`, `idPrefix`, `type`, and `endpoints.grpc.from` / `endpoints.rest.from` lists of gRPC service names and REST endpoint names. One match uses `idPrefix` unchanged; several matches get `-0`, `-1`, ... suffixes. The defaults cover Storage, Event Aggregation, Event Consumer, Live Edit (two entries, types `live-service` and `live-edit`), Permission, User Info, and Thumbnail. If the list is empty, v1 routes return 404. Older Client Library builds need v1; Navigator UI 1.0.4 tries v2 first.

## Static mode

`extraVars.STATIC_ENDPOINTS: "name=http://host:50051#http://host:8080,..."` disables all Kubernetes access and queries those gRPC endpoints instead. Entries go to both catalogs. Use it for non-Kubernetes targets or testing.

## Observability

`observability.logging.level` (tracing filter syntax), `observability.logging.structured.enabled`, `observability.tracing.{enabled,endpoint.grpc,samplingRatio}`, `observability.metrics.{enabled,endpoint.grpc}` (OTLP gRPC). No exporters are configured by default. See [../operate/monitoring.md](../operate/monitoring.md).

## Verify

```bash
kubectl -n <ns> port-forward service/auto-discovery-service 8080:8080
curl -fsS http://127.0.0.1:8080/api/v2/services | jq '."schema-version", [.services[].name]'
curl -fsS "${DISCOVERY_BASE_URL%/}/api/v2/services" | jq .          # from a client network, with a bearer token if required
```

`/api/v2/services` works on both released and snapshot operators; on the snapshot it is an alias of `/api/v2beta/services`, which its chart probes use. Require `schema-version` 2, the expected services, and advertised URLs reachable from the client network. Liveness/readiness only prove the listener responds (an empty catalog is a 200).

The read-only conformance runner checks a catalog and, optionally, explicit capability endpoints, with JSON/JUnit reports:

```bash
cd discovery/reference/discovery-service/operator
uv run --project conformance --locked discovery-conformance \
  --catalog-url https://discovery.example.com/api/v2/services --expect-service <name> \
  --token-file /run/secrets/catalog-token --json-report result.json
```

Tokens are read from files only and require TLS. The snapshot runner's gRPC capabilities profile is v2beta only, so `--grpc-target` against a v2alpha-only service (every service in this snapshot) fails with `UNIMPLEMENTED` even when the operator lists it. See [`conformance/README.md`](../../../../discovery/reference/discovery-service/operator/conformance/README.md).

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Catalog empty | Annotation exactly `"true"`; Service in a watched namespace; resolvable gRPC port; target serves discovery v2alpha or v2beta `ListServices` (`grpcurl -plaintext <svc>:<port> list` if reflection is on); operator logs (`observability.logging.level: auto_discovery_service=debug`) |
| Missing only externally | Visibility allows external; external annotation or matching HTTPProxy exists in the same namespace |
| REST block missing | Service port named `http` or `rest`; target advertises REST capabilities |
| v1 route 404 | `config.v1Endpoints.enabled` and at least one mapping |
| auth-config 404 | `openId.enabled` with real values |
| Mapping change not visible | Restart the Deployment |
| HTTPProxy invalid | Contour installed; FQDN/port set; one match form per additional route; only one root proxy per FQDN |
| 401 on catalog | Expected with `authExtension.enabled`; send a token. Bootstrap must stay public. |

Published guide runbooks that query `/v2alpha/services` are hitting a service's own REST capabilities route (the Storage Service serves it), not the operator's catalog.

## Legacy static chart

Values ([`values.yaml`](../../../../discovery/reference/discovery-service/chart/helm/values.yaml)): `discovery.services[]` entries with `id`, `name`, `type`, and `endpoints.grpc` / `endpoints.rest` objects (`host`, `port`, `path`, `tls`). An empty `host` uses a per-type in-cluster default (`storage` → `storage-service.<ns>.svc.cluster.local:8011/8012`, `permission` → `permission-service` 3010/3000, `notification-*` → 50051/8000, `userinfo` → 50051/8080, `thumbnail` → 50051/3000, `live-edit` → 50001), or `<ingressHost>.<domain>` with `httpProxy.enabled`. `openId.*` serves `/api/v1/auth-config` the same way. nginx listens on 8080 and serves `/health`.

For the released 2.3.11 chart, follow the [published Discovery deployment guide](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/discovery-service.html) and `helm show values`; set `image.repository: nvcr.io/nvidia/omniverse/simple-nginx` and `image.tag: "1.1.0"`. The snapshot chart defaults to `nginx:1.31` instead. The chart README's flat `grpc:`/`rest:` URL example does not match the template. Worked example: [local-development/filesystem-example-stack.md](local-development/filesystem-example-stack.md#add-discovery).

To migrate to the operator: annotate the target Services, install the operator alongside, compare its v1 compatibility output with the static catalog, move clients to v2 ([v1-to-v2 migration](../../../../discovery/reference/discovery-service/operator/docs/v1-to-v2-migration.md)), then remove the static chart.
