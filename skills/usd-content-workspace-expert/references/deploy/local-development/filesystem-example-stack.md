# Filesystem example stack (learning and adapter development)

> **Applies to:** Python filesystem reference service, legacy Discovery chart, Kit · **Released:** example inside NGC `storage-api` resource 1.0.0-beta.4; legacy `discovery-service` chart 2.3.11 with `simple-nginx` 1.1.0 · **This repo's snapshot:** filesystem example, unreleased (`pyproject.toml`); legacy chart 2.4.1, unreleased (`VERSION.toml`) · **Sources:** the [source skill package](../../../NOTICE.md); [`storage/api/filesystem_example/`](../../../../../storage/api/filesystem_example/README.md); [`discovery/reference/discovery-service/chart/`](../../../../../discovery/reference/discovery-service/chart/README.md)

The filesystem example is a Python reference implementation of the Storage API. It stores files on a local directory with content and version trees and serves REST and gRPC from one process. It is for learning, client development, and as a model for custom adapters ([../../develop/custom-storage-adapter/README.md](../../develop/custom-storage-adapter/README.md)). It has no authentication and is not a production backend.

Confirm which example version you run: the NGC resource and this repository's snapshot differ (the snapshot adds, among others, the `FILESERVICE_VERSIONING_MODE` and `FILESERVICE_READ_ONLY` settings). Read the README that ships with the copy you use.

## Prerequisites

- Python 3.10, 3.11, or 3.12, and Poetry (installed into a local virtualenv below).
- Docker (to containerize).
- Any conformant Kubernetes cluster for the cluster steps. MicroK8s specifics are in [microk8s-and-ingress.md](microk8s-and-ingress.md).
- Helm 3 for the Discovery chart.

## Get the source

From this repository: `storage/api/filesystem_example/`. The generated Python stubs under `src/nvidia/` are committed here, so no protobuf generation is needed before `poetry install`.

