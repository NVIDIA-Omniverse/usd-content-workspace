(userinfo-service-troubleshooting-troubleshooting-and-error-behavior)=
# Troubleshooting and Error Behavior

User Info Service is a read-only service that answers directory lookups over REST and gRPC. This
page describes the signals an operator uses to tell whether an instance is healthy and serving,
the error responses a caller can receive and what each one means, why some requests can fail
while others keep working depending on where the data is served from, and how to diagnose the
problems these signals point to.

It builds on the roles and data flow in
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`, the telemetry in
{ref}`Observability <userinfo-service-observability-observability>`, and the deployment steps in
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`. For the availability behavior behind an
outage of one component, see {ref}`Scaling and High Availability <userinfo-service-scaling-scaling-and-high-availability>`.

(userinfo-service-troubleshooting-health-and-readiness)=
## Health and Readiness Probes

Every instance exposes HTTP liveness and readiness probes on the REST port. **Readers** also
expose the gRPC Health Checking Protocol on the gRPC port. The **writer** serves only the HTTP
probes — it does not open the gRPC port because it does not serve the read APIs. What "ready"
means differs by role.

- **Liveness (`GET /healthz`).** Returns a 2xx status whenever the process is running and able to
  answer HTTP. It does not reflect whether the directory is loaded, so a live process that has
  not yet become ready still passes liveness. The writer answers liveness from startup while its
  synchronization runs in the background. A liveness failure means the process is wedged, and
  Kubernetes restarts the container.
- **Readiness (`GET /readyz`).** Returns a 2xx status once the instance reports ready, and a
  `503 Service Unavailable` until then. A reader in the synchronized deployment reports ready as
  soon as it has started and loaded whatever the shared store holds — which can be a partial or
  empty directory if the writer's first synchronization has not been saved yet; a reader in live
  mode is ready immediately. The writer reports ready once it holds a complete directory: at
  startup when the store already holds one, otherwise after its first full synchronization
  completes. Once ready, an instance stays ready until its process stops. A pod that is not ready
  is removed from the endpoints of the Kubernetes Service and receives no traffic through it; a
  failing readiness probe never restarts a container.
- **gRPC health (readers only).** On reader pods, the gRPC port exposes the standard gRPC Health
  Checking Protocol. The service reports `NOT_SERVING` until the reader is ready and `SERVING`
  afterward, mirroring the readiness probe for gRPC clients and gRPC-aware load balancers.

The most common readiness symptom is the writer taking a long time to become ready on a large
directory: when the store does not yet hold a complete directory, the writer reports ready only
after its first full synchronization finishes, which can take hours when membership
synchronization is enabled. This does not cause a restart — Kubernetes restarts a container only
when its liveness probe fails, and the writer keeps passing liveness while it synchronizes — and
because the writer is not part of the Kubernetes Service, an unready writer affects no traffic. See
{ref}`Memory and CPU Sizing <userinfo-service-scaling-sizing>` for how directory size affects the
first synchronization. Readers, by contrast, become ready in seconds by loading whatever the store
already holds. A reader that starts before the writer has saved a complete directory becomes ready
but serves an empty or partial directory until a later reload picks up the saved data, so a ready
reader does not by itself mean the directory is populated; see
{ref}`Diagnosing Common Problems <userinfo-service-troubleshooting-diagnosing>` for what to check.

(userinfo-service-troubleshooting-error-responses)=
## Error Responses

Every non-2xx REST response carries a JSON body with a stable machine-readable `error` code and a
short human-readable `message`, and the equivalent gRPC call returns the matching status code. The
codes and their gRPC equivalents are:

