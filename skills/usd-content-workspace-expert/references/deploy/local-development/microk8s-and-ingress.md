# MicroK8s, local ingress, troubleshooting, cleanup

> **Applies to:** local clusters running the [filesystem example stack](filesystem-example-stack.md) or a local Auto Discovery test · **Released:** not version-specific · **This repo's snapshot:** operator local test environment in [`deploy/local/`](../../../../../discovery/reference/discovery-service/operator/deploy/local/README.md) (Docker Desktop, Contour v1.33.5) · **Sources:** the [source skill package](../../../NOTICE.md); MicroK8s and ingress-nginx behavior is external and not pinned by this repository

Everything here is optional. Any conformant cluster works; MicroK8s is one concrete local option. For other clusters, replace `microk8s kubectl` with `kubectl` and skip the MicroK8s steps.

## Install MicroK8s

```bash
sudo snap install microk8s --classic
sudo usermod -a -G microk8s "$USER"
mkdir -p ~/.kube && chmod 0700 ~/.kube
su - "$USER"                       # re-enter the session for the group change

microk8s start
microk8s enable dns
microk8s enable hostpath-storage
microk8s enable registry           # optional local registry
microk8s enable ingress            # only if you use ingress below
microk8s status --wait-ready
```

Other install methods: [MicroK8s install alternatives](https://microk8s.io/docs/install-alternatives). New to Kubernetes: [Install a local Kubernetes with MicroK8s](https://ubuntu.com/tutorials/install-a-local-kubernetes-with-microk8s#1-overview).

## Import a locally built image

```bash
docker save storageapi_localfilesystem_service:local -o storageapi_localfilesystem_service.tar
microk8s ctr image import ./storageapi_localfilesystem_service.tar
microk8s ctr image ls | grep storageapi_localfilesystem_service
```

Use `imagePullPolicy: Never` for imported images.

## Hostpath storage

Set `storageClassName: microk8s-hostpath` on the PVC. Data lands under `/var/snap/microk8s/common/default-storage/<namespace>-<pvc-name>-<pvc-uid>/` on the host (inspect with `ls`; the exact directory name is assigned by the provisioner).

## Ingress without port-forwarding (MicroK8s NGINX)

Two mechanisms are needed: HTTP ingress for Discovery, and TCP pass-through for the example's REST (8011) and gRPC (50051) ports.

Check the ingress class name first; it varies by MicroK8s version:

```bash
microk8s kubectl get ingressclass       # often "public" or "nginx"
```

HTTP ingress for the legacy Discovery chart (service port 8080):

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: discovery-ingress
  namespace: <ns>
spec:
  ingressClassName: public               # use the class shown above
  rules:
    - host: discovery.local
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service: { name: discovery-service, port: { number: 8080 } }
```

For the Auto Discovery operator instead, route to its **external** port 8081 and service name `auto-discovery-service` (or the rendered name).

TCP pass-through for Storage:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: nginx-ingress-tcp-microk8s-conf
  namespace: ingress
data:
  "8011": "<ns>/storage-service:8011"
  "50051": "<ns>/storage-service:50051"
```

Then patch the ingress controller DaemonSet to expose host ports 8011 and 50051 and add `--tcp-services-configmap=ingress/nginx-ingress-tcp-microk8s-conf` to its args. Read the existing args first and include them all in the patch; a strategic-merge patch of `args` replaces the list:

```bash
kubectl get daemonset -n ingress
kubectl get daemonset nginx-ingress-microk8s-controller -n ingress \
  -o jsonpath='{.spec.template.spec.containers[0].args}' | jq .
```

```yaml
# daemonset-patch.yaml (names vary by MicroK8s version; check them)
spec:
  template:
    spec:
      containers:
        - name: nginx-ingress-microk8s
          ports:
            - { containerPort: 8011, hostPort: 8011, protocol: TCP }
            - { containerPort: 50051, hostPort: 50051, protocol: TCP }
          args:
            # ...every existing arg from the output above...
            - --tcp-services-configmap=ingress/nginx-ingress-tcp-microk8s-conf
```

```bash
kubectl patch daemonset nginx-ingress-microk8s-controller -n ingress --type strategic --patch-file daemonset-patch.yaml
```

Host ports bind on every interface of the node. On a shared machine, firewall them or keep using port-forwards; the example has no authentication.

Add `127.0.0.1 discovery.local storage.local` to `/etc/hosts`, update the Discovery entry's advertised host/port to the ingress addresses (`storage.local:8011` REST, `storage.local:50051` gRPC), upgrade the Discovery release, and validate:

```bash
curl http://discovery.local/api/v1/services
curl http://storage.local:8011/v1beta/capabilities/services
```

For the operator, external addresses come from Service annotations (`discovery.omniverse.nvidia.com/external-rest-endpoint`, `.../external-grpc-endpoint`) or matching Contour HTTPProxy routes, not from a values list; see [../discovery.md](../discovery.md).

## Local Contour test environment for the operator

The operator ships a repeatable Docker Desktop environment that installs a pinned Contour v1.33.5 manifest (ClusterIP Envoy, no host ports), a mock capabilities server, and the operator chart, then asserts v1, v2, and auth-config routes through one HTTPProxy:

```bash
cd discovery/reference/discovery-service/operator
./deploy/local/install.sh      # builds images, installs Contour if absent, deploys, asserts
./deploy/local/test.sh         # re-run HTTP assertions
./deploy/local/forward.sh      # keep http://discovery.local:18080 reachable
./deploy/local/cleanup.sh      # REMOVE_CONTOUR=true also removes Contour
```

It requires a kube context named `docker-desktop`. See [`deploy/local/README.md`](../../../../../discovery/reference/discovery-service/operator/deploy/local/README.md).

## Troubleshooting, in order

1. Pods: `kubectl get pods -n <ns>`; `kubectl describe pod -n <ns> -l app=storage-service`.
2. Services and endpoints: `kubectl get svc,endpoints -n <ns>`. No endpoints means a selector or readiness problem.
3. Port-forwards still running: `ps -ef | grep "kubectl port-forward"`. Restart them in their terminals.
4. Verbose requests: `curl -v http://localhost:8080/api/v1/services`.
5. Logs: `kubectl logs deployment/<name> -n <ns>`.
6. Port in use: pick another local port, or identify the owning process before stopping it. Do not kill unidentified processes.
7. MicroK8s x509 errors: inspect the failing certificate and expected host first. `sudo microk8s refresh-certs --cert server.crt` followed by a MicroK8s restart is disruptive; run it only for a diagnosed certificate problem and in a window where restarting the cluster is acceptable.

## Cleanup

Remove only what you created, after confirming context, namespace, and release names:

```bash
helm list -n <ns>
helm uninstall <release-name> -n <ns>
kubectl delete -f storage-service.yaml        # the reviewed manifest you applied
```

Check PVC reclaim behavior before deleting PVCs. Do not delete the namespace unless it is a confirmed disposable namespace and the data in it is not needed.