From NGC (released resource): download `nvidia/omniverse/storage-api:<version>` as in [../artifact-access.md](../artifact-access.md#download-charts-and-api-resources), unzip, and use its `filesystem_example/` directory.

## Run locally

```bash
cd storage/api/filesystem_example
python -m venv .poetry_venv
.poetry_venv/bin/pip install poetry
.poetry_venv/bin/poetry install
.poetry_venv/bin/poetry run local-filesystem-service
```

The log shows the gRPC server on 50051 and Uvicorn on `http://0.0.0.0:8011`.

Entry points ([`pyproject.toml`](../../../../../storage/api/filesystem_example/pyproject.toml) `[tool.poetry.scripts]`): `local-filesystem-service` (gRPC + REST), `local-filesystem-grpc`, `local-filesystem-rest`. Each takes `[COMMON_OPTIONS] filesystem [BACKEND_OPTIONS]`.

| Option | Env var | Default |
| --- | --- | --- |
| `--grpc-port` | `GRPC_SERVER_PORT` | 50051 |
| `--http-port` | `HTTP_SERVER_PORT` | 8011 |
| `--reload` (REST only) | — | off |
| `filesystem --base-uri` | `FILESERVICE_SERVER_BASE_URI` | `file-storage://fileservice` |
| `filesystem --static-dir` | `FILESERVICE_STATIC_DIR` | system temp directory |
| `filesystem --folder-mode` | `FILESERVICE_TEST_FOLDER_MODE` | `native` (`native`, `no_empty`, `placeholder`) |
| `filesystem --redirect-host` | `REDIRECT_HOST` | `http://localhost` |
| `filesystem --redirect-port` | `REDIRECT_PORT` | 8011 |
| `filesystem --native-metadata-mode` | `FILESERVICE_NATIVE_METADATA_MODE` | `enabled` |
| — | `FILESERVICE_VERSIONING_MODE`, `FILESERVICE_READ_ONLY` | see `local-filesystem-service filesystem --help` |

CLI options override environment variables. In this snapshot the no-subcommand start path also reads these environment variables ([`__main__.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/__main__.py) `_build_default_filesystem_config`). The 1.0.0 GA release notes recorded a known issue where `FILESERVICE_STATIC_DIR` was ignored in Docker/Kubernetes; if you run an older image, pass `filesystem --static-dir /data/storage` explicitly.

Every resource address starts with the configured base URI. With the default it is `file-storage://fileservice/...`. Percent-encode the address in REST paths.

```bash
curl http://localhost:8011/v1beta/capabilities/services

payload='Hello World'
size=$(printf '%s' "${payload}" | wc -c)
curl -X PUT "http://localhost:8011/v1beta/fileobject/by-address/file-storage%3A%2F%2Ffileservice%2Ftest.txt?data_object_size=${size}" \
  -H "Content-Type: application/octet-stream" --data-binary "${payload}"
curl http://localhost:8011/v1beta/fileobject/by-address/file-storage%3A%2F%2Ffileservice%2Ftest.txt

# OpenAPI pages
open http://localhost:8011/v1beta/fileobject/docs
```

The source skill package walkthrough switches to `file-storage://storage-service/...` addresses in its later steps. Those only work if you set `--base-uri file-storage://storage-service`; keep one base URI throughout.

gRPC reflection is enabled, so `grpcurl -plaintext localhost:50051 list` shows the registered services.

## Containerize

```bash
.poetry_venv/bin/poetry build          # wheel into dist/
docker build -f Dockerfile -t storageapi_localfilesystem_service:local .
mkdir -p ~/storage-data
docker run --pull=never -d -p 127.0.0.1:8011:8011 -p 127.0.0.1:50051:50051 \
  -v ~/storage-data:/data/storage -e FILESERVICE_STATIC_DIR=/data/storage \
  --name localfilesystem-test storageapi_localfilesystem_service:local filesystem
docker logs localfilesystem-test
```

The repository [`Dockerfile`](../../../../../storage/api/filesystem_example/Dockerfile) is `python:3.10-slim`, installs the wheel from `dist/`, and sets `ENTRYPOINT ["local-filesystem-service"]` with `CMD []`. The README says the default command includes `filesystem`; it does not, so pass `filesystem` explicitly (as above) when you rely on backend settings. Bind published ports to `127.0.0.1`; the service has no authentication.

## Deploy to Kubernetes

There is no Helm chart for the example. Plain manifests, in namespace `<ns>`:

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata: { name: storage-pvc, namespace: <ns> }
spec:
  accessModes: [ReadWriteOnce]
  resources: { requests: { storage: 10Gi } }
  # storageClassName: microk8s-hostpath   # MicroK8s; use your cluster's class
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: storage-service, namespace: <ns>, labels: { app: storage-service } }
spec:
  replicas: 1
  selector: { matchLabels: { app: storage-service } }
  template:
    metadata: { labels: { app: storage-service } }
    spec:
      securityContext: { runAsNonRoot: true, runAsUser: 1000, fsGroup: 1000 }
      containers:
        - name: storage
          image: storageapi_localfilesystem_service:local
          imagePullPolicy: Never          # local image; use your registry otherwise
          args: ["filesystem"]
          ports:
            - { name: http, containerPort: 8011 }
            - { name: grpc, containerPort: 50051 }
          env:
            - { name: FILESERVICE_STATIC_DIR, value: /data/storage }
            - { name: FILESERVICE_SERVER_BASE_URI, value: "file-storage://fileservice" }
            - { name: REDIRECT_HOST, value: "http://localhost" }
            - { name: REDIRECT_PORT, value: "8011" }
          securityContext: { allowPrivilegeEscalation: false, capabilities: { drop: [ALL] } }
          volumeMounts: [{ name: storage-data, mountPath: /data/storage }]
      volumes:
        - name: storage-data
          persistentVolumeClaim: { claimName: storage-pvc }
---
apiVersion: v1
kind: Service
metadata: { name: storage-service, namespace: <ns>, labels: { app: storage-service } }
spec:
  type: ClusterIP
  selector: { app: storage-service }
  ports:
    - { name: http, port: 8011, targetPort: 8011 }
    - { name: grpc, port: 50051, targetPort: 50051 }
```

`REDIRECT_HOST`/`REDIRECT_PORT` form the URLs returned for redirected transfers; they must be reachable by the client (for port-forwarding, `http://localhost:8011`). The `python:3.10-slim` image runs as root by default; the pod `securityContext` above runs it as UID 1000, so the volume must be writable by that UID (`fsGroup`). Remove those lines only if your storage class cannot honor `fsGroup`.

```bash
kubectl apply -f storage-service.yaml
kubectl -n <ns> rollout status deployment/storage-service
kubectl -n <ns> get endpoints storage-service
kubectl -n <ns> port-forward service/storage-service 8011:8011
```

Repeat the `curl` checks from [Run locally](#run-locally) against `localhost:8011`.

## Add Discovery

As explained in [../local-development.md](../local-development.md#discovery-choice-for-the-example-stack), Auto Discovery cannot register this snapshot's example. Use the legacy static chart from [`discovery/reference/discovery-service/chart/helm`](../../../../../discovery/reference/discovery-service/chart/helm/values.yaml):

```yaml
# discovery-values.yaml
discovery:
  services:
    - id: "storage-service-01"
      name: "Filesystem example"
      type: "storage"
      endpoints:
        grpc: { host: "localhost", port: "8012", path: "", tls: false }   # client-reachable address
        rest: { host: "localhost", port: "8011", path: "", tls: false }
```

```bash
cd discovery/reference/discovery-service/chart/helm
helm upgrade --install discovery-service . -n <ns> -f discovery-values.yaml
kubectl -n <ns> port-forward service/discovery-service 8080:8080     # check the rendered Service name
curl http://localhost:8080/api/v1/services
```

Notes, from the chart templates:

- The values `discovery.services` list replaces the chart's default list, so list only services you deploy. Each entry uses `endpoints.grpc` and `endpoints.rest` objects with `host`, `port`, `path`, `tls`. The chart README's example with flat `grpc:`/`rest:` URL strings does not match the template.
- Leaving `host` empty uses the built-in default for the `type` (for `storage`, `storage-service.<ns>.svc.cluster.local` with ports 8011/8012 — the production adapter's ports, not the example's). Set host and port explicitly for the example.
- URLs are emitted with `http://` or `https://` schemes (`tls` or ingress TLS selects https). The nginx listens on 8080 regardless of `service.ports.http`.
- In the snapshot the default image is `nginx:1.31`; the released 2.3.11 chart is used with `simple-nginx` 1.1.0. Check `helm show values` on the archive you install.
- The catalog advertises addresses to clients; use addresses reachable from them (here, the local port-forwards), not cluster DNS, when clients run outside the cluster.

Port-forward the example's gRPC port to the host port you advertised: `kubectl -n <ns> port-forward service/storage-service 8012:50051`.

End to end:

```bash
STORAGE_URL=$(curl -s http://localhost:8080/api/v1/services | jq -r '.services[] | select(.type=="storage") | .rest')
curl "${STORAGE_URL%/}/v1beta/fileobject/by-address/file-storage%3A%2F%2Ffileservice%2Ftest.txt"
```

## Test with Kit

Kit 109.0.1 or later ([Linux](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/kit-sdk-linux), [Windows](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/kit-sdk-windows)). Set the Discovery **base URL** (no `/api/v1/services`) before launch, for example by adding to the launch script before the `${EXEC:-exec}` line:

```bash
export OMNI_STORAGE_DISCOVERY=http://localhost:8080
```

Upload a `.usda` file with the REST `PUT` above (use `upload_preference=body` for small text files), then **File > Open** `file-storage://fileservice/cube.usda` (no percent-encoding). The source skill package records that Kit 109.0.1 cannot add this connection in the Content Browser; use File > Open. Which Discovery catalog versions a Kit build reads depends on its Client Library; check `omni.client.get_version()` and see [../../components/client-library.md](../../components/client-library.md) and [../../use/usd-workflows.md](../../use/usd-workflows.md).

## Next

- MicroK8s and ingress instead of port-forwards: [microk8s-and-ingress.md](microk8s-and-ingress.md).
- Run the Storage API conformance suite against this service: [../../test/conformance.md](../../test/conformance.md).
- Move to S3/Azure: [../production-stack.md](../production-stack.md).
