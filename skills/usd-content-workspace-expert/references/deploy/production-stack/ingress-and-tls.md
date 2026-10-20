# Ingress, DNS, and TLS (Contour)

> **Applies to:** Contour HTTPProxy ingress for Storage Service, Auto Discovery, Storage Navigator; hand-off to the Envoy auth extension · **Released:** charts in the Developer Guide 1.3.0 inventory; Contour itself is third-party and not part of the NGC inventory · **This repo's snapshot:** `httpProxy` templates in the Storage ([`http-proxy.yaml`](../../../../../storage/reference/storage-service/helm/templates/http-proxy.yaml)), operator ([`http-proxy.yaml`](../../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/templates/http-proxy.yaml)), and Navigator ([`http-proxy.yaml`](../../../../../examples/navigator-ui/helm/storage-navigator/templates/http-proxy.yaml)) charts; operator local tests pin Contour v1.33.5 · **Sources:** the [source skill package](../../../NOTICE.md); [`http-proxy.rst`](../../../../../storage/reference/storage-service/docs/helm/http-proxy.rst); [operator compatibility-and-access doc](../../../../../discovery/reference/discovery-service/operator/docs/service/docs/compatibility-and-access.md)

The stack's charts expose themselves through Contour `HTTPProxy` resources (`projectcontour.io/v1`). Other ingress controllers can work, but then the charts' `httpProxy` blocks do nothing and the Auto Discovery operator cannot derive external URLs from HTTPProxies; you must set the external-endpoint annotations yourself ([../discovery.md](../discovery.md#external-addresses)).

## Install Contour

Install Contour with your platform's method or Contour's own documentation, or reuse an existing installation. This repository does not ship a production Contour install. Two references:

- The operator's local test uses the pinned rendered manifest [`deploy/local/contour-v1.33.5.yaml`](../../../../../discovery/reference/discovery-service/operator/deploy/local/contour-v1.33.5.yaml), patched for a local ClusterIP Envoy. It is a test fixture, not a production manifest.
- The source skill package installs the projectcontour Helm chart (`helm repo add contour https://projectcontour.github.io/helm-charts/`, chart version `0.2.0`, namespace `contour-system`). That chart version is unverified in this snapshot; check the chart repository before using it.

On AWS with an NLB, the source skill package sets these Service annotations through the Contour chart's `envoy.service.annotations` (verify the key against the Contour chart you install):

```yaml
envoy:
  service:
    annotations:
      service.beta.kubernetes.io/aws-load-balancer-type: nlb
      service.beta.kubernetes.io/aws-load-balancer-nlb-target-type: ip
      service.beta.kubernetes.io/aws-load-balancer-scheme: internet-facing
      service.beta.kubernetes.io/aws-load-balancer-security-groups: <security-group-id>
      service.beta.kubernetes.io/aws-load-balancer-attributes: load_balancing.cross_zone.enabled=true
```

Restrict the security group to the client networks that need access.

Contour's `enableExternalNameService` can stay at its secure default (`false`); the operator's single-FQDN v1/v2 routing does not need it.

## DNS and hostnames

Point a DNS name `<domain>` (and the subdomains below) at the Envoy load balancer. Snapshot chart defaults produce these virtual hosts:

| Chart | Virtual host(s) | Values |
| --- | --- | --- |
| Storage Service | REST `storage.<domain>`; gRPC `grpc.storage.<domain>` (route protocol `h2c`) | `httpProxy.fqdn.host` (default `storage`), `httpProxy.fqdn.domain`; per-protocol overrides `httpProxy.rest`, `httpProxy.grpc` |
| Auto Discovery | `<domain>` when `fqdn.host` is empty, else `<host>.<domain>`; or one HTTPProxy per `httpProxy.endpoints[]` entry with its own `fqdn` and port | `httpProxy.fqdn.*`, `httpProxy.endpoints[]` |
| Storage Navigator | `navigator.<domain>` | `httpProxy.fqdn.host` (default `navigator`), `httpProxy.fqdn.domain` |
| Notifications, Permission Service, Permission UI, User Info, Thumbnails, Live Edit | see owners | [../notifications.md](../notifications.md), [../permissions-and-auth.md](../permissions-and-auth.md), [../userinfo.md](../userinfo.md), [../thumbnails.md](../thumbnails.md), [../live-edit.md](../live-edit.md) |

A single HTTPProxy FQDN can only have one root proxy in Contour. If the legacy Discovery chart and the operator must share an FQDN during migration, let the operator own the root and use `httpProxy.endpoints[].additionalRoutes` for the legacy paths ([../discovery.md](../discovery.md#one-fqdn-for-v1-and-v2-clients)).

## Certificates

Get a certificate covering every virtual host you enable, for example:

```
<domain>
storage.<domain>
*.storage.<domain>          # covers grpc.storage.<domain>
navigator.<domain>
```

plus the names used by the other services you deploy (the source skill package lists `event-aggregation.<domain>`, `event-consumer.<domain>`, and their wildcards for notifications; `permissions.<domain>` and `web.permissions.<domain>` for the permission layer).

Provide it as a Kubernetes TLS Secret and set `httpProxy.tls.enabled: true` and `httpProxy.tls.secretName`, or let cert-manager issue it with `httpProxy.tls.certManager.enabled: true` and a real `issuerRef` (the chart defaults `my-issuer`/`my-issuer-kind` are placeholders). Each chart renders its own Certificate when cert-manager is enabled.

With TLS enabled the Storage chart adds `Strict-Transport-Security: max-age=31536000` on the REST route unless you set `httpProxy.responseHeadersPolicy`.

## Shared overlay

The keys below have the same meaning in the Storage, operator, and Navigator snapshot charts, so one overlay can be layered onto all three:

```yaml
# ingress-values.yaml
httpProxy:
  enabled: true
  fqdn:
    domain: "<domain>"
  tls:
    enabled: true
    secretName: usd-content-workspace-cert
  authExtension:
    enabled: true            # only after the Envoy auth extension is installed; ignored by Navigator
```

Do not put `fqdn.host` in the shared overlay; each chart has its own default host. Check any other chart's schema before applying this overlay to it.

```bash
helm upgrade --install storage-service <storage-chart> -n <ns> -f storage-values.yaml -f ingress-values.yaml
helm upgrade --install auto-discovery-service <discovery-chart> -n <ns> -f discovery-values.yaml -f ingress-values.yaml
kubectl get httpproxy -n <ns>         # STATUS should be "valid"
```

## CORS defaults

| Chart | Default | Effect |
| --- | --- | --- |
| Storage | `allowOrigin: []` | The template then allows the regex `http[s]?://.*\.<domain>`, i.e. any subdomain of `httpProxy.fqdn.domain`. Methods `GET PUT POST HEAD PATCH DELETE`. Set explicit origins if other subdomains are not trusted. |
| Auto Discovery | `allowOrigin: ["*"]`, `GET` | The catalog is readable cross-origin. Narrow it if that matters. |
| Navigator | none rendered | Static UI; not needed. |

## Authentication at the edge

When `httpProxy.authExtension.enabled` is true, the chart adds an `authorization` block pointing at `extensionRef.name` (default `auth-extension`) in `extensionRef.namespace` (default: the release namespace) with `responseTimeout` (default `2s`). Installing and configuring that extension service, OIDC, and the Permission Service is covered in [../permissions-and-auth.md](../permissions-and-auth.md).

The operator chart always renders `<path>/api/v(1|2)/auth-config` with `authPolicy.disabled: true`, so clients can fetch authentication bootstrap data before they have a token. Do not remove auth from the other routes to make bootstrap work.

After enabling auth, check both sides:

```bash
curl -sS https://<domain>/api/v1/auth-config | jq .     # public bootstrap, 404 if openId is not enabled
curl -sS -o /dev/null -w '%{http_code}\n' https://<domain>/api/v2/services   # expect 401 without a token
```

## External addresses in the catalog

With Storage's `httpProxy` enabled in the same namespace as the target Service, the operator derives:

- REST: `https://storage.<domain>` (`http://` without TLS), plus the HTTPProxy's `discovery.omniverse.nvidia.com/base-path` annotation if set.
- gRPC: `https://grpc.storage.<domain>`, from the route whose port matches the gRPC port (or whose protocol is `h2`).

Explicit `external-rest-endpoint` / `external-grpc-endpoint` Service annotations take precedence. Check the external catalog (port 8081, or through ingress) from a client network: [validation.md](validation.md).
