# Local single-FQDN routing test

This environment verifies the integrated v1/v2 endpoints through Contour on
Docker Desktop:

- `Host: discovery.local`, `/api/v1/services` → the operator's v1 compatibility response
- `Host: discovery.local`, `/api/v1alpha/services` → the operator's v1 compatibility response
- `Host: discovery.local`, `/api/v2beta/services` and its alias `/api/v2/services` → the operator's v2beta catalog response
- `Host: discovery.local`, `/api/v1/auth-config`, `/api/v1alpha/auth-config`, `/api/v2/auth-config`, and `/api/v2beta/auth-config` → the operator's auth bootstrap response

The operator chart owns the root `HTTPProxy`, whose catch-all route sends all API
versions to the external port. `mock-capabilities` supplies discovery data used
by both the v1 compatibility and v2 responses.

## Prerequisites

- Docker Desktop Kubernetes enabled
- a kubectl context named `docker-desktop` (it need not be the current context)
- `docker`, `kubectl`, Helm, `curl`, and `jq`
- local build access to the base images used by `Dockerfile` and `Dockerfile.mock`

The pinned `contour-v1.33.5.yaml` is based on Contour's official rendered
manifest (SHA-256 before the local patch:
`4f38f923593621365a489b7ed31e45f33559fe75853a31428b917fbc80a6515a`).
It is patched to use a `ClusterIP` Envoy Service, remove host ports, and use the
standard `envoy:v1.35.10` image. Contour's
`enableExternalNameService` setting remains at its secure default, `false`.

## Run

```bash
# Build images, install Contour if absent, deploy, and run assertions:
./deploy/local/install.sh

# Re-run only HTTP assertions:
./deploy/local/test.sh

# Keep Envoy reachable for manual browser/curl testing (run in a terminal):
./deploy/local/forward.sh
# Then use http://discovery.local:18080/api/v1/services, /api/v2beta/services, /api/v2/services,
# or /api/{v1,v1alpha,v2,v2beta}/auth-config.
# The script prints curl --resolve examples that do not require /etc/hosts.

# Skip image builds when auto-discovery-service:local and
# mock-capabilities:local already exist:
BUILD_IMAGES=false ./deploy/local/install.sh

# Remove the test namespace; keep Contour for subsequent runs:
./deploy/local/cleanup.sh

# Also remove the locally installed Contour resources:
REMOVE_CONTOUR=true ./deploy/local/cleanup.sh
```

Tests use `kubectl port-forward service/envoy 18080:80` and send the
`Host: discovery.local` header, so no `/etc/hosts` entry or privileged host port
is required. `test.sh` closes its temporary port-forward when the assertions
finish. Use `forward.sh` when you need the endpoints to remain reachable for
manual testing. Override `LOCAL_PORT` if port 18080 is busy.

To validate just the chart's route rendering and invalid-value handling:

```bash
./deploy/local/test-helm.sh
```
