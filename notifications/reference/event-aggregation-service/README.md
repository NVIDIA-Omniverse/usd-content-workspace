# Event Aggregation Service

Part of the broader Omniverse Notification Service.
The Event Aggregation Service allows other services to publish events
that are then published to RabbitMQ so that the Event Consumer Service
can access them and broadcast them to all consumers.

## Usage

The API is implemented as both a gRPC service and a REST API.

### gRPC API

This service's source is published in the
`NVIDIA-Omniverse/usd-content-workspace` repository under
`notifications/reference/event-aggregation-service`. To build a gRPC client, use
the publisher Protocol Buffer definitions, published once for all notification
services under `notifications/api` in the same repository
(`notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto`);
this service's folder ships the generated Python bindings. The `openapi.yaml`
REST specification and the `conformance_tests/` suite are in this service's
folder.

For usage guides to both notification services' APIs — getting started,
publishing events, consuming events, permissions, and the API reference — see
the customer documentation published with the Event Consumer Service, under
`notifications/reference/event-consumer-service/docs/` in the same repository:
[getting started](../event-consumer-service/docs/getting-started.md),
[publishing events](../event-consumer-service/docs/publishing-events.md),
[consuming events](../event-consumer-service/docs/consuming-events.md),
[API reference](../event-consumer-service/docs/api-reference.md),
[permissions](../event-consumer-service/docs/permissions.md),
and the [index](../event-consumer-service/docs/index.md).
The complete rendered manual is
[`docs.pdf`](../event-consumer-service/docs/docs.pdf).

### REST API

The REST API provides the same functionality as the gRPC API with two endpoints:

- `POST /api/v1beta/events` - Publish a single event
- `POST /api/v1beta/events/batch` - Publish multiple events in parallel

OpenAPI documentation is available at `/docs` and `/redoc` when the service is
running. Those two pages, the generated `/openapi.json`, and the
`/docs/oauth2-redirect` helper are served without an `Authorization` header.
The OpenAPI specification is also available in the `openapi.yaml` file.

#### Request size limit

Every REST request whose body exceeds `OMNI_EVENTS_REST_MAX_REQUEST_BYTES`
(default `4194304` — 4 MiB) is refused with **`413 Content Too Large`** and the
JSON body:

```json
{"detail": "Request body exceeds the maximum of 4194304 bytes accepted by this service."}
```

The refusal also carries `Connection: close` and the server closes the
connection, so a client must open a new one rather than reusing it for a
follow-up request.

The limit applies to every REST route (the `GET` health route carries no body,
so it is unaffected). Enforcement does not depend on framing: a request that
declares an oversize `Content-Length` is refused from the header, and a chunked
upload is refused the moment it passes the limit.

