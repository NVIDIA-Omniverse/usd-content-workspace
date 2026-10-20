# Package and deploy a storage adapter

> **Applies to:** turning a custom storage adapter (any path) into a running, discoverable, authenticated Kubernetes service · **Released:** Auto Discovery **1.1.1**, legacy Discovery chart **2.3.11** (NGC inventory); Storage Service **1.2.3** for comparison · **This repo's snapshot:** filesystem reference from unreleased Storage API source (after 1.0.0-beta.4); Auto Discovery operator snapshot and legacy chart snapshot per [components/discovery.md](../../components/discovery.md) · **Sources:** [`filesystem_example/Dockerfile`](../../../../../storage/api/filesystem_example/Dockerfile), [`filesystem_example/README.md`](../../../../../storage/api/filesystem_example/README.md), discovery operator [`README.md`](../../../../../discovery/reference/discovery-service/operator/README.md), thumbnail and storage-service charts as convention references, the [source skill package](../../../NOTICE.md)

The deployment walkthroughs for the stack are owned elsewhere: the filesystem example on a local cluster in [deploy/local-development.md](../../deploy/local-development.md), the production stack in [deploy/production-stack.md](../../deploy/production-stack.md), Discovery in [deploy/discovery.md](../../deploy/discovery.md). This page covers what is specific to an adapter you wrote. General conventions for any new service (Helm layout, auth, observability) are in [develop/add-your-service.md](../add-your-service.md).

## 1. Build an image

For the Python reference or a Path A backend:

```bash
cd storage/api/filesystem_example
.poetry_venv/bin/poetry build                       # wheel into dist/
docker build -f Dockerfile -t <registry>/<repo>/my-storage-adapter:<tag> .
```

The shipped [`Dockerfile`](../../../../../storage/api/filesystem_example/Dockerfile) is minimal: `python:3.10-slim`, `pip install` of the wheel, `ENTRYPOINT ["local-filesystem-service"]`, `CMD []`. With no subcommand the service starts the `filesystem` backend from environment variables, so `FILESERVICE_STATIC_DIR` and friends apply. (The README says the default command "includes the backend subcommand"; it does not, but the env-driven default has the same effect since 1.0.0-beta.4.) For your own backend, set `CMD ["mystorage"]` or pass args in the Pod spec.

Harden it before using it outside a laptop:

- Run as a non-root numeric UID (`USER 1000`) so `runAsNonRoot` can be verified at admission, matching the stack's charts.
- Pin the base image by digest and rebuild for security updates.
- Write only to the data mount; keep the root filesystem read-only where the runtime allows.
- Do not bake credentials into the image or the wheel.

Compiled adapters (Path B, Path C) follow their language's practice; the stack's Rust services use multi-stage builds onto distroless images (for example the Storage Service [`Dockerfile`](../../../../../storage/reference/storage-service/Dockerfile)). Image build constraints for the components in this repo are in [deploy/building-from-source.md](../../deploy/building-from-source.md).

## 2. Kubernetes resources

Minimum: a Deployment, a ClusterIP Service whose ports are **named** `grpc` and `http` (or `rest`), persistent storage if the backend is local, and a ServiceAccount. Example for the Python reference (adjust ports, args and storage for your adapter):

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata: { name: my-storage-adapter-data }
spec:
  accessModes: [ReadWriteOnce]
  resources: { requests: { storage: 10Gi } }
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: my-storage-adapter }
spec:
  replicas: 1                      # local-filesystem backend: one writer per volume
  selector: { matchLabels: { app: my-storage-adapter } }
  template:
    metadata: { labels: { app: my-storage-adapter } }
    spec:
      securityContext: { runAsNonRoot: true, runAsUser: 1000, fsGroup: 1000 }
      containers:
        - name: adapter
          image: <registry>/<repo>/my-storage-adapter:<tag>
          ports:
            - { name: grpc, containerPort: 50051 }
            - { name: http, containerPort: 8011 }
          env:
            - { name: FILESERVICE_STATIC_DIR, value: /data }
            - { name: FILESERVICE_SERVER_BASE_URI, value: "file-storage://fileservice" }
            - { name: REDIRECT_HOST, value: "https://<client-reachable-host>" }
            - { name: REDIRECT_PORT, value: "443" }
          securityContext: { allowPrivilegeEscalation: false, capabilities: { drop: [ALL] } }
          volumeMounts: [{ name: data, mountPath: /data }]
      volumes:
        - name: data
          persistentVolumeClaim: { claimName: my-storage-adapter-data }
