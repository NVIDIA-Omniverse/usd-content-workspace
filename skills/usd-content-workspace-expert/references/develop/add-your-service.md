# Add your own service to the stack

> **Applies to:** a new microservice (your own contract, or a new implementation of an existing contract) that should run beside the Omniverse content services, be found through Discovery, use the stack's authentication and authorization, and be validated the same way · **Released:** Auto Discovery **1.1.1**, Envoy auth extension **2.4.0**, Permission Service **0.0.53** (NGC inventory) · **This repo's snapshot:** discovery capabilities `v2alpha` proto ([`discovery/api/`](../../../../discovery/api/), untagged `origin/main`); conventions read from the Storage, Thumbnail, Permission, User Info and notification charts in this snapshot; observability crates **0.14.0** + `[Unreleased]` · **Sources:** [`discovery/api/README.md`](../../../../discovery/api/README.md), [`discovery/reference/discovery-service/operator/README.md`](../../../../discovery/reference/discovery-service/operator/README.md), [`thumbnails/reference/thumbnail-service/helm/thumbnail-service/templates/service.yaml`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/templates/service.yaml), [`thumbnails/reference/thumbnail-service/crates/service/src/grpc/capabilities.rs`](../../../../thumbnails/reference/thumbnail-service/crates/service/src/grpc/capabilities.rs), Storage Service `backend/permissions/` and [`docs/helm/storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst), [`common/observability/`](../../../../common/observability/)

A service is "in the stack" when it has a versioned contract, answers the Discovery capabilities RPC, sits behind the same ingress authentication, asks the Permission Service for authorization decisions, ships a Helm chart with the shared conventions, emits OpenTelemetry signals, and comes with a smoke test and a conformance suite. This page walks those pieces in order. Explain the plan by default; scaffold or deploy only when asked.

If you are implementing an **existing** contract, start from that contract's page instead and come back here for the shared pieces: [storage adapter](custom-storage-adapter/README.md), [notifications adapter](custom-notifications-adapter.md), [permissions adapter](custom-permissions-adapter.md), [thumbnail implementation](custom-thumbnail-implementation.md).

## Define and version the contract

Follow the layout and conventions the existing API workspaces use:

- Protos under `proto/nvidia/omniverse/<service>/<version>/`, directory mirroring the package (buf `PACKAGE_DIRECTORY_MATCH`). Package `nvidia.omniverse.<service>.<version>`, always with the version suffix ([`discovery/api/README.md`](../../../../discovery/api/README.md#conventions)).
- File options: `go_package`, `java_multiple_files = true`, `java_outer_classname` and `java_package`. The NVIDIA contracts use `go_package = "pb.api.nvidia.com/omniverse/<service>/<version>;<alias>"` and `java_package = "com.nvidia.omniverse.<service>.<version>"`; the Go path is only an import path, so building the protos needs no access to that host. For your own service, use your own Go module path and Java package.
- Wire versions move `v1alpha` → `v1beta` → `v1`. Within alpha and beta, add fields and RPCs; never reuse or renumber a field (`reserved` instead); deprecate rather than delete. Keep a `CHANGELOG.md` with a document version independent of the wire version ([versions/api-contracts.md](../versions/api-contracts.md)).
- Publish an OpenAPI document for the REST surface beside the protos. Keep it in sync with what the service serves; the Storage API suite diffs the two with `oasdiff`.
- Gate the protos with `buf lint`, `buf format` and `buf breaking` (the commands are in [`discovery/api/README.md`](../../../../discovery/api/README.md#working-with-the-protos)).
- Consumers compile their **own copy** and record the pin in a `VERSION` file beside it, as [`storage-service-protos/proto/nvidia/omniverse/*/VERSION`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/) does. Nothing propagates contract changes automatically.

A client must generate code from the definitions the deployed service was built with; say which in your release notes.

## Expose Discovery capabilities

The Auto Discovery operator registers a Kubernetes Service only if **both** are true:

1. The Service carries `discovery.omniverse.nvidia.com/enabled: "true"`.
2. The operator can call `CapabilitiesService/ListServices` on the Service's gRPC port and gets a response. Released 1.1.1 calls `nvidia.omniverse.discovery.capabilities.v2alpha`; the unreleased snapshot operator calls `...capabilities.v2beta` first and falls back to v2alpha on `UNIMPLEMENTED`, logging a deprecation warning. Serving v2alpha therefore works with both today; the snapshot's conformance runner checks v2beta only.

There is no REST fallback in the snapshot operator: it only builds entries from the gRPC call ([`src/client/capabilities.rs`](../../../../discovery/reference/discovery-service/operator/src/client/capabilities.rs)). The Storage API's own `nvidia.omniverse.storage.capabilities.*` service does not satisfy it.

### The RPC

Proto: [`discovery/api/proto/nvidia/omniverse/discovery/capabilities/v2alpha/`](../../../../discovery/api/proto/nvidia/omniverse/discovery/capabilities/v2alpha/). [`discovery/api/README.md`](../../../../discovery/api/README.md) still describes the file as `v1beta/capabilities.proto`; the files are `v2alpha`. The v2beta package, with the same messages, is only in the operator's [`discovery-protos/`](../../../../discovery/reference/discovery-service/operator/discovery-protos/README.md).

```text
ListServicesRequest  { int32 page_size = 1; string page_token = 2; }
ListServicesResponse { repeated ServiceEntry services = 1; string next_page_token = 2; }
ServiceEntry         { string service_name = 1; repeated GrpcService grpc_services = 2; repeated RestService rest_services = 3; }
GrpcService          { repeated GrpcEndpoint endpoints = 1; }   GrpcEndpoint { string service_name = 1; }  // fully qualified
RestService          { repeated RestEndpoint endpoints = 1; }   RestEndpoint { string name = 1; repeated RestVersion versions = 2; }
RestVersion          { string version = 1; RestVersionDetail url_detail = 2; }   RestVersionDetail { string path = 1; }
```

The Thumbnail Service's implementation is a compact template ([`capabilities.rs`](../../../../thumbnails/reference/thumbnail-service/crates/service/src/grpc/capabilities.rs)): one entry, `service_name` = a short type (`thumbnail`), the fully qualified gRPC service name, and one REST endpoint whose `name` is what v2 catalog clients see, with `version` (`v1alpha`) and `path` (`/v1alpha/`). Advertise only domain APIs, not health or reflection. For Rust services on Storage Core, see the Storage Service's [`api/discovery/grpc.rs`](../../../../storage/reference/storage-service/storage-service/src/api/discovery/grpc.rs); for Python, generate the stubs from the proto and register the servicer on the same gRPC server.

The operator connects directly to `{service}.{namespace}.svc.cluster.local:{port}` (or through a port-forward in `--use-kube-portforward` mode) and sends **no credentials**. If your service authenticates in-process, exempt `ListServices` and the health service, and nothing else.

### The Kubernetes Service

| Requirement | Detail |
|---|---|
| Opt-in annotation | `discovery.omniverse.nvidia.com/enabled: "true"` (key configurable on the operator) |
| gRPC port | Port named `grpc`, or the `discovery.omniverse.nvidia.com/grpc-port` annotation. Without either the Service is skipped |
| REST port | Port named `http` or `rest`; otherwise no REST URL is published |
| Visibility | `discovery.omniverse.nvidia.com/visibility`: `internal`, `external`, or `both` (default; unknown values also mean `both`) |
| External URLs | `discovery.omniverse.nvidia.com/external-rest-endpoint` / `external-grpc-endpoint`, else derived from a Contour HTTPProxy routing to this Service (`https` when the proxy has TLS; HTTPProxy annotation `discovery.omniverse.nvidia.com/base-path` appended to REST only; `h2` routes count as gRPC) |

Render the opt-in annotation by default in your chart and merge `service.annotations` over it, as the Thumbnail Service does:

```yaml
{{- $discoveryAnnotations := dict "discovery.omniverse.nvidia.com/enabled" "true" -}}
metadata:
  annotations:
    {{- toYaml (mergeOverwrite $discoveryAnnotations (deepCopy (.Values.service.annotations | default dict))) | nindent 4 }}
```

The Storage Service chart does not add the annotation itself; operators set `service.annotations`. Verify registration in the catalog (`GET /api/v2/services` on the operator's internal port `8080`), not by the Helm release status. The legacy static Discovery chart needs a catalog entry instead of the RPC ([deploy/discovery.md](../deploy/discovery.md)).

## Authentication and authorization

The stack separates the two:

| Concern | Component | Your service |
|---|---|---|
| Who is calling | Envoy auth extension on the Contour ingress validates the bearer token ([deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md)) | Route external traffic only through the authenticated HTTPProxy; keep pod ports `ClusterIP` and add a NetworkPolicy |
| May they do this | Permission Service, gRPC `nvidia.omniverse.permission.v1beta` (`CheckPermission`, `CheckPermissionBatch`) or REST `POST /v1beta/authorization/` and `/v1beta/authorization/batch/` ([apis/permissions.md](../apis/permissions.md)) | Ask per operation with (principal, action, resource) |
| Calling other services | Outbound OAuth2 client credentials or a workload-identity token file | Use the `serviceIdentity` chart block below |

How the Storage Service does it, as a pattern: it requires an `Authorization: Bearer …` header on each request, forwards that header (plus any headers listed in `observability.headers.forward`) to the Permission Service, and batches checks (`config.permissions.batchSize`, default 100). Defaults: gRPC client at `http://permission-service:3010`, retries with backoff. Action names per operation are overridable with `config.permissions.actionNames` ([`storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst), [`backend/permissions/request.rs`](../../../../storage/reference/storage-service/storage-service/src/backend/permissions/request.rs)).

For your service:

- Choose a stable `service` name and action names (`read`, `write`, domain-specific actions), and document them so policy authors can grant them. Seed least-privilege policies; never ship a wildcard grant as a default.
- Fail closed: a Permission Service error is a deny (or a retryable 503), never an allow.
- Do not accept identity from arbitrary request headers unless only the ingress can reach the pod.
- Token audiences: the ingress validates tokens for your service's app registration; outbound tokens to the Permission Service or Storage use their own audience/scope. Do not forward a user token to a service that expects a different audience unless that service is designed for it (the Storage Service forwards to the Permission Service by design).

## Helm chart conventions

These keys recur across the stack's charts in this snapshot ([storage](../../../../storage/reference/storage-service/helm/values.yaml), [thumbnail](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-service/values.yaml), [permission](../../../../permissions/reference/permission-service/helm/permission-service/values.yaml), [user info](../../../../user-identity/reference/microsoft-graph-userinfo-service/helm/microsoft-graph-userinfo-service/values.yaml), [event consumer](../../../../notifications/reference/event-consumer-service/helm/event-consumer-service/values.yaml)). Use the same names so operators and scripts carry over.

| Key | Convention |
|---|---|
| `image.{repository,tag,pullPolicy,pullSecrets}` | `tag` empty means chart `appVersion`; `Chart.yaml` `version`/`appVersion` are `0.0.0` placeholders set at release |
| `serviceAccount.{create,name,annotations,automount}` | Workload-identity annotations go here |
| `podSecurityContext`, `securityContext` | Non-root numeric UID (1000), drop all capabilities, no privilege escalation |
| `service.{type,…ports…,annotations}` | Port names `grpc` and `http`/`rest`; discovery annotation rendered by default |
| `httpProxy` | Contour ingress: REST at `<host>.<domain>`, gRPC at `grpc.<host>.<domain>`, TLS, optional auth extension |
| `serviceIdentity.clientCredentials` | `enabled`, `openIdConfigurationUri`, `clientId`, `clientScope`, `clientSecretRef.{name,key}` (an existing Secret), `extraHeaders`, `extraBodyParams` |
| `serviceIdentity.accessTokenFile` | `enabled`, `path` (`/etc/secrets/access-token/token`), `provisionerSidecar.container` (rendered as a Kubernetes 1.29+ native sidecar) |
| `observability` | OTLP export settings; header forwarding |
| `smokeTest` / `tests` | Helm hook Job (`helm.sh/hook: test,post-install,post-upgrade`), own ServiceAccount and identity, `ttlSecondsAfterFinished` |

Never accept a secret value directly in values; take a Secret reference. One known exception to avoid copying: the event-aggregation conformance chart's `auth.token` is a plain value ([test/conformance.md](../test/conformance.md#event-aggregation-service)).

## Observability

Rust services in the stack use the shared crates in [`common/observability/`](../../../../common/observability/) ([components/common-libraries.md](../components/common-libraries.md)):

- `tonic-observability`: server and client trace and metric layers for tonic gRPC (A66 metric conventions, W3C trace-context propagation).
- `axum-observability`: server trace and metric layers for axum HTTP.
- `otel-tools`: OTLP exporter setup bound to `tracing`, configured only through the standard `OTEL_*` environment variables (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_SDK_DISABLED`, …).

Consumers use path dependencies, either on `common/observability` (the Permission Service's [`Cargo.toml`](../../../../permissions/reference/permission-service/crates/permission-service/Cargo.toml)) or on a copy vendored into the component (the Envoy auth extension's [`Cargo.toml`](../../../../permissions/reference/envoy-auth-extension/Cargo.toml)). Also serve the standard gRPC health service for probes. Dashboards, alerting and SLOs: [operate/monitoring.md](../operate/monitoring.md).

## Tests that ship with the service

Match the stack's two-level pattern:

| Test | Shape in the stack | Example |
|---|---|---|
| **Smoke test** | Helm hook Job in the service chart, minimal end-to-end flow, uses the chart's own identity, exits non-zero on failure | [`permission-smoke-test`](../../../../permissions/reference/permission-service/crates/permission-smoke-test/README.md), [`thumbnail-smoke-test`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-smoke-test/README.md) |
| **Conformance suite** | Black-box, runs against any implementation of the contract; CLI flags with env-var equivalents; same three auth modes (none, client credentials, access-token file); separate Helm chart installing a one-shot Job; documented exit codes | [`thumbnail-conformance-test`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/README.md), [`userinfo-conformance-test`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/userinfo-conformance-test/README.md) |

Design rules the existing suites follow and you should too: never require production data; sample fixtures from the live service or take an explicit disposable target; state exactly what is left behind; fail rather than silently skip when prerequisites are missing; report skipped and deselected counts. Every suite in the repo is listed in [test/conformance.md](../test/conformance.md).

## Done checklist

- [ ] Contract published with protos, OpenAPI, CHANGELOG; buf lint and breaking checks pass.
- [ ] Discovery v2alpha `ListServices` served without auth; Service annotated; ports named; entry visible in `/api/v2/services` from the client's network.
- [ ] Only reachable through the authenticated ingress; Permission Service checks on every operation; fails closed.
- [ ] Chart follows the shared keys; no secret values in values; non-root.
- [ ] OTLP traces and metrics; gRPC health; probes.
- [ ] Smoke test hook and conformance suite with a Helm Job chart; both green in a pre-production namespace.
