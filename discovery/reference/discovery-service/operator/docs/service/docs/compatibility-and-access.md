(auto-discovery-service-compatibility-and-access-compatibility-authentication-and-ingress)=
# Compatibility, Authentication, and Ingress

Version 1 compatibility is enabled by default. Authentication bootstrap, Contour ingress, TLS certificate creation, and the root redirect are disabled by default. When these optional features are not used, version 2 discovery remains available through the cluster Service on the internal and external ports.

(auto-discovery-service-compatibility-and-access-version-1-compatibility)=
## Version 1 Compatibility

Compatibility mappings translate advertised capability names into legacy service entries. Each mapping defines a display name, ID prefix, legacy type, and lists of matching gRPC service names and REST endpoint names. A discovered service is included when either transport matches.

When exactly one target matches a mapping, the configured ID prefix is used unchanged. When several targets match, the service appends a zero-based numeric suffix. A mapping can yield a REST-only or gRPC-only entry.

The chart writes mappings to a ConfigMap and supplies them to the process at startup. The generated ConfigMap has no workload checksum annotation, so changing only the mappings does not automatically restart existing Pods.

(auto-discovery-service-compatibility-and-access-authentication-bootstrap)=
## Authentication Bootstrap

OpenID bootstrap is off by default. When enabled, the chart requires an HTTP or HTTPS provider metadata URL and at least one client registration containing `name`, `clientId`, and `scope`. Placeholder or empty client IDs are rejected during chart rendering.

The service publishes the resulting JSON at `/api/v1/auth-config`, `/api/v1alpha/auth-config`, `/api/v2/auth-config`, and `/api/v2beta/auth-config`. These bootstrap routes are served without Contour external authorization so clients can obtain authentication configuration before authenticating. The bootstrap response does not itself authenticate requests to the discovery catalog.

(auto-discovery-service-compatibility-and-access-ingress)=
## Contour Ingress

When `httpProxy.enabled` is true, the chart creates one HTTPProxy per configured endpoint. Each endpoint selects an FQDN and one of the discovery Service ports. When the endpoint list is empty, the legacy single-FQDN settings route to the external port.

The generated HTTPProxy includes:

- A public auth-config route for API versions `v1`, `v1alpha`, `v2`, and `v2beta`.
- Optional additional exact, prefix, or regular-expression routes to another in-cluster Service.
- An optional root path routed to the service for application-level redirection.
- A catch-all prefix route to the selected discovery listener.
- Optional Contour external authorization on routes that do not explicitly disable it.
- Optional TLS and cert-manager Certificate creation.

Each additional route must set exactly one match form: `exact`, `prefix`, or `regex`. The template rejects an invalid combination.

(auto-discovery-service-compatibility-and-access-configuration)=
## Configuration

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Version 1 responses | `config.v1Endpoints.enabled` | `true` | Enables version 1 and v1alpha compatibility endpoints. |
| Compatibility mappings | `config.v1Endpoints.endpoints` | Built-in service mappings | Maps capability names to legacy names, IDs, and types. |
| Auth bootstrap | `openId.enabled` | `false` | Publishes OpenID client bootstrap configuration. |
| Provider metadata | `openId.openIdConfigurationUri` | Empty | OpenID Provider configuration URL; required when enabled. |
| Client registrations | `openId.clientRegistrations` | `[]` | Named client IDs and scopes; at least one is required when enabled. |
| HTTPProxy creation | `httpProxy.enabled` | `false` | Creates Contour ingress resources. |
| Ingress endpoints | `httpProxy.endpoints` | `[]` | Named FQDN and service-port pairs. |
| Legacy FQDN | `httpProxy.fqdn` | `nodomain.none` | Single endpoint used only when `endpoints` is empty. |
| Route prefix | `httpProxy.path` | `/` | Prefix for the discovery catch-all route. |
| Root redirect | `httpProxy.rootRedirect.enabled` | `false` | Enables application-level redirect for an exact root path. |
| TLS | `httpProxy.tls.enabled` | `false` | Enables TLS on generated HTTPProxies. |
| Certificate creation | `httpProxy.tls.certManager.enabled` | `false` | Creates a cert-manager Certificate for ingress FQDNs. |
| CORS policy | `httpProxy.corsPolicy` | GET with wildcard origins/headers | Contour CORS policy rendered verbatim. |
| External authorization | `httpProxy.authExtension.enabled` | `false` | Enables a Contour authorization extension. |
| Route timeouts | `httpProxy.timeoutPolicy` | `10s` response and idle | Per-route response and idle timeouts. |

(auto-discovery-service-compatibility-and-access-configuring-compatibility-and-access-in-the-helm-chart)=
### Configuring Compatibility and Access in the Helm Chart

```yaml
config:
  v1Endpoints:
    enabled: true
    endpoints:
      - name: Storage Service
        idPrefix: storage-service
        type: storage
        endpoints:
          grpc:
            from:
              - nvidia.omniverse.storage.fileobject.v1.FileObjectService
          rest:
            from:
              - fileobject

openId:
  enabled: true
  description: OpenID Authentication
  openIdConfigurationUri: https://identity.example.com/.well-known/openid-configuration
  clientRegistrations:
    - name: default
      clientId: discovery-client
      scope: openid profile

httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.example.com
      port: 8081
  path: /
  tls:
    enabled: true
    secretName: discovery-tls
    certManager:
      enabled: false
  rootRedirect:
    enabled: true
    target:
      fqdn: docs.example.com
      path: /discovery
      statusCode: 302
```

Install or update the release as described in {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`. Restart the Deployment after a mapping-only change.

(auto-discovery-service-compatibility-and-access-references)=
## References

- [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html)
- [Contour HTTPProxy virtual hosts](https://projectcontour.io/docs/main/config/virtual-hosts/)
- [Kubernetes Ingress concepts](https://kubernetes.io/docs/concepts/services-networking/ingress/)

(auto-discovery-service-compatibility-and-access-see-next)=
## See next

- {ref}`Operations <auto-discovery-service-operations-operations>`