---
apiVersion: v1
kind: Service
metadata:
  name: my-storage-adapter
  annotations:
    discovery.omniverse.nvidia.com/enabled: "true"   # only once the adapter serves discovery v2alpha ListServices
spec:
  type: ClusterIP
  selector: { app: my-storage-adapter }
  ports:
    - { name: grpc, port: 50051, targetPort: grpc }
    - { name: http, port: 8011, targetPort: http }
```

The `runAsUser: 1000` line assumes you added that user to the image; the shipped Dockerfile runs as root.

Check that the configured root really is the mounted volume before writing data. Otherwise objects land in the container's ephemeral filesystem and vanish on restart:

```bash
kubectl -n <namespace> exec deploy/my-storage-adapter -- python -c \
  "import os;print(os.environ['FILESERVICE_STATIC_DIR'], os.path.ismount('/data'))"
```

`REDIRECT_HOST`/`REDIRECT_PORT` are written into redirect and multipart URLs that clients call directly. The defaults (`http://localhost:8011`) only work for a client on the same host. Point them at the TLS ingress host, and remember the reference serves the content directory unauthenticated under `/download`, so its REST port must never be reachable without the ingress auth in front.

Probes: the Python reference has no health endpoint; use a TCP probe on the gRPC port or `GET /v1beta/capabilities/services`. Compiled adapters should serve the standard gRPC health service (the Storage Service uses a separate probe port, `8100`).

## 3. Expose it safely

- Keep the Service `ClusterIP`. Expose REST and gRPC through the stack's Contour `HTTPProxy` with TLS, REST at `<host>.<domain>` and gRPC (`h2c` upstream) at `grpc.<host>.<domain>`, the convention used by the stack's charts ([deploy/production-stack.md](../../deploy/production-stack.md)).
- Put the Envoy auth extension in front so every request carries a validated bearer token, and authorize operations with the Permission Service ([deploy/permissions-and-auth.md](../../deploy/permissions-and-auth.md), [develop/add-your-service.md](../add-your-service.md#authentication-and-authorization)).
- Add a NetworkPolicy so only the ingress, Discovery and in-namespace callers reach the pod ports.
- Backend credentials come from a Kubernetes Secret reference or workload identity, never from values files or images.

## 4. Register with Discovery

| Discovery deployment | What your adapter needs |
|---|---|
| **Auto Discovery operator** (released 1.1.1; snapshot operator) | Serve `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` on the gRPC port (the snapshot operator tries v2beta first and falls back to v2alpha); annotate the Service `discovery.omniverse.nvidia.com/enabled: "true"`; name ports `grpc` and `http`/`rest` (or set `discovery.omniverse.nvidia.com/grpc-port`). Optional: `visibility`, `external-rest-endpoint`, `external-grpc-endpoint`; external URLs otherwise come from a matching HTTPProxy |
| Operator in static mode (`--static-endpoint NAME=GRPC_URL[#REST_URL]`) | Still calls `ListServices` on the given endpoint (v2beta, then v2alpha, in the snapshot) |
| **Legacy Discovery chart** (static nginx catalog) | An entry in the chart's catalog values; no RPC needed. Its values schema differs from the operator's |

The Python reference and Path A backends implement neither the v2alpha nor the v2beta API, so the operator ignores them even with the annotation. Either add the API ([develop/add-your-service.md](../add-your-service.md#expose-discovery-capabilities)) or use the legacy static catalog. Installing the Helm release alone does not prove catalog membership; check:

```bash
kubectl -n <discovery-namespace> port-forward svc/<auto-discovery-service> 8080:8080
curl -s http://localhost:8080/api/v2/services | python3 -m json.tool
```

The operator's internal API listens on `8080` and the external API on `8081` by default; the Service name depends on your Discovery release.

## 5. Validate in the cluster

1. Run the Storage API conformance suite against the in-cluster adapter through a port-forward, with a disposable resource base ([test/conformance.md](../../test/conformance.md#run-against-a-deployed-adapter)).
2. Confirm the adapter appears in the Discovery catalog with the REST and gRPC URLs clients will use from their network.
3. With authentication on, confirm a read and a write with the intended identity succeed, and a write with an identity lacking permission fails with 403/`PERMISSION_DENIED`.
4. If the stack runs the released integration tests, point them at the adapter through Discovery or `manualEndpoints` and record executed, skipped and deselected counts ([test/integration-tests.md](../../test/integration-tests.md)).