| REST status | REST `error` | gRPC status | Meaning |
|---|---|---|---|
| 400 | `bad_request` | `INVALID_ARGUMENT` | The request is malformed — for example an unparseable pagination cursor. The **Omniverse User Info API** documentation describes valid query parameters and pagination. |
| 401 | `unauthorized` | `UNAUTHENTICATED` | The bearer token is missing or invalid, or the caller's identity could not be resolved from it. |
| 403 | `forbidden` | `PERMISSION_DENIED` | The caller is authenticated but not permitted to perform the operation. |
| 404 | `not_found` | `NOT_FOUND` | The requested user, group, or membership does not exist. |
| 429 | `rate_limited` | `RESOURCE_EXHAUSTED` | The identity provider's rate limit was exceeded. REST responses include a `Retry-After` header. |
| 502 | `bad_gateway` | `UNAVAILABLE` | The identity provider returned an unexpected error. |
| 500 | `internal_error` | `INTERNAL` | An unexpected internal failure, or the Permission Service could not be reached to authorize the request. |

The client-visible `message` is a fixed, generic string; it never echoes identity-provider
diagnostics, internal error chains, or caller-supplied input back to the client. The detailed
cause is recorded only in the service's logs, so when a 5xx response needs investigating, the
matching log line on the instance that produced it carries the specifics. Server-side errors
(`bad_gateway`, `internal_error`) are logged at error level and a `rate_limited` response at
warning level, so both surface on dashboards; the expected client-side statuses (`unauthorized`,
`forbidden`, `not_found`) are logged at debug level and are not treated as faults. See
{ref}`Observability <userinfo-service-observability-observability>` for the request metrics and logs that expose these responses
in aggregate.

Token validation itself is performed by the platform's authentication gateway in front of the
service; a `401` from the service means the request reached it without a usable token or with one
whose subject does not match a directory record, not that a specific token was rejected by
internal validation.

(userinfo-service-troubleshooting-partial-failures)=
## Why Some Requests Fail While Others Keep Working

Whether a request depends on the identity provider being reachable is decided entirely by where
the requested data is served from, so a provider problem can affect some requests while leaving
others untouched.

- **Data served from the replica.** In the synchronized deployment, readers answer user, group,
  and membership lookups from their in-memory copy of the directory. These requests never contact
  the identity provider, so they keep succeeding — returning the last loaded data — even while the
  provider is unreachable, rate-limiting, or returning errors. A provider outage pauses data
  freshness rather than availability; see
  {ref}`High Availability <userinfo-service-scaling-high-availability>`.
- **Data served live from the provider.** When a data category is not cached — for example, when
  membership synchronization is disabled — requests for that kind of data are answered directly
  from the identity provider instead of from the replica. Those specific requests can therefore
  return `429 rate_limited` or `502 bad_gateway` when the provider throttles or fails, while user
  and group lookups served from the replica continue to succeed. The same is true of the
  reader-only live mode, in which every request goes to the provider and provider problems surface
  on all of them; see {ref}`Live Mode <userinfo-service-directory-synchronization-live-mode>`.
- **Authorization dependency.** When Permission Service integration is enabled, each request is
  authorized before it runs and list results are filtered per item. If the Permission Service
  cannot be reached, the service fails closed and returns `500 internal_error` even though the
  directory data is available. Health, readiness, and current-user requests are exempt from
  permission checks and keep working. See
  {ref}`Authorization with the Permission Service <userinfo-service-authorization-authorization-with-the-permission-service>`.

(userinfo-service-troubleshooting-diagnosing)=
## Diagnosing Common Problems

The signals above localize most problems to a role and a cause:

- **Ready readers return empty or incomplete results.** Readers become ready without waiting for
  the writer, so empty or partial results shortly after a deployment usually mean the writer has
  not yet saved a complete directory. Check that the writer reports ready and that its
  synchronization logs show a completed cycle, then confirm that the readers' snapshot size
  metrics (`directory.snapshot.users`, `directory.snapshot.groups`) and reload logs show the
  expected counts. After the writer finishes, each reader picks up the complete directory within
  one reload interval. See
  {ref}`How Readers Load and Refresh the Replica <userinfo-service-directory-synchronization-how-readers-refresh>`.
