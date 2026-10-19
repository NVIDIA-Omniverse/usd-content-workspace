(storage-navigator-app-deployment-deployment)=
# Deployment

**Goal**: Deploy Storage Navigator to Kubernetes with the Storage Navigator Helm chart. The chart deploys a single nginx container serving the pre-built static bundle; there is no database or other stateful component to provision.

(storage-navigator-app-deployment-service-ports)=
## Service Ports

| Port | Protocol | Description |
|------|----------|--------------|
| 8080 | HTTP | Serves the static single-page application and its runtime `config.json`. |

(storage-navigator-app-deployment-installing-the-chart)=
## Installing the Chart

Create a values file to configure the release. At minimum, set the image pull secret your cluster uses to reach the container registry:

```yaml
imagePullSecrets:
  - name: ngcpull-secret
```

navigator-values.yaml

Validate and install:

```bash
# validate the chart
helm template . -f navigator-values.yaml

# dry-run the install
helm upgrade --install storage-navigator . -f navigator-values.yaml --namespace storage-apis --dry-run --debug
```

If everything looks good, install the release:

```bash
# install storage navigator
helm upgrade --install storage-navigator . -f navigator-values.yaml --namespace storage-apis

# validate the pod is running
kubectl get pods -n storage-apis
```

Verify the service is responding:

```bash
# test the health endpoint
curl http://storage-navigator.storage-apis.svc.cluster.local:8080/health
```

```{note}
This service exposes no APIs of its own beyond serving static content and the `/health` endpoint; there is no database or other stateful backend to provision before installing.
```

(storage-navigator-app-deployment-health-checks)=
## Health Checks

The chart configures both liveness and readiness probes against `/health` on the container's HTTP port:

| Probe | Path | Initial delay | Period | Timeout | Failure threshold |
|---|---|---|---|---|---|
| Liveness | `/health` | 10s | 10s | 5s | 3 |
| Readiness | `/health` | 5s | 5s | 3s | 3 |

(storage-navigator-app-deployment-resources-and-scaling)=
## Resources and Scaling

The chart requests `100m` CPU and `128Mi` memory per pod by default, with limits of `500m` CPU and `256Mi` memory. A single replica runs by default (`replicaCount: 1`); horizontal pod autoscaling is available but disabled by default, scaling between 1 and 100 replicas on 80% target CPU utilization when enabled.

```{note}
Autoscaling and `replicaCount` are mutually exclusive in the chart: when autoscaling is enabled, `replicaCount` and the rolling-update strategy are not applied, since the autoscaler manages replica count instead.
```

### Configuring Resources and Scaling in the Helm Chart

```yaml
resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    cpu: 500m
    memory: 256Mi

autoscaling:
  enabled: false
  minReplicas: 1
  maxReplicas: 100
  targetCPUUtilizationPercentage: 80
```

navigator-values.yaml

Apply changes with the install/upgrade commands in {ref}`Installing the Chart <storage-navigator-app-deployment-installing-the-chart>`.

(storage-navigator-app-deployment-ingress-access)=
## Ingress Access

Ingress is **off by default** (`httpProxy.enabled: false`). To expose Storage Navigator outside the cluster, enable the `httpProxy` section; the chart creates a [Contour HTTPProxy](https://projectcontour.io/docs/) resource routing to the service.

```yaml
httpProxy:
  enabled: true
  fqdn:
    host: "navigator"
    domain: "{DNS_URL}"
  tls:
    enabled: true
    certManager:
      enabled: true
      issuerRef:
        name: my-issuer
        kind: my-issuer-kind
```

navigator-values.yaml

By default the proxy routes every path prefix (`httpProxy.path: "/"`), and applies a 10-second response timeout and a 3-second idle timeout. When both `tls.enabled` and `tls.certManager.enabled` are set, the chart also creates a CertManager `Certificate` resource for the route's TLS secret; the secret name defaults to `tls-cert-<release-name>` and can be overridden with `tls.secretName`.

Redeploy with the updated values:

```bash
helm upgrade --install storage-navigator . -f navigator-values.yaml --namespace storage-apis

kubectl get httpproxy -n storage-apis
```

For full details on setting up Contour as a load balancer, configuring DNS records, and enabling TLS, see the
[Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html)
guide.

(storage-navigator-app-deployment-clean-up)=
## Clean Up

```bash
# uninstall the release
helm uninstall storage-navigator -n storage-apis

# delete the namespace
kubectl delete namespace storage-apis

# validate the namespace was deleted
kubectl get namespace storage-apis
```

(storage-navigator-app-deployment-references)=
## References

- [Contour HTTPProxy](https://projectcontour.io/docs/): ingress resource used by the Helm chart
- [Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html): full guide for Contour, DNS, and TLS setup

(storage-navigator-app-deployment-see-next)=
## See next

- {ref}`Configuration <storage-navigator-app-configuration-configuration>`. The runtime `config.json` surface: default discovery, reference links, and content security policy.
