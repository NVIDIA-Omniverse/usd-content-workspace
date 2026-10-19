# Local Kubernetes deployment test

`scripts/test-local-kind.sh` runs the core Discovery Service deployment
lifecycle in an isolated [kind](https://kind.sigs.k8s.io/) cluster.

The test does not use or change the current Kubernetes context. It writes a
temporary kubeconfig, passes its kind context explicitly, and deletes the
cluster when the run finishes.

## Prerequisites

- Docker with a running daemon
- kind
- kubectl
- Helm
- curl
- Python 3
- A production-compatible Discovery Service image in the local Docker image
  store

The default Discovery Service image is `simple-nginx-arm64:latest`, matching
the existing local values. Build or pull that image before running the test, or
select another local image through `DISCOVERY_IMAGE`.

## Run the complete test

From the repository root:

```shell
./scripts/test-local-kind.sh
```

To use another local Discovery Service image:

```shell
DISCOVERY_IMAGE=nvcr.io/nvidia/omniverse/simple-nginx:1.1.6 \
  ./scripts/test-local-kind.sh
```

The image must already be present in Docker. The runner retags it with a stable
local name, sets `imagePullPolicy: Never`, and loads it directly into kind. The
smoke-test and conformance-test images are built from this repository and also
loaded directly, so the cluster does not need registry credentials.

## What the runner verifies

1. Lints both Helm charts and renders default and local manifests.
2. Creates an isolated kind cluster and runs a Kubernetes server-side dry-run.
3. Installs the Discovery Service and waits for its Deployment and Pod.
4. Verifies Helm ownership of the ConfigMap, Service, and Deployment.
5. Checks `/health`, `/metrics`, `/api/v1alpha/services`, and
   `/api/v1/services` through a Service port-forward.
6. Compares both discovery payloads with exact expected internal, explicit,
   path, port, and TLS URL generation.
7. Runs the chart's Helm smoke-test hook.
8. Runs all 14 REST and health tests through the in-cluster conformance Job.
9. Upgrades the service catalog and verifies the config checksum, pod rollout,
   and updated API payload.
10. Uninstalls both releases, waits for Pods to disappear, and checks that no
    release-owned resources remain.

On failure, the runner prints Helm status, resource state, Pod descriptions,
container logs, and namespace events before teardown.

## Useful overrides

- `CLUSTER_NAME=name` selects a fixed kind cluster name. The default includes
  the runner PID to avoid collisions.
- `KEEP_CLUSTER=1` leaves the cluster and temporary kubeconfig in place. The
  runner prints the commands needed to inspect it. Delete it afterward with
  `kind delete cluster --name <name>`.
- `REBUILD_TEST_IMAGES=1` rebuilds the local smoke and conformance images even
  when their tags already exist.
- `LOCAL_PORT=18080` changes the initial Service port-forward port. The upgrade
  check uses the next port.
- `TIMEOUT=5m` changes Kubernetes and Helm wait timeouts.

## Pod Security warnings

The namespace enforces the Pod Security `baseline` profile and warns on
`restricted` violations. The current main Deployment and Helm smoke-test Pod
do not render the security-context values declared in `helm/values.yaml`, so
restricted-profile warnings are expected. The conformance Job runs as non-root
without privilege escalation but also warns because it does not drop all
capabilities or set a seccomp profile. These are visible test findings, not
failures of the baseline deployment test.