- **An instance does not become ready.** A reader in the synchronized deployment that cannot
  connect to the shared store at startup exits instead of becoming ready, and Kubernetes restarts
  it; repeated reader restarts therefore point to the store address, credentials, or network path —
  check the reader's logs and see
  {ref}`Database Configuration <userinfo-service-database-database-configuration>`. If the writer is
  unready long after startup, it does not yet hold a complete directory — check the writer's
  synchronization logs and metrics for progress or repeated cycle failures.
- **Requests return `429` or `502`.** These come from the identity provider and appear only on
  requests served live from it — live mode, or a data category that is not cached. Check the
  Microsoft Graph client metrics and the writer's synchronization logs for throttling or provider
  errors. Data served from the replica is unaffected. See
  {ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>`.
- **Requests return `500` with authorization enabled.** A fail-closed `internal_error` most often
  means the Permission Service is unreachable or misconfigured. Confirm its address and that it
  holds policies for the configured service name; see
  {ref}`Permission Service Integration <userinfo-service-deployment-permission-service>`.
- **Data looks stale or differs between callers.** This is expected for the service's
  eventually-consistent model: a change becomes visible only after the writer's next cycle saves
  it and each reader's next reload picks it up, and readers reload independently. See
  {ref}`Data Freshness and Eventual Consistency <userinfo-service-directory-synchronization-data-freshness>`.

In every case, the per-instance logs carry the detailed cause behind a generic client message,
and the request and Microsoft Graph client metrics show whether errors are concentrated on a
particular route, role, or upstream call.

(userinfo-service-troubleshooting-configuration)=
## Configuration

The probe paths are fixed at `/healthz` and `/readyz` on the REST port; the probe definitions are
configurable through the Helm chart and apply to both the reader and the writer. The chart
defines a liveness probe and a readiness probe, and no startup probe.

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Liveness probe | `livenessProbe` | `httpGet` on `/healthz`, port `http` | HTTP liveness probe run against the REST port. Kubernetes restarts the container when it fails. The chart sets no timing fields, so the Kubernetes defaults apply. |
| Readiness probe | `readinessProbe` | `httpGet` on `/readyz`, port `http` | HTTP readiness probe run against the REST port. A failing check removes the pod from Service endpoints; it never restarts the container. |
| Readiness initial delay | `readinessProbe.initialDelaySeconds` | `5` | Seconds to wait after container start before the first readiness check. |
| Readiness period | `readinessProbe.periodSeconds` | `10` | Interval between readiness checks. |
| Readiness failure threshold | `readinessProbe.failureThreshold` | `180` | Consecutive failed readiness checks before Kubernetes marks a ready container as not ready. It does not restart the container and does not limit how long the writer may take to become ready. |

Because a readiness failure never restarts a container, the writer's first full synchronization
can take as long as the directory requires without any probe change; the readiness settings do not
need to grow with directory size.

(userinfo-service-troubleshooting-helm)=
### Configuring Probes in the Helm Chart

Add the probe settings to `userinfo-values.yaml`. The example below restates the chart's default
probes; adjust the timing fields to suit your cluster:

```yaml
livenessProbe:
  httpGet:
    path: /healthz
    port: http

readinessProbe:
  httpGet:
    path: /readyz
    port: http
  initialDelaySeconds: 5
  periodSeconds: 10
  failureThreshold: 180
```

Install or upgrade the release with the same commands used for the synchronized directory
deployment in {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-troubleshooting-references)=
## References

- [Configure Liveness, Readiness and Startup Probes][k8s-probes] — how Kubernetes runs HTTP
  probes, treats a 2xx–3xx response as success, restarts a container on liveness failure, and
  removes an unready pod from Service endpoints.
- [Liveness, Readiness, and Startup Probes][k8s-probes-concepts] — the probe types and fields,
  including that a failed readiness probe keeps the container running and only sets the pod's
  `Ready` condition to `false`, while liveness and startup failures trigger a restart.
- [gRPC Health Checking Protocol][grpc-health] — the standard health service the gRPC port
  exposes, and its `SERVING` / `NOT_SERVING` statuses.

[k8s-probes]: https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/
[k8s-probes-concepts]: https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/
[grpc-health]: https://github.com/grpc/grpc/blob/master/doc/health-checking.md
