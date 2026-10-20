# Discovery Service

A Kubernetes-native service that automatically discovers gRPC/REST microservices running in a cluster and exposes a unified REST API for querying them.

It watches Kubernetes `Service` objects annotated for discovery, queries each service's capabilities via the `CapabilitiesService` gRPC protocol, and optionally resolves external URLs through Contour `HTTPProxy` resources.

## Architecture

The discovery service is normally deployed inside a Kubernetes cluster alongside the microservices it monitors. It connects to the Kubernetes API server using either in-cluster credentials or an explicit kubeconfig context. For local development and other non-Kubernetes uses, it can instead query a configured set of static gRPC endpoints.

At its core the service maintains the following components:

1. **Kubernetes Watchers** — two long-lived watch streams (one for `Service` objects, one for Contour `HTTPProxy` objects) that receive real-time change notifications from the Kubernetes API. Whenever a Service or HTTPProxy is created, updated, or deleted, the watcher pushes an event into an internal notification channel. The HTTPProxy watcher is best-effort: if the Contour CRD is not installed in the cluster the watcher logs a debug message and the service continues without external URL resolution.

2. **Reconciliation Loop** — the central state-building engine. It is triggered either by a watcher event or by a periodic timer (default every 30 seconds). On each run it lists all Services across the cluster (or within the configured namespaces), filters down to those annotated with `discovery.omniverse.nvidia.com/enabled=true`, and for each qualifying service opens a gRPC connection to call `CapabilitiesService.ListServices()`. The response contains the APIs and versions that the target service implements. The loop also reads all HTTPProxy resources to map service names to externally reachable URLs. A 500 ms debounce window coalesces bursts of events into a single reconciliation pass.

3. **Shared State** — two thread-safe in-memory registries hold the latest internal and external snapshots. The reconciliation loop builds complete new snapshots on every pass and atomically swaps each one only when it changed.

4. **REST API** — two HTTP servers expose the same endpoint, `GET /api/v2beta/services` (also served at the compatibility alias `GET /api/v2/services`): the internal server defaults to port `8080` and serves the internal registry; the external server defaults to port `8081` and serves the external registry.

5. **Static discovery** — when one or more `--static-endpoint` values are supplied, Kubernetes discovery is skipped. The service periodically queries those gRPC endpoints and publishes the resulting capabilities to both registries.

In Kubernetes discovery mode, Services that do not carry the discovery annotation are completely ignored — no gRPC connection is attempted and they never appear in the API response.

## Discovery Loop

The discovery loop is the heart of the service. It follows a controller-style reconciliation pattern similar to Kubernetes operators, where the desired state is continuously derived from the live cluster and compared against the current in-memory state.

### Trigger

Two event sources feed into the loop. The first is the pair of Kubernetes watchers: any change to a `Service` or `HTTPProxy` object sends a notification. The second is a periodic interval timer that fires every `reconcile_interval_secs` seconds (default 30). This timer acts as a safety net to pick up any changes that may have been missed by the watchers, for instance after a temporary loss of the watch connection.

### Debounce

When a notification arrives the loop waits 500 milliseconds and drains any additional events that accumulated during that window. This prevents redundant reconciliation passes when Kubernetes emits a burst of events (e.g. during a rolling deployment that updates many Services simultaneously).

### Fetch Cluster State

The loop queries the Kubernetes API for two resource types in parallel:
- **Services** — all Service objects in the configured scope (one or more namespaces, or cluster-wide).
- **HTTPProxies** — all Contour HTTPProxy objects. If the CRD is not installed or the service account lacks permissions, this step returns an empty list and the reconciliation continues without external URL information.

### Filter

Only Services whose annotations include the discovery-enabled key set to `"true"` proceed to the next step. All other Services are skipped.

### Query Capabilities

For each qualifying Service, the loop determines the gRPC port from the annotation or a Service port named `grpc`. If neither is present, the service is skipped. It then opens a gRPC channel to `CapabilitiesService.ListServices()` and follows `next_page_token` until every page is collected. If any page fails, or the server repeats a token or returns more than 100 pages, the query fails and the previous state for that service is retained.

