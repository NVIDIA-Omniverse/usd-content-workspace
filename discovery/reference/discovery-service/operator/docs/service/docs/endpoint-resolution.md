(auto-discovery-service-endpoint-resolution-endpoint-resolution)=
# Endpoint Resolution

Endpoint resolution converts capability declarations into callable base URLs for the internal and external catalog views. The service resolves REST and gRPC independently. An unresolved transport is omitted, while other resolved transports for the same service remain available.

(auto-discovery-service-endpoint-resolution-visibility)=
## Visibility

The target Service's visibility annotation accepts `internal`, `external`, or `both`. The default is `both`. Any unrecognized value also falls back to `both`.

- `internal` permits only the cluster-internal entry.
- `external` permits only the externally resolved entry.
- `both` permits each entry when its required addresses can be resolved.

Visibility is an output-selection setting, not an authorization control. It does not restrict network access to a target service.

(auto-discovery-service-endpoint-resolution-internal-addresses)=
## Internal Addresses

Internal addresses use the Kubernetes Service DNS name:

```text
http://<service>.<namespace>.svc.cluster.local:<port>
```

The gRPC port is selected in this order:

1. The configured gRPC-port annotation on the target Service.
2. A Service port named `grpc`.
3. No gRPC address; the target is skipped because the capability query cannot be made.

The REST port is the first Service port named `http` or `rest`. When neither exists, the catalog omits the REST block. No fallback REST or gRPC port is assumed.

(auto-discovery-service-endpoint-resolution-external-addresses)=
## External Addresses

An explicit external endpoint annotation has precedence for its transport. When no explicit annotation is set, the service looks for a matching Contour HTTPProxy route:

- The HTTPProxy virtual-host FQDN supplies the host.
- TLS on the virtual host selects `https`; otherwise the URL uses `http`.
- A route with protocol `h2` supplies the gRPC address.
- A route without `h2` supplies the REST address.
- The configured base-path annotation is appended to the REST address only.

A target without an explicit external endpoint or matching HTTPProxy route is omitted from the external catalog. Its internal entry can remain available.

(auto-discovery-service-endpoint-resolution-configuration)=
## Configuration

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Base-path annotation key | `config.annotations.basePath` | `discovery.omniverse.nvidia.com/base-path` | Names the HTTPProxy annotation appended to external REST URLs. |
| Visibility annotation key | `config.annotations.visibility` | `discovery.omniverse.nvidia.com/visibility` | Names the Service annotation selecting catalog views. |
| External REST annotation key | `config.annotations.externalRestEndpoint` | `discovery.omniverse.nvidia.com/external-rest-endpoint` | Names the Service annotation overriding the external REST base URL. |
| External gRPC annotation key | `config.annotations.externalGrpcEndpoint` | `discovery.omniverse.nvidia.com/external-grpc-endpoint` | Names the Service annotation overriding the external gRPC base URL. |

(auto-discovery-service-endpoint-resolution-configuring-endpoint-resolution-in-the-helm-chart)=
### Configuring Endpoint Resolution in the Helm Chart

```yaml
config:
  annotations:
    basePath: discovery.omniverse.nvidia.com/base-path
    visibility: discovery.omniverse.nvidia.com/visibility
    externalRestEndpoint: discovery.omniverse.nvidia.com/external-rest-endpoint
    externalGrpcEndpoint: discovery.omniverse.nvidia.com/external-grpc-endpoint
```

An opted-in target can bypass HTTPProxy resolution with explicit addresses:

```yaml
metadata:
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
    discovery.omniverse.nvidia.com/visibility: external
    discovery.omniverse.nvidia.com/external-rest-endpoint: https://api.example.com
    discovery.omniverse.nvidia.com/external-grpc-endpoint: https://grpc.example.com
```

Install or update the release as described in {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`.

(auto-discovery-service-endpoint-resolution-references)=
## References

- [Kubernetes DNS for Services and Pods](https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/)
- [Contour virtual hosts and HTTPProxy](https://projectcontour.io/docs/main/config/virtual-hosts/)

(auto-discovery-service-endpoint-resolution-see-next)=
## See next

- {ref}`Service-Specific HTTP API <auto-discovery-service-service-api-service-specific-http-api>`