The refusal happens **before authentication and before the request is parsed**,
so no event is published — the limit is a resource bound, not an authorization
control. It is also independent of the 1000-event batch limit (see
[Event limits](#event-limits)):
either can refuse a request first, and for events averaging more than about
4 KiB the byte limit is the one that binds.

Two consequences of refusing this early are worth knowing when you debug a 413:
the response carries **no CORS headers** and produces **no trace span** — a
browser cross-origin caller therefore sees an opaque network error rather than
the status. (The publish API is server-to-server.) Server-side, each refusal is
logged at `WARNING` with the declared or streamed body size and the configured
limit.

The default matches the gRPC API's default 4 MiB inbound message cap, so both
APIs refuse at the same bound.

### Event limits

Both APIs apply the same per-request limits, which are fixed rather than
configurable:

- A batch may carry at most **1000 events**.
- An event's `message`, encoded as JSON, may be at most **65536 bytes** (64 KiB).
  This is measured as Python's `json.dumps` encodes it with its defaults, which
  is how the event is published: `", "` and `": "` separators, every non-ASCII
  character escaped as `\uXXXX` (six bytes; a character outside the Basic
  Multilingual Plane becomes a twelve-byte surrogate pair), plus JSON's usual
  escapes for quotes, backslashes, and control characters. On gRPC the
  message is first converted from a `Struct`, so every number is a double
  (`1` is measured as `1.0`). Leave headroom rather than sizing a message to the
  exact byte.
- `event_type` may be at most 256 characters (Unicode code points),
  `resource.resource_id` at most 2048, and (REST only, where it is a string)
  `occurred_at` at most 64.

A request over any limit publishes nothing: REST answers `422` with a
validation error whose `loc` names the field, and gRPC answers
`INVALID_ARGUMENT` with a message naming the field (for example
`events[1].message must be at most 65536 bytes ...`). One oversize event refuses
its whole batch. These limits are separate from the 4 MiB request-size bound on
each transport: a gRPC request over gRPC's 4 MiB inbound message cap is refused
by the transport with `RESOURCE_EXHAUSTED`, which names no field, and a REST
body over the [request size limit](#request-size-limit) with `413`.

## Deployment

This service is built to be deployed in k8s or NVCF.

### Configuration

The app is configured with environment variables:

| Env Var Name                               | Default Value           | Description                                                                                                                                                                                                      |
|--------------------------------------------|-------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| OMNI_EVENTS_RABBITMQ_URI                   | "amqp://127.0.0.1:5672" | The address for the RabbitMQ instance.                                                                                                                                                                           |
| OMNI_EVENTS_GRPC_HEALTH_CHECKING_ENABLED   | True                    | Optionally turn off gRPC health checking servicer.                                                                                                                                                               |
| OMNI_EVENTS_LOG_LEVEL                      | INFO                    | The logging level for the deployed service (DEBUG, INFO, WARNING, ERROR, CRITICAL)                                                                                                                               |
| OMNI_EVENTS_GRPC_PORT                      | "[::]:50051"            | Set the port to use for the gRPC server.                                                                                                                                                                         |
| OMNI_EVENTS_REST_API_ENABLED               | True                    | Whether the REST API is enabled. When disabled, only gRPC is available.                                                                                                                                          |
| OMNI_EVENTS_REST_PORT                      | 8000                    | Set the port to use for the REST API server. `uv run start` defaults it to 8001 when unset.                                                                                                                      |
| OMNI_EVENTS_CORS_ALLOW_ORIGINS             | []                      | Browser origins the REST server answers CORS requests for, with credentials allowed; a comma-separated list or a JSON array. Empty adds no CORS middleware, so browsers receive no Access-Control-Allow-Origin header. Write each entry exactly as a browser sends the Origin header: `http` or `https`, an ASCII (punycode) host, no default port (`:80` / `:443`), and no path or trailing slash, for example `https://app.example.com`. The service refuses `*`, `null`, and a malformed entry at startup. The Helm chart sets it from `rest.corsAllowOrigins` and refuses at render time only `*`, `null`, a value that is not a list, and an entry that is not a string; a malformed origin renders and is refused at startup. |
| OMNI_EVENTS_REST_MAX_REQUEST_BYTES         | 4194304                 | The largest REST request body accepted, in bytes (4 MiB). A larger request is refused with `413 Content Too Large` — see [Request size limit](#request-size-limit). Must be greater than 0. Exposed as the Helm value `rest.maxRequestBytes`. |
| OMNI_EVENTS_TELEMETRY_ENABLED              | False                   | Whether telemetry is enabled. Endpoint vars must be populated                                                                                                                                                    |
| OMNI_EVENTS_OTLP_TRACING_ENDPOINT          | "localhost:4317"        | OTLP-compatible endpoint for traces                                                                                                                                                                              |
| OMNI_EVENTS_OTLP_METRICS_ENDPOINT          | "localhost:4317"        | OTLP-compatible endpoint for metrics                                                                                                                                                                             |
| OMNI_EVENTS_OTLP_LOGS_ENDPOINT             | "localhost:4317"        | OTLP-compatible endpoint for logs                                                                                                                                                                                |
| OMNI_EVENTS_PRINCIPAL_SOURCE               | "jwtUnverified"         | The single first-class gate selecting how the principal (user) is derived and whether a token is required. One of: `disabled` (no token required, no principal, no auth middleware — incompatible with permission enforcement), `jwtUnverified` (the built-in default, refused at startup unless OMNI_EVENTS_ALLOW_UNVERIFIED_JWT is true — decodes the JWT payload to extract the principal but performs NO signature/audience/expiry verification, trusting an upstream validator such as Envoy — see the [Authentication mode](#authentication-mode-principalsource) section), `jwt` (validates the token against the IdP JWKS; requires OMNI_EVENTS_OPENID_CONFIGURATION_URI and OMNI_EVENTS_JWT_AUDIENCES), `opaque` (validates the token via the IdP `/userinfo` endpoint; requires OMNI_EVENTS_OPENID_CONFIGURATION_URI), `userInfoService` (resolves the principal via `GET {OMNI_EVENTS_USER_INFO_SERVICE_URI}/api/v1alpha/users/me` with the forwarded bearer token; requires OMNI_EVENTS_USER_INFO_SERVICE_URI). |
| OMNI_EVENTS_ALLOW_UNVERIFIED_JWT           | False                   | Explicit opt-in for OMNI_EVENTS_PRINCIPAL_SOURCE=`jwtUnverified`. Set it to true only when every path to the service verifies the bearer token first — a verifying proxy such as the Envoy auth extension, plus a control (for example a NetworkPolicy) that stops callers from reaching the service any other way; without it, `jwtUnverified` fails at startup. Inert in every other mode. The Helm chart renders it from `allowUnverifiedJwt`, and only together with an explicit `principalSource: jwtUnverified`. |
| OMNI_EVENTS_USER_INFO_SERVICE_URI          | ""                      | Base URI of the user-info service. Consulted only when OMNI_EVENTS_PRINCIPAL_SOURCE is `userInfoService`, where the principal is resolved via `GET {uri}/api/v1alpha/users/me` with the caller's forwarded bearer token. Required in that mode.                                                                |
| OMNI_EVENTS_OPENID_CONFIGURATION_URI       | ""                      | The OpenID Connect discovery endpoint (/.well-known/openid-configuration) for fetching OIDC provider metadata and JWKS. Required when OMNI_EVENTS_PRINCIPAL_SOURCE is `jwt` or `opaque`.                          |
| OMNI_EVENTS_JWT_AUDIENCES                  | ""                      | Comma-separated list of expected audience values for JWT tokens (the `aud` claim). Required when OMNI_EVENTS_PRINCIPAL_SOURCE is `jwt`.                                                                          |
| OMNI_EVENTS_OPENID_ENABLED                 | False                   | Infra-only field (enables the Envoy authExtension / auth filter); it does not by itself gate token-requirement or permission checks. Authentication behavior is keyed on OMNI_EVENTS_PRINCIPAL_SOURCE and permission enforcement on OMNI_EVENTS_PERMISSIONS_ENABLED (`permissions.enabled`).                       |
| OMNI_EVENTS_PRINCIPAL_ID_CLAIM             | "sub"                   | The JWT/userinfo claim used as the principal (user) identity. Set to `oid` for Azure AD, where `sub` is pairwise per client-app and `oid` is the stable tenant-wide object ID. This is a one-way, deploy-time choice that must match the Event Consumer Service — see [Principal identity claim](#principal-identity-claim) before changing it on an existing deployment. |
| OMNI_EVENTS_AUTH_CONTEXT_TTL               | 1800                    | Time-to-live in seconds for cached authentication context (default: 30 minutes).                                                                                                                                 |
| OMNI_EVENTS_PERMISSIONS_ENABLED            | False                   | The enforcement gate for the per-event-type `publish-event` permission check. When true, the check is enforced against OMNI_EVENTS_PERMISSIONS_ENDPOINT; when false, checks pass. Requires a non-`disabled` OMNI_EVENTS_PRINCIPAL_SOURCE and OMNI_EVENTS_PERMISSIONS_ENDPOINT to be configured.                  |
| OMNI_EVENTS_PERMISSIONS_ENDPOINT           | None                    | The endpoint to use for the Permissions API — the target the permission check is made against. Required when OMNI_EVENTS_PERMISSIONS_ENABLED is true; inert (ignored) when that gate is false.                    |
| OMNI_EVENTS_PERMISSIONS_TTL_SECONDS        | 600                     | Set the time to live for the permissions cache.                                                                                                                                                                  |
| OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_SIZE     | 100                     | The channel count at which packing begins. While OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED is true (the default) this is **not** a cap on concurrent publish operations: pool size × client limit is the threshold at which the overflow band engages, and checkouts past it are packed onto already-open channels rather than refused. Raising it defers packing; it does not raise a ceiling. Disable overflow to make pool size × client limit a hard cap again. The Helm chart sets `25`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_MIN_SIZE | 75                      | Minimum channels to pre-create at startup. Avoids cold-start latency. The Helm chart sets `5`.                                                                                                                    |
| OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT  | 8                       | The packing target for concurrent clients per channel. While overflow is enabled this is a **target, not an isolation guarantee**: once the pool is full and no channel is under the limit, a checkout is admitted onto the least-loaded open channel anyway and that channel's client count rises past this value. At 1 the pool opens a new channel per concurrent publish until the pool is full and only then packs — the most isolation this setting can buy while overflow is on. For strict per-request isolation, set OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED to False and raise the pool size instead. The Helm chart sets `1`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED | True                 | Whether channel checkouts are admitted past the packing threshold. When true (the default), a burst arriving after every channel is open and at the client limit is packed onto the least-loaded open channel instead of failing; the tradeoff is blast radius, since a channel-level error fails every publish in flight on that channel. When false, the pool is strict one-tier: the same state waits up to OMNI_EVENTS_RABBITMQ_CHANNEL_WAIT_MS and then fails the publish. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_WAIT_MS       | 500                     | Upper bound, in milliseconds, on how long a publish waits to acquire a channel before the publish fails. With overflow enabled a full pool packs rather than waits, so this bounds the wait for channel **creation** (it only bites when the broker or connection is in trouble); with overflow disabled it also bounds the wait for a free slot on a saturated pool. |
| OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS  | 0.0                     | How long, in seconds, the service keeps serving normally after SIGTERM before it begins draining — readiness stays SERVING and both APIs admit new publishes for the whole delay, so the endpoint change pod deletion triggers has time to propagate. `0` skips the phase. The Helm chart sets `15.0` (`shutdown.settleDelaySeconds`) — see [Graceful shutdown](#graceful-shutdown).                    |
| OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS  | 30.0                    | Upper bound, in seconds, on how long shutdown waits, once the settle delay has elapsed, for the channel pool's in-flight publish checkouts to fall to zero before the RabbitMQ connector is marked closing and closed. The Helm chart sets `5.0` — see [Graceful shutdown](#graceful-shutdown).                                                                          |
| OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS | 0.5               | How often, during the drain window, shutdown re-reads the in-flight checkout count to decide whether the pool has drained early. A smaller interval finishes shutdown sooner at the cost of more frequent polls. Must be greater than 0.                                                                                                          |

### Graceful shutdown

On pod deletion the Helm chart gives the pod an ordered termination budget, sized
by its `shutdown.*` values:

1. **Pod deletion, then SIGTERM, then the settle**
   (`shutdown.settleDelaySeconds`, default `15.0`) — the kubelet sends SIGTERM
   immediately, and the service keeps serving normally for the settle delay:
   readiness stays SERVING and both the gRPC and the REST API admit new
   publishes for its whole length. Kubernetes flips the pod's EndpointSlice
   entry to not-ready (terminating) at deletion, and because the Service does
   not publish not-ready addresses Contour drops it from Envoy's endpoint set.
   That removal is not instantaneous, and the settle is the budget for the
   propagation lag (EndpointSlice → Contour xDS → Envoy). It is the only thing
   in this chart that covers that lag: the HTTPProxy carries no retry policy, so
   a request Envoy dispatches to an already-stopped pod is not re-placed.
   Setting the delay to `0` skips the phase and the drain begins immediately on
   SIGTERM. The delay is applied by the service itself, not by a pod lifecycle
   hook.
2. **The drain** (`shutdown.drainWindowSeconds`, default `5.0`, polled every
   `shutdown.drainPollIntervalSeconds`, default `0.5`) — once the settle
   elapses, the service opens the RabbitMQ connector's drain and waits, up to
   the window, for the channel pool's in-flight publish checkouts to fall to
   zero, returning early as soon as none are in flight. This defers marking the
   connector closing past publishes already in flight. A publish is a
   millisecond-scale operation, so the pool normally drains early and shutdown
   proceeds immediately. Both APIs keep serving for the whole drain too —
   readiness stays SERVING and publishes are admitted right up until the servers
   stop.
3. **Both servers stop, then the connector closes** — the gRPC and the REST
   server stop together once the drain returns, whether the pool emptied early
   or the window elapsed. The gRPC server takes up to its 5s stop grace, and the
   REST teardown runs alongside it (it stops accepting connections, waits on
   in-flight requests, then sleeps an unconditional 5s in the lifespan handler);
   the connector is closed once both have returned.

`shutdown.terminationGracePeriodSeconds` (default `60`) is the total budget
Kubernetes allows from pod deletion to SIGKILL, so it must cover all three
phases:

```
terminationGracePeriodSeconds >= settleDelaySeconds + drainWindowSeconds + 10
```

The trailing 10 seconds are the post-drain server teardown. That term is fixed
by the application rather than being a knob: the gRPC stop grace and the REST
teardown run concurrently once the drain ends, so 10s covers the longer leg —
the REST lifespan's 5s sleep — with roughly 5 seconds of slack, which the
post-teardown connector close described below relies on. At the shipped
defaults the invariant holds comfortably: `60 >= 15 + 5 + 10 = 30`, leaving 30s
spare. The chart enforces it at render time, so an operator who lowers the
grace period, or raises `settleDelaySeconds` or `drainWindowSeconds`, must keep
it true or the render is rejected. The render also fails when any `shutdown.*`
value, `terminationGracePeriodSeconds` included, is non-numeric, empty,
negative, or non-finite (`drainPollIntervalSeconds` must additionally be
greater than zero), and when `terminationGracePeriodSeconds` is not a whole
number of seconds (Kubernetes takes it as an integer). Those checks run in that
order, ahead of the invariant, so a bad value is named against its own knob
rather than reported as a shortfall on another; `settleDelaySeconds` and
`drainWindowSeconds` are float knobs and may be fractional.

Because the settle runs on every pod deletion, a rollout spends at least the
settle delay per pod — about 15 seconds at the shipped defaults — before that
pod stops serving. Set `shutdown.settleDelaySeconds: 0` to opt out, at the cost
of the de-routing coverage above.

**What the invariant does not cover.** The REST teardown also waits for open
connections to close, and that wait is unbounded. A publish is a
millisecond-scale request, so this is not a concern for this service's own
routes, but a client holding a connection open past teardown can push the tail
beyond the 10s term. The connector close that runs after both server legs return
is normally sub-second, and the roughly 5 seconds of slack the 10s term carries
over the real (~5s, concurrent) server legs is what absorbs it — at every grace
period the render guard admits, including one exactly at the floor. What the 10s
term does not bound is that close blocking on an unresponsive broker. That tail,
like the unbounded connection wait above, is covered only by whatever margin the
grace period leaves above the floor — 30 seconds at the shipped defaults, zero at
the floor itself. Raise the grace period above the floor if either applies to
your deployment.

The chart carries **no Kubernetes-version requirement** for graceful shutdown —
it renders only `terminationGracePeriodSeconds` and the settings env vars, all
supported on every current Kubernetes release.

## Authentication

### Authentication mode (principalSource)

How the principal (user) is derived from each request, and whether a bearer
token is required at all, is selected by a single first-class setting,
`OMNI_EVENTS_PRINCIPAL_SOURCE`. It is the sole gate for authentication behavior.
Its built-in default is `jwtUnverified`, but that mode is refused at startup
unless `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true` is also set, so a deployment
must choose a mode explicitly. For a local run without tokens, set
`OMNI_EVENTS_PRINCIPAL_SOURCE=disabled`.

The Helm chart chooses as follows (see `principalSource` and
`allowUnverifiedJwt` in the chart values):

1. An explicit `principalSource` is used as given.
2. Otherwise, when OpenID is configured — `openId.openIdConfigurationUri` is set
   and the JWT audience list (`openId.additionalJwtAudience` plus the
   `clientId` of the `default` entry in `openId.clientRegistrations`) is not
   empty — the chart selects `jwt`, so the service verifies every token itself.
3. Otherwise the render fails with a message naming both fixes.

The chart never selects `jwtUnverified` by itself, and it fails the render for
every authentication or permissions configuration the service would refuse at
startup (for example `jwt` without an audience, or `disabled` with
`permissions.enabled`), so such a mistake surfaces at `helm template` /
`helm upgrade` rather than as a crash-looping pod. The one startup check the
chart does not repeat is the exact form of each `rest.corsAllowOrigins` entry
(it refuses only `*`, `null`, and a value that is not a list). The modes are:

- **`disabled`** — no bearer token is required, no principal is derived, and no
  authentication middleware runs. Permission enforcement cannot be used in this
  mode (`OMNI_EVENTS_PERMISSIONS_ENABLED` must be false).
- **`jwtUnverified`** — the JWT payload is decoded to extract the principal, but
  the token's signature, issuer, audience, and expiry are **not** verified (see
  the trust boundary below). Requires the explicit opt-in
  `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`; without it the service refuses to
  start.
- **`jwt`** — the token is validated against the IdP's JWKS. Requires
  `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES`. A
  failure to *reach* the JWKS endpoint is reported as `500` / `INTERNAL`, since
  an identity-provider outage is not the caller's bad credential; an unknown
  signing-key id remains a `401`.
- **`opaque`** — the token is validated via the IdP's `/userinfo` endpoint.
  Requires `OMNI_EVENTS_OPENID_CONFIGURATION_URI`.
- **`userInfoService`** — the principal is resolved by calling
  `GET {OMNI_EVENTS_USER_INFO_SERVICE_URI}/api/v1alpha/users/me` with the caller's
  forwarded bearer token. Requires `OMNI_EVENTS_USER_INFO_SERVICE_URI`.

Whenever authentication is enabled — that is, in every mode except `disabled` —
the bearer token is resolved before any publish handler runs: a caller whose
token cannot be resolved to a principal — including one whose configured
principal claim is present but empty — is refused with `401` /
`UNAUTHENTICATED` on REST and `UNAUTHENTICATED` on gRPC, before the request
body is parsed and without publishing anything. The only refusals that
precede that check are the two size bounds: the REST `413` described under
[Request size limit](#request-size-limit) and the gRPC transport's inbound
message cap.

**Trust boundary for `jwtUnverified`.** In the `jwtUnverified` mode the
service does NOT verify the token's signature, audience, or expiry — it only
decodes the payload to read the principal claim. It trusts an upstream component
(for example Envoy) to have already validated the token before the request
reaches this service. Use this mode only where every path to the service passes
such a validator; otherwise the service trusts attacker-controllable claims.
The Envoy auth extension on the chart's HTTPProxy is not enough on its own: the
chart's Service is `ClusterIP` by default, and an in-cluster caller that reaches it
directly bypasses the HTTPProxy and its auth extension. If you cannot also make
the Service unreachable except through the proxy (for example with a
NetworkPolicy), use `jwt` or `opaque` so the service validates the token
itself. Because of this, the mode is an explicit opt-in: with the Helm chart,
set both `principalSource: jwtUnverified` and `allowUnverifiedJwt: true`; the
render fails when either is set without the other.

> **Migration.** Earlier releases accepted unverified tokens by
> default. A deployment that relied on that default now fails at startup until
> it chooses a mode. With the Helm chart, a configuration that cannot
> authenticate fails at render time (`helm template` / `helm upgrade`) rather
> than as a crash-looping pod.
>
> A chart deployment that sets OpenID (`openId.openIdConfigurationUri` plus a
> `default` client registration or `openId.additionalJwtAudience`) but no
> `principalSource` moves from `jwtUnverified` to `jwt` on upgrade, with no
> values change. The service then verifies every token itself, so check
> before upgrading that your callers' tokens pass:
>
> - An expired token is refused with `401` / `UNAUTHENTICATED`;
>   `jwtUnverified` accepted it.
> - A token whose `aud` is not an accepted audience (the `default` client's
>   `clientId` plus `openId.additionalJwtAudience`) is refused the same way.
> - The pods need network egress to the identity provider's discovery and
>   JWKS (signing-key) endpoints; a provider the service cannot reach is
>   reported as `500` / `INTERNAL`.
>
> A render that succeeds does not prove any of these. A chart deployment with
> neither OpenID nor an explicit `principalSource` must now set one
> (`disabled` for a run without tokens).
>
> To roll back, set `principalSource: jwtUnverified` together with
> `allowUnverifiedJwt: true` in the deployment values. Do this only where the
> trust boundary above holds: every path to the service passes a verifying
> proxy, which takes the HTTPProxy's auth extension plus a control (for
> example a NetworkPolicy) that keeps the Service unreachable directly.

## Permissions

Permission enforcement is controlled by a single gate,
`OMNI_EVENTS_PERMISSIONS_ENABLED` (`permissions.enabled`, default `False`). When
true, a per-event-type `publish-event` permission check is enforced against the
Permissions API at `OMNI_EVENTS_PERMISSIONS_ENDPOINT` before a client may publish
a given event_type. When false, the checks pass.

Enforcement is no longer inferred from whether `OMNI_EVENTS_PERMISSIONS_ENDPOINT`
is set — the endpoint is only the target the check is made against once the gate
is on, and it is inert when the gate is off. When `OMNI_EVENTS_PERMISSIONS_ENABLED`
is true, `OMNI_EVENTS_PERMISSIONS_ENDPOINT` must be configured. Enabling the gate
also requires a non-`disabled` `OMNI_EVENTS_PRINCIPAL_SOURCE` — there must be a
principal to authorize.

To allow publishing, a policy must be set for each event_type that is to be published.
E.g.:

```
permit(
  principal == Principal::"principal-id-goes-here",
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"omni.storage.created"
);
```

### Principal identity claim

The claim that identifies the principal (user) is configurable via
`OMNI_EVENTS_PRINCIPAL_ID_CLAIM` (default `sub`). Both token decoders — JWT
and opaque/userinfo — read this configured claim, and the derived principal ID
is what publish permission policies are matched against (the
`principal == Principal::"..."` value above). Set it to `oid` for Azure AD,
where `sub` is pairwise per client-app while `oid` is the stable, tenant-wide
object ID.

**WARNING — this is a one-way, deploy-time choice that must be consistent
across the whole Notification Service.** This service shares the same identity
derivation (`notification_common`) as the Event Consumer Service, where the
principal ID is hashed into durable-queue names
(`event-queue:{hashed_principal_id}:{queue_id}`). Changing
`OMNI_EVENTS_PRINCIPAL_ID_CLAIM` re-derives every user's principal ID, so it
must be set to the same value here and on the Event Consumer Service. Changing
it on an existing deployment will not only break permission-policy matching for
publishers, it also **orphans all previously-created durable queues on the
consumer side** — clients can no longer reach them and the buffered messages
they hold are stranded. Choose the claim before first deploy and never hot-swap
it on a live deployment.

## Building and reimplementing

This service is published source-available in the
`NVIDIA-Omniverse/usd-content-workspace` repository, under
`notifications/reference/event-aggregation-service`. That folder carries the
reference implementation together with the customer-facing API material:

- `BUILDING.md`, the from-source build guide (`uv sync` / `uv run start`).
- `openapi.yaml`, `conformance_tests/`, and `AGENTS.md`, a guide to building a
  protocol-compatible implementation from the contract.

The publisher Protocol Buffer definitions are published separately, under
`notifications/api` in the same repository, and the customer documentation for
both notification services under
`notifications/reference/event-consumer-service/docs/`.

## License

This project is licensed under the Apache License, Version 2.0. See the
[LICENSE](LICENSE) file for the full license text.

## Contributing

This project is currently not accepting contributions.