Two connectivity modes are supported:
- **Direct mode** (default) — the loop connects directly via the cluster-internal DNS name (`{service}.{namespace}.svc.cluster.local:{port}`). This is the most efficient path and works when the discovery service pod has network connectivity to target pods.
- **Port-forward mode** — enabled via `--use-kube-portforward`. The loop selects a running pod behind the Service, establishes a Kubernetes port-forward tunnel through the API server, and runs the gRPC call over that tunnel. This is useful for local development or environments where direct pod-to-pod connectivity is restricted.

If the gRPC call fails for a given service, the loop retains the previously known state for that service (if any) so that a transient failure does not remove a service from the registry.

### Resolve URLs

For each discovered service the loop builds an internal entry when its visibility allows it, using cluster DNS URLs. It independently builds an external entry only when the service has an explicit external endpoint annotation or a matching HTTPProxy route. Routes with protocol `h2` are treated as gRPC endpoints; all others are REST endpoints.

### Update Shared State

The loop compares the newly built service map against the current shared state. If anything changed, the map is swapped atomically and an info-level log line records the number of services. If nothing changed, only a debug-level message is emitted. This diffing keeps logs clean in stable clusters and avoids unnecessary state updates.

### Serve

Independently of the reconciliation loop, each HTTP server handles incoming `GET /api/v2beta/services` requests by reading its registry and returning the current snapshot as JSON. Multiple concurrent API requests are served in parallel without blocking each other or the reconciliation loop.

## URL Resolution

The internal API uses `http://{service}.{namespace}.svc.cluster.local:{port}`. REST requires a Service port named `http` or `rest`; gRPC requires the resolved gRPC port. No default REST or gRPC port is assumed.

For the external API, a target Service can provide `external-rest-endpoint` and/or `external-grpc-endpoint` annotations. Otherwise, the loop scans HTTPProxy routes matching the Service name and port. The virtual-host FQDN becomes an `https` URL when the proxy has TLS, or an `http` URL otherwise. The HTTPProxy `base-path` annotation is appended to REST URLs only.

`rest` and `grpc` are optional in the response. A block is included only when both a URL and at least one corresponding capability are available.

## API

### `GET /api/v2beta/services`

Returns all discovered services with their API capabilities and endpoint URLs. `GET /api/v2/services` is a compatibility alias that returns the identical response.

**Response:**

```json
{
  "schema-version": 2,
  "services": [
    {
      "name": "storage-service-revised",
      "capability_source": "grpc",
      "rest": {
        "url": "https://storage.example.com",
        "endpoints": [
          {
            "name": "fileobject",
            "versions": {
              "v1": { "path": "/api/v1/fileobject" },
              "v1alpha": { "path": "/api/v1alpha/fileobject" }
            }
          }
        ]
      },
      "grpc": {
        "url": "https://grpc.storage.example.com",
        "endpoints": [
          {
            "service": "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
          },
          {
            "service": "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService"
          },
          {
            "service": "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
          },
          {
            "service": "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
          }
        ]
      }
    }
  ]
}
```

The `rest` and `grpc` blocks are omitted when no URL can be determined or when no endpoints are available for that transport. REST version map keys come from `RestVersion.version`; gRPC endpoints are returned as flat fully qualified service names.

### Client Compatibility

- Clients should read `schema-version` and expect schema version `2`.
- Use `rest.endpoints` and each endpoint's `versions` map to enumerate REST APIs.
- Use `grpc.endpoints[*].service` to enumerate gRPC APIs.

For the previous response format and client migration guidance, see [v1 response schema](docs/v1-response-schema.md) and [v1-to-v2 migration](docs/v1-to-v2-migration.md).

## Configuration

All parameters can be set via CLI flags or environment variables.

