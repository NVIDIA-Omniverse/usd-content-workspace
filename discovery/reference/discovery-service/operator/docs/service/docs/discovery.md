(auto-discovery-service-discovery-discovery-and-reconciliation)=
# Discovery and Reconciliation

Kubernetes discovery is the default operating mode. The service watches the selected namespace, but it queries only Services whose configured discovery annotation is exactly `"true"`. Without that annotation, a target is ignored and never appears in either catalog. When static endpoints are configured, Kubernetes discovery is disabled and those endpoints become the complete discovery source.

(auto-discovery-service-discovery-kubernetes-flow)=
## Kubernetes Discovery Flow

Each reconciliation follows this observable sequence:

1. A Kubernetes Service or HTTPProxy watch event, or the periodic timer, requests reconciliation.
2. A 500 millisecond debounce window combines a burst of changes into one pass.
3. The service lists Services and, when available, Contour HTTPProxy resources in the selected namespace.
4. Services without the discovery-enabled annotation are discarded.
5. For each opted-in Service, the gRPC port is resolved and `CapabilitiesService.ListServices` is called, following `next_page_token` until all pages are collected. A query that fails on any page, repeats a page token, or exceeds 100 pages is treated as a failed query.
6. REST and gRPC addresses are resolved independently for the internal and external views.
7. Complete replacement snapshots are built and published only when their contents change.

The Capabilities contract is described in the Omniverse Discovery Capabilities API documentation. This service uses the gRPC `ListServices` method as its capability source.

(auto-discovery-service-discovery-failure-behavior)=
## Failure Behavior

A failed capability query does not immediately remove a previously discovered target. The service retains the target's last known internal and external entries for that reconciliation pass. A new target that has never completed a successful query has no previous entry and is omitted.

If the Contour HTTPProxy custom resource is unavailable, HTTPProxy listing becomes an empty result and Kubernetes discovery continues. Internal addresses and explicit external endpoint annotations remain usable.

A target is skipped when its gRPC port cannot be determined. No implicit gRPC port is assumed.

(auto-discovery-service-discovery-static-mode)=
## Static Endpoint Mode

Static mode is off by default. Setting one or more `STATIC_ENDPOINTS` values, or equivalent repeated command-line options, disables all Kubernetes access. Each entry has one of these forms:

```text
NAME=GRPC_URL
NAME=GRPC_URL#REST_URL
```

The gRPC URL is required because the service obtains capability data through `ListServices`. The optional REST URL becomes the advertised REST base URL. The gRPC URL becomes the advertised gRPC base URL. Static entries are published to both internal and external catalog views and are refreshed at the normal reconciliation interval.

(auto-discovery-service-discovery-configuration)=
## Configuration

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Discovery opt-in key | `config.annotations.discoveryEnabled` | `discovery.omniverse.nvidia.com/enabled` | Annotation that must have value `"true"`. |
| gRPC port key | `config.annotations.grpcPort` | `discovery.omniverse.nvidia.com/grpc-port` | Annotation that overrides target gRPC port selection. |
| Visibility key | `config.annotations.visibility` | `discovery.omniverse.nvidia.com/visibility` | Annotation selecting `internal`, `external`, or `both`. |
| Reconciliation interval | `config.reconcileInterval` | `30` | Periodic refresh interval in seconds. |
| Static targets | `extraVars.STATIC_ENDPOINTS` | Unset | Comma-separated static endpoint entries; enables static mode. |
| Port-forward mode | `extraVars.USE_KUBE_PORTFORWARD` | Unset (`false`) | Queries a selected target Pod through the Kubernetes API instead of cluster DNS. |

(auto-discovery-service-discovery-configuring-discovery-in-the-helm-chart)=
### Configuring Discovery in the Helm Chart

Kubernetes mode with customized annotation names:

```yaml
config:
  kubeNamespace: workloads
  reconcileInterval: 20
  annotations:
    discoveryEnabled: example.com/discovery-enabled
    grpcPort: example.com/discovery-grpc-port
    basePath: example.com/discovery-base-path
    visibility: example.com/discovery-visibility
    externalRestEndpoint: example.com/external-rest
    externalGrpcEndpoint: example.com/external-grpc
```

Static mode for two targets:

```yaml
extraVars:
  STATIC_ENDPOINTS: >-
    storage=http://storage:50051#http://storage:8080,
    permissions=http://permissions:50051#http://permissions:8080
config:
  reconcileInterval: 30
```

Install or update the release as described in {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`.

(auto-discovery-service-discovery-target-service-opt-in)=
## Target Service Opt-In

The following Kubernetes Service opts in, identifies its gRPC port, and publishes it to both catalog views:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: storage
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"
    discovery.omniverse.nvidia.com/grpc-port: "50051"
    discovery.omniverse.nvidia.com/visibility: both
spec:
  ports:
    - name: grpc
      port: 50051
    - name: http
      port: 8080
```

(auto-discovery-service-discovery-references)=
## References

- [Kubernetes Service API](https://kubernetes.io/docs/reference/kubernetes-api/core/service-v1/)
- [Kubernetes DNS for Services and Pods](https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/)

(auto-discovery-service-discovery-see-next)=
## See next

- {ref}`Endpoint Resolution <auto-discovery-service-endpoint-resolution-endpoint-resolution>`
