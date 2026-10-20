(auto-discovery-service-deployment-deployment-and-configuration)=
# Deployment and Configuration

The service is packaged as a non-root container and a Helm chart. The default chart deploys one Pod, a ClusterIP Service with internal port `8080` and external port `8081`, a dedicated service account, and cluster-scoped read permissions for Kubernetes Services and Contour HTTPProxy resources. Ingress and OpenID bootstrap are off by default; version 1 compatibility responses are on by default.

(auto-discovery-service-deployment-prerequisites)=
## Prerequisites

- A Kubernetes cluster with permissions to create the chart resources.
- Image pull access to `nvcr.io/nvidia/omniverse/auto-discovery-service` or an overridden image repository.
- Contour only when HTTPProxy discovery or ingress is required.
- cert-manager only when the chart should create a TLS Certificate.

The service needs `get`, `list`, and `watch` access to Kubernetes Services and Contour HTTPProxy resources. The default chart creates a ClusterRole and ClusterRoleBinding. When `rbac.create` is disabled, the selected service account must already have equivalent permissions.

(auto-discovery-service-deployment-installing-with-helm)=
## Installing with Helm

```console
helm upgrade --install auto-discovery-service \
  ./helm/auto-discovery-service \
  --namespace discovery \
  --create-namespace \
  --set image.tag=<release-tag>
```

The default empty `config.kubeNamespace` is rendered as the Pod namespace. Set it explicitly to watch a different namespace, or several: either a comma-separated string (`--set config.kubeNamespace=storage-api-dev\,usdsearch`) or a YAML list in a values file. The operator then discovers Services and HTTPProxies only in the listed namespaces and logs the list at startup. The service account needs list/watch on Services and HTTPProxies in each listed namespace; the default ClusterRole created by the chart covers that.

Published services are keyed by Service name, so an annotated Service name that exists in more than one listed namespace is published once, from the first-listed namespace, and the operator logs a warning naming both namespaces. List namespaces in the order you want collisions resolved. An HTTPProxy only supplies external URLs for Services in its own namespace.

(auto-discovery-service-deployment-core-settings)=
## Core Configuration

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Replicas | `replicaCount` | `1` | Number of service Pods when autoscaling is disabled. |
| Image repository | `image.repository` | `nvcr.io/nvidia/omniverse/auto-discovery-service` | Container image repository. |
| Image tag | `image.tag` | Empty | Uses the chart `appVersion` when empty. |
| Pull policy | `image.pullPolicy` | `IfNotPresent` | Kubernetes image pull policy. |
| Pull secrets | `image.pullSecrets` | `[{name: regcred}]` | Secrets used to pull the image. |
| Internal listener | `service.ports.internal` | `8080` | Port serving the internal catalog. |
| External listener | `service.ports.external` | `8081` | Port serving the external catalog. |
| Watched namespaces | `config.kubeNamespace` | Release namespace | Namespace(s) whose Services and HTTPProxies are discovered: a comma-separated string or a YAML list. The first-listed namespace wins a Service-name collision. |
| Kubeconfig context | `config.kubeContext` | Empty | Explicit kubeconfig context, normally unused in-cluster. |
| Reconciliation interval | `config.reconcileInterval` | `30` | Maximum seconds between periodic reconciliation triggers. |
| Log filter | `observability.logging.level` | `info` | Log level or tracing filter expression. |
| Service account creation | `serviceAccount.create` | `true` | Creates a service account for the workload. |
| API credential mount | `serviceAccount.automount` | `true` | Mounts service-account credentials into the Pod. |
| RBAC creation | `rbac.create` | `true` | Creates the required ClusterRole and binding. |
| Additional environment | `extraVars` | `{}` | Adds deployment-specific environment variables. |

(auto-discovery-service-deployment-configuring-the-helm-chart)=
### Configuring the Helm Chart

The following values select a namespace, shorten the periodic interval, set explicit image details, and use an existing service account:

```yaml
image:
  repository: nvcr.io/nvidia/omniverse/auto-discovery-service
  tag: "1.2.3"
  pullPolicy: IfNotPresent
  pullSecrets:
    - name: regcred

serviceAccount:
  create: false
  automount: true
  name: discovery-reader

rbac:
  create: false

config:
  kubeNamespace: workloads
  reconcileInterval: 15

observability:
  logging:
    level: info
```

Apply changes with `helm upgrade`. Environment-backed settings are read at process startup, so configuration changes take effect after the Deployment creates a new Pod. The chart does not add checksum annotations for its generated ConfigMap; if only version 1 endpoint mappings change and the Pod template is otherwise unchanged, restart the Deployment after the upgrade.

(auto-discovery-service-deployment-container-runtime)=
## Container Runtime

The production image exposes ports `8080` and `8081`, runs as user and group `1000`, and starts the service directly. The image contains the service executable, its license, and a compressed third-party source archive. It does not include a shell or Kubernetes client.

The chart maps its service ports to the same container ports through environment settings. Liveness and readiness checks call `GET /api/v2beta/services` on the internal listener.

(auto-discovery-service-deployment-references)=
## References

- [Kubernetes Services](https://kubernetes.io/docs/concepts/services-networking/service/)
- [Kubernetes RBAC](https://kubernetes.io/docs/reference/access-authn-authz/rbac/)

(auto-discovery-service-deployment-see-next)=
## See next

- {ref}`Discovery and Reconciliation <auto-discovery-service-discovery-discovery-and-reconciliation>`