| CLI Flag | Environment Variable | Default | Description |
|---|---|---|---|
| `--internal-port` | `INTERNAL_PORT` | `8080` | Port for the internal API |
| `--external-port` | `EXTERNAL_PORT` | `8081` | Port for the external API |
| `--kube-namespace` | `KUBE_NAMESPACE` | *(all namespaces)* | Restrict Kubernetes discovery to these namespaces, comma-separated (e.g. `storage-api-dev,usdsearch`). If an annotated Service name exists in more than one, the first-listed namespace wins and a warning is logged |
| `--kube-context` | `KUBE_CONTEXT` | *(default context)* | Kubeconfig context to use |
| `--reconcile-interval-secs` | `RECONCILE_INTERVAL` | `30` | Seconds between reconciliation cycles |
| `--use-kube-portforward` | `USE_KUBE_PORTFORWARD` | `false` | Query a selected backing pod through the Kubernetes API rather than cluster DNS |
| `--static-endpoint` | `STATIC_ENDPOINTS` | *(unset)* | Skip Kubernetes discovery and query `NAME=GRPC_URL` or `NAME=GRPC_URL#REST_URL`; repeat the flag or provide a comma-separated environment value |
| `--discovery-enabled` | `ANNOTATION_DISCOVERY` | `discovery.omniverse.nvidia.com/enabled` | Annotation key that marks a Service as discoverable |
| `--grpc-port` | `ANNOTATION_GRPC_PORT` | `discovery.omniverse.nvidia.com/grpc-port` | Annotation key that specifies the gRPC port on a discovered Service |
| `--base-path` | `ANNOTATION_BASE_PATH` | `discovery.omniverse.nvidia.com/base-path` | Annotation key for the HTTPProxy REST base-path prefix |
| `--visibility` | `ANNOTATION_VISIBILITY` | `discovery.omniverse.nvidia.com/visibility` | Annotation key for service visibility (`internal`, `external`, or `both`) |
| `--external-rest-endpoint` | `ANNOTATION_EXTERNAL_REST_ENDPOINT` | `discovery.omniverse.nvidia.com/external-rest-endpoint` | Annotation key overriding the external REST URL |
| `--external-grpc-endpoint` | `ANNOTATION_EXTERNAL_GRPC_ENDPOINT` | `discovery.omniverse.nvidia.com/external-grpc-endpoint` | Annotation key overriding the external gRPC URL |
| — | `RUST_LOG` | `info` | Log level filter (supports `tracing` directives, such as `discovery_service=debug,hyper=warn`) |

### Service Annotations

Target services opt in to discovery by setting annotations on their Kubernetes `Service` object:

| Annotation                                              | Required | Description                                                                                                        |
|---------------------------------------------------------|----------|--------------------------------------------------------------------------------------------------------------------|
| `discovery.omniverse.nvidia.com/enabled`                | Yes      | Set to `"true"` to make the service discoverable                                                                   |
| `discovery.omniverse.nvidia.com/grpc-port`              | No       | Override the gRPC port; a port named `grpc` is required when this annotation is absent                              |
| `discovery.omniverse.nvidia.com/visibility`             | No       | Controls which API exposes the service: `internal`, `external`, or `both` (default `both`)                          |
| `discovery.omniverse.nvidia.com/external-rest-endpoint` | No       | Override the external REST endpoint URL (takes precedence over HTTPProxy-derived URL)                              |
| `discovery.omniverse.nvidia.com/external-grpc-endpoint` | No       | Override the external gRPC endpoint URL (takes precedence over HTTPProxy-derived URL)                              |

Contour `HTTPProxy` resources also support:

| Annotation                                    | Required | Description                                                                          |
|-----------------------------------------------|----------|--------------------------------------------------------------------------------------|
| `discovery.omniverse.nvidia.com/base-path`    | No       | Base path prefix appended to the virtualhost URL when building endpoint URLs         |

### gRPC Port Resolution Order

1. Value from the `grpc-port` annotation
2. Port named `grpc` in the Service spec
3. Otherwise, the service is skipped

### REST Port Resolution Order

1. Port named `http` or `rest` in the Service spec
2. Otherwise, no REST URL is published

## Building

Requires Rust 1.97.0 and the protobuf compiler (`protoc`).

The build compiles the protobuf sources in `discovery-protos/`. If that directory
is empty, it is an uninitialized git submodule; populate it first:

```bash
git submodule update --init
```

When `discovery-protos/` already contains the protobuf sources, skip this step.
Run the build from the service's root directory:

