(auto-discovery-service-operations-operations)=
# Operations

The default deployment emits structured application logs at `info`, exposes HTTP health checks on the internal catalog endpoint, and requests modest CPU and memory resources. Metrics and distributed tracing exporters are not configured by the chart. When no optional operational settings are supplied, the service runs with one replica and the default scheduling policy.

(auto-discovery-service-operations-health-and-readiness)=
## Health and Readiness

Both liveness and readiness probes call `GET /api/v2beta/services` on the internal listener. The endpoint remains successful with an empty catalog, so these probes verify that the HTTP listener responds; they do not verify that any target service was discovered or that external URL resolution works.

The same endpoint is suitable for a basic manual check:

```console
kubectl --namespace discovery port-forward service/auto-discovery-service 8080:8080
curl --fail --show-error http://127.0.0.1:8080/api/v2beta/services
```

(auto-discovery-service-operations-logging)=
## Logging

The chart sets the runtime log filter from `observability.logging.level`. Use `info` for normal operation or a more specific filter during diagnosis. Reconciliation publishes an informational message when a catalog snapshot changes; stable passes are logged at a lower level.

```yaml
observability:
  logging:
    level: auto_discovery_service=debug,hyper=warn
```

(auto-discovery-service-operations-resource-and-scheduling-settings)=
## Resource and Scheduling Settings

| Setting | Helm value | Default | Description |
|---|---|---|---|
| CPU request | `resources.requests.cpu` | `25m` | Reserved CPU. |
| Memory request | `resources.requests.memory` | `32Mi` | Reserved memory. |
| CPU limit | `resources.limits.cpu` | `250m` | CPU limit. |
| Memory limit | `resources.limits.memory` | `128Mi` | Memory limit. |
| Pod annotations | `podAnnotations` | `{}` | Additional Pod annotations. |
| Pod labels | `podLabels` | `{}` | Additional Pod labels. |
| Pod security context | `podSecurityContext` | `{}` | Pod-level security settings. |
| Container security context | `securityContext` | `{}` | Container-level security settings. |
| Node selector | `nodeSelector` | `{}` | Node label constraints. |
| Tolerations | `tolerations` | `[]` | Scheduling tolerations. |
| Affinity | `affinity` | `{}` | Pod affinity and anti-affinity rules. |
| Additional volumes | `volumes` | `[]` | Additional Pod volumes. |
| Additional mounts | `volumeMounts` | `[]` | Additional container volume mounts. |

The values file includes an `autoscaling` section, but the current chart does not render a HorizontalPodAutoscaler. Changing `autoscaling.*` has no effect until the chart adds that resource. Scale with `replicaCount` or an independently managed autoscaler.

(auto-discovery-service-operations-configuring-resources-in-the-helm-chart)=
### Configuring Resources in the Helm Chart

```yaml
replicaCount: 2

resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    cpu: 500m
    memory: 256Mi

podSecurityContext:
  runAsNonRoot: true
  seccompProfile:
    type: RuntimeDefault

securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop:
      - ALL
```

The image already runs as user and group `1000`. Verify any additional volumes are writable by that identity when the read-only root filesystem is enabled. Install or update the release as described in {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`.

(auto-discovery-service-operations-troubleshooting)=
## Troubleshooting

| Symptom | Checks |
|---|---|
| Catalog is empty | Confirm target Services have the configured discovery annotation set to `"true"`, are in the watched namespace, and expose a resolvable gRPC port. |
| A target disappears only from the external listener | Confirm its visibility permits external output and that an explicit external endpoint or matching HTTPProxy route exists. |
| REST block is absent | Confirm the target advertises at least one REST capability and its Kubernetes Service has a port named `http` or `rest`, or an external REST override. |
| gRPC query fails | Confirm the target port annotation or `grpc` Service port, network reachability, and `CapabilitiesService.ListServices` support. |
| Version 1 endpoint returns 404 | Enable `config.v1Endpoints` and provide at least one mapping. |
| Auth-config endpoint returns 404 | Enable `openId` and supply valid provider and client registration values. |
| Updated version 1 mappings are not visible | Restart the Deployment after the Helm upgrade because the ConfigMap does not trigger an automatic rollout. |
| HTTPProxy is missing or invalid | Confirm Contour is installed, required FQDN/port values are set, and every additional route specifies exactly one match form. |

(auto-discovery-service-operations-local-validation)=
## Local Validation

The repository provides operator-facing scripts that deploy a local Contour environment, run HTTP assertions, and validate chart rendering:

```console
./deploy/local/install.sh
./deploy/local/test.sh
./deploy/local/test-helm.sh
./deploy/local/cleanup.sh
```

The local environment uses `discovery.local` and exercises version 1, version 2, and auth-config routes through one HTTPProxy.

(auto-discovery-service-operations-references)=
## References

- [Kubernetes resource management for Pods and containers](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
- [Kubernetes liveness and readiness probes](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/)