```bash
cargo build --release --locked
```

The `build.rs` script compiles the v2beta capability protos from `discovery-protos/proto` during the build. The service queries `CapabilitiesService.ListServices` on v2beta and, if a service returns `UNIMPLEMENTED`, falls back to the deprecated v2alpha API and logs a deprecation warning. The v2alpha protos are compiled only so tests can exercise that fallback.

## Running Locally

```bash
# Uses default kubeconfig
./target/release/auto-discovery-service

# With specific context and namespace
./target/release/auto-discovery-service \
  --kube-context my-context \
  --kube-namespace my-namespace \
  --use-kube-portforward

# Via environment variables
KUBE_NAMESPACE=default RECONCILE_INTERVAL=10 RUST_LOG=debug \
  ./target/release/auto-discovery-service

# Without Kubernetes, query a local capabilities server
./target/release/auto-discovery-service \
  --static-endpoint mock=http://localhost:50051#http://localhost:8082
```

The service starts the internal API on port `8080` and the external API on port `8081` by default. In Kubernetes mode it begins watching the cluster immediately; static mode periodically re-queries the configured endpoints. Query either API with:

```bash
curl http://localhost:8080/api/v2beta/services
curl http://localhost:8081/api/v2beta/services
```

## Docker

```bash
docker build -t auto-discovery-service .
docker run -p 8080:8080 -p 8081:8081 \
  -e KUBECONFIG=/kube/config \
  -v ~/.kube/config:/kube/config:ro \
  auto-discovery-service
```

The image exposes ports `8080` and `8081` and runs as a non-root user. To use a host kubeconfig, ensure its credentials and server address are reachable from the container.

## Deploying with Helm

A Helm chart is provided in `helm/auto-discovery-service/`. It creates a Service with internal and external ports. By default, discovery watches only the namespace where the Pod runs; set `config.kubeNamespace` to explicitly select other namespaces, as a comma-separated string or a YAML list (the first-listed namespace wins a Service-name collision). The chart currently creates a ClusterRole granting permission to list, get, and watch `Service` and Contour `HTTPProxy` resources.

```bash
helm install auto-discovery-service ./helm/auto-discovery-service \
  --set config.kubeNamespace=my-namespace \
  --set config.reconcileInterval=15
```

Set `rbac.create=false` only when the selected service account already has the required permissions. Set `httpProxy.enabled=true` to expose the discovery service itself through Contour.

### One FQDN for v1 and v2

A single root `HTTPProxy` can route legacy paths to an ordinary in-cluster
Service while its catch-all continues to serve v2. This does not require
Contour's `enableExternalNameService` option:

```yaml
httpProxy:
  enabled: true
  endpoints:
    - name: external
      fqdn: discovery.example.com
      port: 8081
      additionalRoutes:
        - exact: /api/v1/services
          service:
            name: auto-discovery-service
            port: 8080
        - exact: /api/v1alpha/services
          service:
            name: auto-discovery-service
            port: 8080
```

Deploy the legacy Service in the same namespace and disable its own root
`HTTPProxy`; the operator chart must own the only root proxy for this FQDN.
Each additional route accepts exactly one of `exact`, `prefix`, or `regex` and
is rendered before the operator's `/` catch-all. The request path is preserved,
which matches the legacy discovery service's existing nginx locations.

A repeatable Docker Desktop setup with a mock legacy deployment, pinned Contour,
and end-to-end assertions is available in [`deploy/local/`](deploy/local/README.md).

The chart's default ClusterRole does not grant the Pod or `pods/portforward` permissions required by `--use-kube-portforward`; grant those permissions separately before enabling that mode in a Helm deployment.

## License generation

To generate `ATTRIBUTIONS.md`, run the following commands:

1. `cargo install dd-rust-license-tool`
2. `dd-rust-license-tool write`
3. `python tools/generate_attributions.py` (Python 3.10+ is required)

## Customer Conformance Checks

The optional [conformance runner](conformance/README.md) validates an existing
Discovery catalog and explicitly selected backend REST/gRPC Capabilities APIs.
It provides human-readable, JSON and JUnit reports without modifying deployments.
