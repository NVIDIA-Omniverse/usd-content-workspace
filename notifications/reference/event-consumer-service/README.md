# Event Consumer Service

Part of the broader Omniverse Notification Service.
The Event Consumer Service allows clients to consume events
that have been published to RabbitMQ via the Event Aggregation Service.

## Usage

The service exposes two API surfaces: a gRPC API (primary, port 50052) and an
optional REST/HTTP API (port 8000, enabled with `OMNI_EVENTS_REST_API_ENABLED`).
Both expose the same operations — consuming durable and non-durable events and
managing durable queues.

This service's source is published in the
`NVIDIA-Omniverse/usd-content-workspace` repository under
`notifications/reference/event-consumer-service`. To build a client, use the
published API contract:

- The consumer Protocol Buffer definitions, published once for all notification
  services under `notifications/api` in the same repository
  (`notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto`).
- The `openapi.yaml` REST specification and the `conformance_tests/` suite, in
  this service's folder.
- `AGENTS.md`, in this service's folder, a guide to building a compatible
  implementation from the contract.

For usage guides to both notification services' APIs — getting started,
consuming events, publishing events, permissions, and the API reference — see
the `docs/` directory in this service's folder, which also carries the complete
rendered manual as `docs/docs.pdf`.

### Authorization

A bearer token must be passed with each request (except when authentication is
disabled), and a permission check will be made based on the event type.
If a more detailed authorization check is necessary based on the contents of the
event messages, then a service should be created to sit between end clients
and this service. For each client "consume" request, it should open a connection
to this service to consume events,
and then check the authorization of each event before returning them to the client.

#### Authentication mode (principalSource)

How the principal (user) is derived from the request, and whether a token is
required at all, is selected by a single first-class setting,
`OMNI_EVENTS_PRINCIPAL_SOURCE`. It is the sole gate for authentication behavior.
Its built-in default is `jwtUnverified`, but that mode is refused at startup
unless `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true` is also set, so a deployment
must choose a mode explicitly. With the Helm chart, leave `principalSource`
unset and configure OpenID (`openId.openIdConfigurationUri` plus a `default`
client registration or `openId.additionalJwtAudience`): the chart then selects
`jwt`, in which the service verifies every token itself. Without that OpenID
configuration and without an explicit `principalSource`, the chart refuses to
render. It never selects `jwtUnverified` on its own (see the trust boundary
below). The modes are:

- **`disabled`** — no bearer token is required, no principal is derived, and no
  authentication middleware runs. Permission enforcement cannot be used in this
  mode (`OMNI_EVENTS_PERMISSIONS_ENABLED` must be false).
- **`jwtUnverified`** — the JWT payload is decoded to extract the principal, but
  the token's signature, issuer, audience, and expiry are **not** verified (see
  the trust boundary below). Requires the explicit opt-in
  `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`; without it the service refuses to
  start.
- **`jwt`** — the token is validated against the IdP's JWKS. Requires
  `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES`.
- **`opaque`** — the token is validated via the IdP's `/userinfo` endpoint.
  Requires `OMNI_EVENTS_OPENID_CONFIGURATION_URI`.
- **`userInfoService`** — the principal is resolved by calling
  `GET {OMNI_EVENTS_USER_INFO_SERVICE_URI}/api/v1alpha/users/me` with the caller's
  forwarded bearer token. Requires `OMNI_EVENTS_USER_INFO_SERVICE_URI`.

**Trust boundary for `jwtUnverified`.** In the `jwtUnverified` mode the
service does NOT verify the token's signature, audience, or expiry — it only
decodes the payload to read the principal claim, so it trusts any token that
reaches the pod. Use it only when every path to the pods verifies tokens
first. The Envoy auth extension on the chart's HTTPProxy verifies only the
requests that come through the HTTPProxy: callers inside the cluster can reach
the Kubernetes Service directly and bypass it, so the Service must also be
unreachable directly (for example through a NetworkPolicy). Without that, use
`jwt` or `opaque` so the service validates the token itself. Because of this,
the mode is a deliberate opt-in: set `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`
yourself (with the Helm chart, `principalSource: jwtUnverified` together with
`allowUnverifiedJwt: true`; the chart refuses either one without the other).

> **Migration.** Earlier releases accepted unverified tokens by
> default. The service now refuses to start in `jwtUnverified` mode without
> `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`, and the Helm chart refuses at
> render time every authentication combination the service would refuse at
> startup, so such a values file fails `helm template` / `helm upgrade`
> instead of crash-looping the pods.
>
> A chart deployment that sets OpenID (`openId.openIdConfigurationUri` plus a
> `default` client registration or `openId.additionalJwtAudience`) but no
> `principalSource` moves from `jwtUnverified` to `jwt` on upgrade, with no
> values change. The service then verifies every token itself, so check
> before upgrading that your callers' tokens pass:
>
> - An expired token is refused with `401` / `UNAUTHENTICATED` when the
>   request or stream is opened; `jwtUnverified` accepted it.
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
> trust boundary above holds: every path to the pods verifies tokens first,
> which takes the HTTPProxy's auth extension plus a control (for example a
> NetworkPolicy) that keeps the Service unreachable directly.

#### When the token is checked

Every request to an authenticated endpoint is checked before the operation it
asks for begins. The service resolves the bearer token through the configured
`OMNI_EVENTS_PRINCIPAL_SOURCE` ahead of the handler and refuses a token it
cannot resolve with `401` (REST) or `UNAUTHENTICATED` (gRPC), whichever
operation was called. The token itself is not re-validated part-way through a
request: once accepted at entry, the identity resolved from it is held for the
whole request.

**Long-lived streams.** A consume stream is a single request that can run for
hours, and the authentication check above is not re-run on it. Authorization of
the events it delivers is separate: each event is authorized against the
credentials the stream was opened with, so an authorization error (`401` or
`403`) can occur at any point during a streaming session, not only at the
initial connection. If the token lapses while the stream is active, the server
terminates the stream with that error. Reconnect to the same queue with
refreshed credentials — for a non-durable queue use the `reconnect_token` or
`Last-Event-ID`; for a durable queue use the same `queue_id`.

An identity provider the service cannot reach is reported as `500` / `INTERNAL`
rather than as an authentication failure, so a provider outage is
distinguishable from a bad credential. A token signed with a key the provider
does not publish is still `401`.

A request whose token cannot be resolved is answered as an authentication
failure even when the pod it reaches is at its stream capacity: the response is
`401` / `UNAUTHENTICATED`, not the capacity refusal (`503` /
`RESOURCE_EXHAUSTED`).

#### Endpoints served without a bearer token

- `GET /api/v1/health` and `GET /api/v1beta/health`.
- `GET /api/v1/metrics/channel-pool` and `GET /api/v1beta/metrics/channel-pool` —
  scraped by the channel-pool autoscaler, which presents no token.
- `GET /docs`, `GET /docs/oauth2-redirect`, `GET /openapi.json`, and `GET /redoc` —
  the generated API documentation, which serves the API contract and nothing else.

Every other endpoint requires a bearer token unless
`OMNI_EVENTS_PRINCIPAL_SOURCE` is `disabled`.

#### Principal identity claim

The claim that identifies the principal (user) is configurable via
`OMNI_EVENTS_PRINCIPAL_ID_CLAIM` (default `sub`). This claim is read in every
mode that derives a principal — `jwtUnverified`, `jwt`, and `opaque` — and the
`userInfoService` response resolves to the same principal value; only `disabled`
omits it entirely. Set it to `oid` for Azure AD, where `sub` is pairwise per
client-app while `oid` is the stable, tenant-wide object ID.

**The configured claim must be present, and non-empty, in every token the
service accepts.** Resolving the principal is what validates a request's token,
so a token that carries no value for the configured claim — the claim absent, or
present but empty — is rejected as an authentication failure. Pick a claim your
identity provider issues for every principal that consumes events, service
accounts included — `sub` and `oid` do not always behave the same way for
non-interactive principals as they do for users.

**Keep the claim consistent across the notification services.** This service and
the Event Aggregation Service decode tokens through the same shared library, so
they resolve a caller to the same principal identity only when both read the
same claim. A deployment that configures a different claim in each service
resolves one caller to two different identities.

Durable-queue naming does not depend on this setting. A durable queue is
addressed by its `queue_id` alone, so a deployment can change the claim without
losing access to existing queues or the messages they hold.

For self-hosted operators: the Helm chart's own default for
`openId.principalIdClaim` is `"sub"`; some deployments overlay it to `oid` (for
example, Azure AD, where `oid` is the stable tenant-wide object ID). Set the
same value everywhere in the deployment.

### Choosing a durable queue id

`CreateDurableQueue` accepts an optional `queue_id`. When you omit it the
service generates one; when you supply it, that id names the queue.

**A supplied `queue_id` must be unique across the whole deployment.** Queue ids
share a single namespace. Creation is ensure-exists: if a queue with the
supplied id already exists — regardless of who created it or which
`filter_groups` the call carries — the call attaches to that existing queue and
returns it with its existing filters and `created` set to `false`. Two callers
that pick the same id therefore share one queue: each receives the other's
events, and either can delete it.

The single namespace governs every durable operation, not creation alone: the
`queue_id` given to a consume, update, or delete call must be the id of a queue
that was created by (or for) your client.

Pick a UUID, or a namespaced convention such as `<service-name>-<purpose>`
(`thumbnailer-uploads`, say). Never use a bare generic word like `events`.

An id is 1 to 200 characters, each one of `A-Z`, `a-z`, `0-9`, `.`, `_`, or `-`.
The rule applies wherever an id is accepted — creating, consuming from,
updating, and deleting a queue — so an id that breaks it is refused (REST `422`,
gRPC `INVALID_ARGUMENT`) before the request reaches the message broker.

**Per-queue isolation is not enforced.** The three durable-queue permissions are
deployment-wide rather than per-queue: a principal granted
`consume-durable-queues` can consume from any durable queue whose id it knows,
one granted `delete-durable-queues` can delete any of them, and
`create-durable-queues` authorizes both creating a queue and updating the
filters of any existing one. Grant these permissions deliberately — to the
service accounts that consume events, rather than to broad user groups — and
treat a `queue_id` as configuration rather than as a secret that protects a
queue.

A queue name carries nothing about who created it, so the service records that
pair in its own logs instead. Every durable-create call writes one `INFO` line.
A call that actually declares the queue writes:

```
Durable queue created: id=<queue_id> name=event-queue:<queue_id> principal=<principal-id>
```

A call that finds the id already in use attaches to the existing queue and
writes:

```
Durable queue attached (existing): id=<queue_id> principal=<principal-id>
```

`principal` reads `authn-disabled` when authentication is off. Searching the
logs for a `queue_id` therefore shows every principal that has created against
it — the one that made the queue, and each one that has since attached to it —
which is how a shared id becomes visible. The lines cover create calls only:
they say nothing about who is consuming from the queue.

### Handling the reconnect token

A non-durable stream's `connected` event returns a `reconnect_token`. Presenting
the token reattaches to the queue it names, which is what makes it usable both
for reconnecting after a drop and for fanning one queue out across parallel
clients, which then split the queue's events between them. When several clients
share a queue this way, at most one of them may change its filters, and that
client alone can follow the acknowledgement-driven replay recipe below. A token
stops resolving once its queue is gone — a queue outlives its last consumer only
for the reconnect-grace window (`OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS`,
default 20 s).

The token identifies a queue; it is not a secret and it grants no access on its
own. Presenting it requires a valid bearer token, and every event delivered on
the stream is authorized against the permissions of the principal that presented
it — storage events against that principal's storage permissions, every other
event against the permissions-service policy. A principal your policy does not
allow to consume receives nothing, with or without the token, and each principal
attached to a queue is authorized independently of the others. What holding a
token buys is attachment: consumers on one queue split its events between them
rather than each receiving every event. So keeping a token inside the
application that received it is what ensures only your own authorized consumers
share — and split — that queue's events.

The service never writes a token down. Its log lines, traces, metrics, and its
own HTTP access log identify a non-durable stream by a one-way reference derived
from the token — `sha256-` followed by 12 hexadecimal characters — never by the
token itself. That includes a request that sends the token as a query parameter:
the access line records the reference in the token's place, and a trace of the
request records the URL without its query string.

The reference is the first 12 hex characters of the SHA-256 digest of the token,
and it is the same value on every pod, across restarts, and on every surface, so
an operator can follow one stream through logs, traces, metrics, and access lines
and can compute the reference for a token a client reports — but cannot recover a
token from a reference. Durable identifiers are unaffected: a durable `queue_id`
and its queue name appear verbatim, which is what makes a queue findable in the
broker.

On the REST surface, the `Last-Event-ID` request header is the recommended way
to send a token back, and the `reconnect_token` query parameter is deprecated in
favour of it. Infrastructure between a client and the service — a reverse proxy,
API gateway, or CDN — logs request URLs, and the service cannot redact what it
does not write. The header carries the same value, never appears in a URL, and is
what a browser's built-in SSE reconnect uses. The query parameter is still
accepted and carries the token the same way; it is deprecated as guidance, not
withdrawn.

The header carries every operation the query parameter does, a filter update
included: send `filter_groups` and `previous_filter_groups` alongside the header
and the stream's filters are updated. Sending `previous_filter_groups` is what
makes a request a filter update — a request that carries the token alone resumes
the stream on the filters it already had, which is what a browser's automatic
reconnect asks for when it replays the URL the stream was opened on. Clients can
therefore move from the query parameter to the header without giving anything
up.

A token that comes back must be 1 to 200 printable ASCII characters — no spaces,
control characters, or non-ASCII — or the request is refused (REST `422`, gRPC
`INVALID_ARGUMENT`) with a message that names the field and the rule and quotes
none of the token. The service mints tokens as random identifiers well inside
that rule, so a client echoing back what it was given never meets it; what the
rule catches is a value that could not name a broker queue at all.

**Resuming with the token alone resumes the queue with the bindings it already
holds.** That preserves what the queue buffered only while your filter set still
matches the last set the service confirmed. A client that changes filters while
a gRPC stream is open therefore tracks which of its updates the service
acknowledged and replays the rest after reconnecting, as the next section
describes. A client that does not follow that recipe — or that reaches a
server which does not acknowledge filter updates — has no safe way to resume a
changed filter set by token, and opens a fresh stream with `filter_groups` and
no `reconnect_token` instead.

### Acknowledged filter updates on a gRPC non-durable stream

`ConsumeNonDurableEvents` is bidirectional: a client changes the filters its
queue is bound to by sending another `ConsumeNonDurableEventsRequest` carrying
the new set in `filter_groups` and the set it replaces in
`previous_filter_groups`. The service acknowledges each update it processes,
which is what lets such a client resume by reconnect token.

**The capability is advertised per connection.** Every response of a stream
served by a server that acknowledges filter updates carries
`supports_filter_update_acks` set to `true`. It is a statement about the server
serving the stream rather than about the individual response, so a client reads
it off any response it receives — including the first, as soon as the stream
opens. A server that leaves it `false` may still apply filter updates, but never
reports them, so a client cannot know which of its updates took effect.

**What an acknowledgement is.** One response carrying a `filter_update_ack` and
no events, reporting that the service acted on the filter set the request named.
Receiving it means the service is done with that request: any bind and unbind
calls the request led to have returned. It does not report how many calls that
was, and the answer may be none. It carries two fields:

- `applied_filter_groups` — a verbatim copy of the `filter_groups` of the
  request being acknowledged. The service holds no filter state and cannot read
  a queue's bindings back from the broker, so this echoes what you sent rather
  than reporting what the broker holds.
- `unchanged` — `true` when the acknowledged request's `filter_groups`
  equalled its `previous_filter_groups` **as you sent them**, so the service
  issued no bind or unbind call. `false` means the two differed as sent, or
  that the request named no previous set at all, as a fresh connect does. This
  is a statement about the request the service received, not about the queue:
  it does not confirm that the queue's bindings match `applied_filter_groups`,
  and a `false` value does not promise that any bind or unbind call was issued
  — a request that re-sends the same groups reordered, or with a group
  repeated, differs as sent and still changes no bindings.

The first response on a stream carries an acknowledgement when the opening
request named a filter set the service acted on:

- A **fresh connect** — `filter_groups` and no `reconnect_token` — is
  acknowledged with `unchanged` `false`.
- A **reconnect that names `filter_groups` and `previous_filter_groups`
  together** is a filter update applied before the stream yields anything, and
  is acknowledged with `unchanged` set according to whether the two sets were
  equal as you sent them.
- A **reconnect that carries only a `reconnect_token`** names no set, so it gets
  no acknowledgement: the service keeps no filter state and cannot report a set
  it was not told.
- A **reconnect that names `filter_groups` without `previous_filter_groups`** is
  not a resume either. It is an incomplete filter update and is refused with
  `INVALID_ARGUMENT`, so filter groups the service does not apply are never
  acknowledged as applied.

Two kinds of mid-stream request are never acknowledged: one that names neither
`filter_groups` nor `previous_filter_groups`, which is ignored; and one the
service rejects, which ends the stream with `INVALID_ARGUMENT`. Nor is an
update still in flight when the stream ends with a terminal error, as it does
when the broker cancels the consumer (see [A consume stream ends when the
broker drops your consumer](#a-consume-stream-ends-when-the-broker-drops-your-consumer)):
the stream ends and the update is never acknowledged.

**Resuming a stream whose filters changed.** Track every update you send and the
set it replaced, and mark it confirmed when its acknowledgement arrives. After a
drop, reconnect with the `reconnect_token`, then re-send in order each update
you sent but did not see acknowledged, each naming in `previous_filter_groups`
the set it replaced. Each replayed update binds what its set adds and unbinds
what its set drops, so the queue ends on the set you last asked for, and the
events buffered across the drop are still there.

Three caveats bound that recipe:

- **Acknowledgements are ordered against each other, not against events.** They
  arrive in the order the updates were sent, but an acknowledgement and the
  event responses around it are interleaved in no guaranteed order.
- **Replaying more than one unacknowledged update is best-effort.** It walks the
  queue back through each intermediate filter set in turn, and applying a set
  unbinds every resource that set drops, so an event published for a resource
  that an earlier replayed update unbinds is lost until a later replayed update
  binds it again.
- **Exactly one client per queue may change filters.** Other clients sharing the
  `reconnect_token` may consume from the queue, but must not send filter
  updates: each client names the set it believes it replaced, computed from
  acknowledgements that describe only its own updates, so two clients updating
  one queue walk its bindings to a set neither asked for.

On the REST surface there is no mid-stream update and no separate
acknowledgement: filters are changed by reconnecting with `filter_groups` and
`previous_filter_groups`, and the `connected` event that follows is sent only
after that update has been applied, so its arrival is the confirmation.

### Durable-queue lifecycle

A durable queue ends in one of two ways: you delete it with `DeleteDurableQueue`,
or the broker reaps it after it has gone without an attached consumer for the
deployment's idle window — **24 hours by default**, set by
`policyHook.policy.definition.expiresMs` in the Helm chart. When a queue is
reaped, any messages it still holds are **discarded silently**; they are not
dead-lettered, so they do not appear in the dead-letter queue.

Reaping is aligned with the message TTL: a durable queue's messages expire after
24 hours in any case, so what a reap at the default window removes is the empty
queue handle rather than buffered events.

Only an *attached consumer* keeps a queue alive — publishing to it and
re-declaring it do not — and the idle clock starts when that consumer
disconnects or cancels; the consuming-events guide's "Durable Queue Lifetime"
section has the full table for client authors.

**Recovering from a reap.** Consuming from a reaped queue fails loudly rather
than returning an empty stream: `NOT_FOUND` on gRPC, `404` on REST. Recover by
calling `CreateDurableQueue` again with the stored `queue_id` **and** the stored
`filter_groups` — creation is ensure-exists, so the same call works whether the
queue survived or not, and the queue's bindings are rebuilt from the filters,
which is why a client must persist the id and its filter groups together as one
unit of configuration. Events published while the queue was gone are not
recoverable, the same as if the queue's messages had aged out under the message
TTL.

**Tuning the window.** A deployment whose clients are legitimately idle for
longer than the window — a batch job that runs weekly, for example — can raise
`expiresMs` rather than have those clients take the recreate-on-return path
above (the consuming-events guide covers the client-side treatment). Setting
`expiresMs` to `null` disables reaping entirely, so a queue exists until it is
explicitly deleted — at the cost of abandoned queues accumulating on the broker
forever.

### Settling a durable delivery

**Handing a message back for retry.** On the durable consume-with-acks stream
(`ConsumeDurableEventsWithAcks`) a client settles each delivery one of two ways:
it acknowledges the delivery tag in `ack_delivery_tags`, or it negatively
acknowledges (nacks) it by sending that tag in `nack_delivery_tags`. A nack
always **requeues** — there is no discard-on-nack — and the broker redelivers the
message **immediately**, possibly on the same stream. A client that wants a delay
between attempts implements that backoff itself, before it nacks. A tag sent in
both fields on one request is acknowledged; acks settle first. A tag the server
does not recognize — already settled, or from an earlier connection — is skipped
with a log line and does not end the stream.

**Retries are bounded, and the budget is a lifetime one.** Every requeue
increments the message's delivery count, and that count is cumulative over the
message's whole life and never reset: a nack, a client disconnect, and a
broker-side channel close all requeue the message and all spend from the same
budget. Once the count passes `policyHook.policy.definition.deliveryLimit` on the
`event-queue-limits` policy — **10 attempts as shipped**, and tunable per
deployment — the message is dead-lettered instead of redelivered, where it is
available to the dead-letter management API for inspection and replay. The
practical consequence for client authors: a consumer that nacks in a tight loop
while its own downstream is unavailable spends a message's entire budget in well
under a second and dead-letters work that was only briefly unprocessable, so
backoff between nacks is a requirement rather than a refinement.

### A consume stream ends when the broker drops your consumer

This applies to **every** consume stream, not only durable ones — the
non-durable consume path and the dead-letter browse stream share the same
consume loop and get the identical termination.

Every consume stream is attached to its queue by a broker-side consumer, and
the broker can take that consumer
away underneath a healthy-looking connection. It does so when the queue is
deleted, and when it kills the channel the consumer lives on (on durable queues
an acknowledgement timeout is the common cause; see
[Bounding a hung durable consumer](#bounding-a-hung-durable-consumer) under
Deployment). Nothing in the AMQP client surfaces that loss, so the service
checks for it directly while the stream is idle and **ends the stream with an
error** rather than leaving it open against a queue nothing is consuming.
Detection is bounded by `OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS` (default
15 s, stretched by a random 0–15% per stream). Two outcomes, distinguished so a
client knows
which recovery to take. On REST the response status was already sent as `200 OK`
when the stream opened, so the status codes below reach the client inside an SSE
`error` event — with the status in the event's `data` payload — rather than as
the HTTP response status:

- **The queue is gone** — `NOT_FOUND` on gRPC, `404` on REST (an SSE `error`
  event, after which the stream closes). Which recovery applies depends on the
  queue type:
  - *Durable* — **reprovision.** Call `CreateDurableQueue` again with the
    stored `queue_id` **and** `filter_groups`, then reconnect, exactly as for a
    reaped queue.
  - *Non-durable* — **reconnect with your reconnect token** (`queue_id`). A
    non-durable queue outlives its consumer only for the `x-expires` grace
    window (`OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS`, default 20 s); within
    it the queue and its buffered backlog are still there, so a token
    reconnect resumes where the stream left off. Past the window the queue is
    gone and a token reconnect gets `NOT_FOUND` / `404` again — start a fresh
    stream with your `filter_groups` instead (no `queue_id`), which creates a
    new queue.
  - *Dead-letter browse* — the dead-letter queue is service-owned and declared
    at startup, so this outcome means the deployment's dead-letter queue is
    missing on the broker. That is an operator condition, not something a
    client can reprovision.

  Events published while the queue was gone are not recoverable.
- **The consumer was cancelled, or its channel or connection was lost** —
  `INTERNAL` on gRPC, `500` on REST (an SSE `error` event carrying this status,
  after which the stream closes), with exactly this message:

  ```
  Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.
  ```

  The broker-side queue name is recorded in the service's own logs rather than
  returned to you. **Reconnect; do not reprovision.** The queue normally
  survives with everything buffered in it, and messages that were in flight
  unacknowledged are redelivered after you reconnect — either requeued by the
  broker itself, when the loss came with a closed channel or a dropped
  connection, or nacked back onto the queue by the service, when the broker
  cancelled only the consumer and left the channel open. This error does not
  by itself confirm that the queue survived: if the queue no longer exists, the
  reconnect gets `NOT_FOUND` / `404`, and the recovery for a queue that is
  gone, above, applies.

Treat both as terminal for the stream: neither recovers in place, and a client
that ignores the error and waits will simply never receive another event.

## Deployment

This service is built to be deployed in k8s or NVCF.

### Configuration

The app is configured with environment variables:

| Env Var Name                               | Default Value           | Description                                                                                                                                                                                                                                                       |
|--------------------------------------------|-------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| OMNI_EVENTS_RABBITMQ_URI                   | "amqp://127.0.0.1:5672" | The address for the RabbitMQ instance.                                                                                                                                                                                                                            |
| OMNI_EVENTS_GRPC_HEALTH_CHECKING_ENABLED   | True                    | Optionally turn off gRPC health checking servicer. Helm value: `service.grpc.healthCheckingEnabled`.                                                                                                                                                              |
| OMNI_EVENTS_LOG_LEVEL                      | INFO                    | The logging level for the deployed service (DEBUG, INFO, WARNING, ERROR, CRITICAL)                                                                                                                                                                                |
| OMNI_EVENTS_TELEMETRY_ENABLED              | False                   | Whether telemetry is enabled. Endpoint vars must be populated                                                                                                                                                                                                     |
| OMNI_EVENTS_OTLP_TRACING_ENDPOINT          | "localhost:4317"        | OTLP-compatible endpoint for traces                                                                                                                                                                                                                               |
| OMNI_EVENTS_OTLP_METRICS_ENDPOINT          | "localhost:4317"        | OTLP-compatible endpoint for metrics                                                                                                                                                                                                                              |
| OMNI_EVENTS_OTLP_LOGS_ENDPOINT             | "localhost:4317"        | OTLP-compatible endpoint for logs                                                                                                                                                                                                                                 |
| OMNI_EVENTS_GRPC_PORT                      | "[::]:50052"            | Set the port to use for the gRPC server.                                                                                                                                                                                                                          |
| OMNI_EVENTS_REST_PORT                      | 8000                    | Set the port to use for the HTTP server.                                                                                                                                                                                                                          |
| OMNI_EVENTS_CORS_ALLOW_ORIGINS             | []                      | Browser origins the REST server answers CORS requests for, with credentials allowed; a comma-separated list or a JSON array. Empty adds no CORS middleware, so browsers receive no Access-Control-Allow-Origin header. Write each entry exactly as a browser sends the Origin header: `http` or `https`, an ASCII (punycode) host, no default port (`:80` / `:443`), and no path or trailing slash, for example `https://app.example.com`. The service refuses `*`, `null`, and a malformed entry at startup. The Helm chart sets it from `service.rest.corsAllowOrigins` and refuses at render time only `*`, `null`, a value that is not a list, and an entry that is not a string; a malformed origin renders and is refused at startup. |
| OMNI_EVENTS_REST_API_ENABLED               | False                   | Enable or disable the HTTP server.                                                                                                                                                                                                                                |
| OMNI_EVENTS_PERMISSIONS_ENDPOINT           | None                    | The endpoint to use for the Permissions API — the target the permission-service gate checks against. Required when OMNI_EVENTS_PERMISSIONS_ENABLED is true; inert (ignored) when that gate is false. This endpoint backs only the permission-service gate; it does not configure the storage-service gate (see OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED).                                                                                                                                                   |
| OMNI_EVENTS_PERMISSIONS_TTL_SECONDS        | 600                     | Set the time to live for the permissions cache.                                                                                                                                                                                                                   |
| OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH     | None                    | If fine-grained, per-event_type permissions are required, then the path to a yaml file that defines them should be set. The yaml file is described in the [permissions section](#fine-grained-permissions) below.                                                 |
| OMNI_EVENTS_PRINCIPAL_SOURCE               | "jwtUnverified"         | The single first-class gate selecting how the principal (user) is derived and whether a token is required. One of: `disabled` (no token required, no principal, no auth middleware), `jwtUnverified` (the built-in default, refused at startup unless OMNI_EVENTS_ALLOW_UNVERIFIED_JWT is true — decodes the JWT payload to extract the principal but performs NO signature/audience/expiry verification, so it trusts any token that reaches the pod — see the [Authorization section](#authorization)), `jwt` (validates the token against the IdP JWKS; requires OMNI_EVENTS_OPENID_CONFIGURATION_URI and OMNI_EVENTS_JWT_AUDIENCES), `opaque` (validates the token via the IdP `/userinfo` endpoint; requires OMNI_EVENTS_OPENID_CONFIGURATION_URI), `userInfoService` (resolves the principal via `GET {OMNI_EVENTS_USER_INFO_SERVICE_URI}/api/v1alpha/users/me` with the forwarded bearer token; requires OMNI_EVENTS_USER_INFO_SERVICE_URI). The Helm chart sets it from `principalSource`, and selects `jwt` when that is left empty and OpenID is configured. |
| OMNI_EVENTS_ALLOW_UNVERIFIED_JWT           | False                   | Explicit opt-in for OMNI_EVENTS_PRINCIPAL_SOURCE=`jwtUnverified`. Set it to true only when every path to the pods verifies the bearer token first: the Envoy auth extension covers only requests through the HTTPProxy, so the Service must also be unreachable directly (for example through a NetworkPolicy). Without it, `jwtUnverified` fails at startup. Inert in every other mode. The Helm chart renders it true only when the values set both `principalSource: jwtUnverified` and `allowUnverifiedJwt: true`. |
| OMNI_EVENTS_PERMISSIONS_ENABLED            | False                   | The gate for the permission-service authorization path. When true, requests checked via the Permissions API (consume/create/delete durable queues, dead-letter management, and durable-queue storage events) are enforced; when false, those checks pass. Requires a non-`disabled` OMNI_EVENTS_PRINCIPAL_SOURCE and OMNI_EVENTS_PERMISSIONS_ENDPOINT to be configured. This gate does not cover the storage-service path — non-durable storage events are gated independently by OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED.                                          |
| OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED    | False                   | The independent gate for the storage-service authorization path. When true, non-durable storage events checked via the Storage API are enforced; when false, those checks pass. Independent of OMNI_EVENTS_PERMISSIONS_ENABLED — either gate may be on or off without the other. Requires a non-`disabled` OMNI_EVENTS_PRINCIPAL_SOURCE and the storage endpoints (`permissions.storage_permissions.endpoints`) to be configured.                                          |
| OMNI_EVENTS_USER_INFO_SERVICE_URI          | ""                      | Base URI of the user-info service. Consulted only when OMNI_EVENTS_PRINCIPAL_SOURCE is `userInfoService`, where the principal is resolved via `GET {uri}/api/v1alpha/users/me` with the caller's forwarded bearer token. Required in that mode.                                                                                                                                                                       |
| OMNI_EVENTS_OPENID_ENABLED                 | False                   | Infra-only field (Envoy authExtension); it does not gate token-requirement or permission checks. Authentication behavior is keyed on OMNI_EVENTS_PRINCIPAL_SOURCE and permission enforcement on OMNI_EVENTS_PERMISSIONS_ENABLED (`permissions.enabled`).                                                                                                                                                              |
| OMNI_EVENTS_OPENID_CONFIGURATION_URI        | ""                      | The OpenID Connect discovery endpoint (/.well-known/openid-configuration) for fetching OIDC provider metadata and JWKS. Required when OMNI_EVENTS_PRINCIPAL_SOURCE is `jwt` or `opaque`.                                                                          |
| OMNI_EVENTS_JWT_AUDIENCES                  | ""                      | Comma-separated list of expected audience values for JWT tokens (the `aud` claim). Required when OMNI_EVENTS_PRINCIPAL_SOURCE is `jwt`.                                                                                                                          |
| OMNI_EVENTS_PRINCIPAL_ID_CLAIM             | "sub"                   | The JWT/userinfo claim used as the principal (user) identity. Set to `oid` for Azure AD, where `sub` is pairwise per client-app and `oid` is the stable tenant-wide object ID. A token that carries no value for the configured claim fails authentication, and the same claim must be configured across the notification services so a caller resolves to one identity — see the [Authorization section](#principal-identity-claim). Durable-queue naming does not depend on this setting. |
| OMNI_EVENTS_AUTH_CONTEXT_TTL               | 1800                    | How long, in seconds, the identity provider's JWKS signing keys are cached before they are fetched again (default: 30 minutes). Applies only when OMNI_EVENTS_PRINCIPAL_SOURCE is `jwt`, and is inert in every other mode. It does not govern how long a resolved principal is cached for a request. |
| OMNI_EVENTS_REDIS_ENABLED                  | False                   | Enable Redis (or Valkey) for storage permissions caching. Must be explicitly enabled; when enabled, OMNI_EVENTS_REDIS_URI must be provided.                                                                                                                       |
| OMNI_EVENTS_REDIS_URI                      | ""                      | The Redis (or Valkey) connection string - either a uri or an Azure Cache for Redis style connection string. Required when OMNI_EVENTS_REDIS_ENABLED is True. Provides a performance improvement when clients disconnect and reconnect to an event consumer queue. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT  | 8                       | The number of clients that may share a single RabbitMQ channel. On the non-durable pool this is a packing target rather than a hard limit: once every channel is open and full, additional streams are packed onto the least-loaded open channel instead of being refused (see OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED). On the durable pool it is a hard limit, which stays strict by construction. Lower values provide more isolation (reduced blast radius from channel errors), higher values pack more concurrent clients per pod. Helm value: `rabbitmq.channel_client_limit`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_SIZE     | 100                     | Maximum size of the RabbitMQ channel pool. `channel_pool_size * channel_client_limit` is the pod's packing target — the capacity that readiness and the channel-pool autoscaler are fed from — and not a refusal point: the non-durable pool admits past it into its overflow band, and the hard refusal is the memory-safe stream cap (see OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD below). See the Capacity and admission tiers subsection under Deployment. Helm value: `rabbitmq.channel_pool_size`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_MIN_SIZE | 75                      | Minimum number of pre-warmed channels in the pool at startup. Helm value: `rabbitmq.channel_pool_min_size`.                                                                                                                                                                                                      |
| OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED | True                 | Enables the overflow band on the non-durable channel pool: when the pool is at its maximum size and every channel is already at OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT, a new stream is packed onto the least-loaded open channel instead of being refused. The durable pool is strict by construction and has no equivalent setting. Helm value: `rabbitmq.channel_overflow_enabled`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT | 15                    | The number of unacknowledged messages RabbitMQ may push to a non-durable consumer channel. It sizes the worst-case prefetch buffer a stream can hold, so it is also the divisor in the computed per-pod stream cap: lowering it raises the cap under the same broker memory budget, at the cost of a smaller in-flight buffer per stream. Helm value: `rabbitmq.channel_prefetch_count`. |
| OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB | 1024                  | The worst-case broker memory budget, in mebibytes (MiB), for non-durable prefetch buffers. Fleet-wide, not per pod: the per-pod stream cap divides this budget by OMNI_EVENTS_SCALER_MAX_REPLICAS. Must be >= 1. Helm value: `rabbitmq.broker_stream_memory_budget_mib`. |
| OMNI_EVENTS_SCALER_MAX_REPLICAS            | 10                      | The maximum number of service pods the fleet may run, and the divisor that turns the fleet-wide OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB into a per-pod stream cap. Shared with the channel-pool autoscaler so the fleet ceiling is configured once: the Helm chart templates it into the service pods from `channelPoolAutoscaling.maxReplicas` when autoscaling is enabled and from `replicaCount` when it is disabled, so the default here applies only to non-Helm deployments. Raising it lowers the per-pod cap — see the Capacity and admission tiers subsection under Deployment before changing it. Must be >= 1. |
| OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD | None                  | An absolute cap, in concurrent non-durable streams per pod, for operators who have measured their own deployment. When set it wins unconditionally; when unset the cap is computed from OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB, OMNI_EVENTS_SCALER_MAX_REPLICAS and the configured prefetch. There is no off switch: unset means computed, set means absolute. Must be >= 1. Helm value: `rabbitmq.max_non_durable_streams_per_pod`. |
| OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD | None                  | The per-pod stream count at which re-routable non-durable streams are refused, reserving the band above it for pinned streams that scaling out cannot rescue. Re-routable means every REST/SSE stream, plus gRPC streams that arrived through the proxy (carrying the `X-Via-Proxy` stamp); pinned streams are subject only to the cap. Clamped at admission into [packing target, cap] — unset means the floor-division midpoint of the live band, which on a width-1 band is the packing target itself. Setting it at or above the cap disables the shed tier, which is the documented posture for single-replica deployments. Helm value: `rabbitmq.max_re_routable_streams_per_pod`. See the Capacity and admission tiers subsection under Deployment. Must be >= 1. |
| OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS | 80                  | The readiness reserve. Its unit is non-durable STREAM slots, not channels, despite what the name suggests; the name is held stable so that renaming the OMNI_EVENTS_ var never silently reverts a tuned deployment. It is a readiness/scaling signal that refuses nobody: the channel-pool metrics feed subtracts it from the pool's stream capacity so the autoscaler reacts before the soft threshold is reached, and a pod whose free slots fall below it reports NOT_SERVING while still admitting streams. At the shipped defaults it leaves an 800 soft threshold − 80 reserve = 720 usable slots in the scaling signal. Helm value: `rabbitmq.readiness_min_available_channels`. |
| OMNI_EVENTS_RABBITMQ_CHANNEL_WAIT_MS       | 500                     | How long, in milliseconds, a non-durable stream waits for a channel from the pool before the pool reports exhaustion. Under two-tier admission a wait expiry is not the refusal point — the memory-safe per-pod stream cap is; this only bounds how long a checkout blocks while the pool is momentarily saturated. Helm value: `rabbitmq.channel_wait_ms`. |
| OMNI_EVENTS_DURABLE_CONSUMER_PREFETCH_COUNT | 20                    | Per-channel QoS prefetch applied to durable consumers (both the with-acks and server-delayed-ack paths), bounding the in-flight messages requeued per disconnect. Applied per checkout via set_qos, so it — not the durable pool's base prefetch — governs durable-consumer broker memory. Must be >= 1. Helm value: `rabbitmq.durable_consumer_prefetch_count`. |
| OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS | 15.0                    | How often, in seconds, an idle consume stream re-checks that its own broker-side consumer is still attached. Applies to every consume path — durable, non-durable, and dead-letter browse. The broker gives no notification when it cancels a consumer or kills the channel it lives on, so this poll is what turns a silently dead consumer into a prompt terminal error on the stream — it bounds detection latency and nothing else. Each stream's individual deadline is stretched by a random 0–15% so the streams sharing a pod's AMQP connection do not all wake together; the stretch is upward-only, so a deadline never lands below the configured value and averages about 7.5% above it. Lower it for faster failover, raise it to trim the (already tiny) idle wakeup cost; the check is in-process and issues no broker traffic. Must be >= 1.0. Env-var only; no chart value. |
| OMNI_EVENTS_DEAD_LETTER_BROWSE_PREFETCH_COUNT | 10                  | Per-session QoS prefetch applied to a dead-letter browse session, bounding how many dead-letters one browse holds in-flight on the single global DLQ. Applied per checkout via set_qos. Must be >= 1. Env-var only; no chart value. |
| OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS  | 20                      | The reconnect-grace window, in seconds, for a non-durable queue: after a stream drops, its queue stays bound and buffers matching events for this long (`x-expires`) so a client can reconnect with its reconnect token and miss nothing. Helm value: `rabbitmq.non_durable_queue_ttl_seconds`. |
| OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS  | 0.0                     | The endpoint-settle delay, in seconds, at the start of shutdown. On SIGTERM the pod keeps serving normally for this long — readiness still reports `SERVING` and new streams are still admitted — so the endpoint change a terminating pod triggers has time to reach the clients that dial the readiness-gated main Service directly. Only when it elapses does the pod begin draining, so app-side refusal starts at SIGTERM plus this delay, not at SIGTERM. The application default is `0.0`, which skips the phase and begins the drain immediately on SIGTERM; the Helm chart ships `15` through `shutdown.settleDelaySeconds`, so a chart-based deployment settles for 15 s unless it sets that value. It is dead time added to every pod deletion and rollout, so size it to worst-case endpoint propagation (single-digit to low-tens of seconds) and no larger; the pod's grace period must cover it together with the drain window and the post-drain teardown (see the Graceful drain on scale-down and rollout subsection under Deployment). Helm value: `shutdown.settleDelaySeconds`. |
| OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS  | 30                      | The bounded graceful-drain window, in seconds, on shutdown. Once the endpoint-settle delay above has elapsed the pod enters a draining state (readiness flips to NOT_SERVING, new non-durable streams are refused) and waits up to this long for its in-flight non-durable streams to finish before the connector closes and any streams still held are actively reclaimed. A longer window drains more streams cleanly but slows every pod deletion and rollout by up to this much; the pod's grace period must cover it together with the settle delay and the post-drain teardown (see the Graceful drain on scale-down and rollout subsection under Deployment). Helm value: `shutdown.drainWindowSeconds`. |
| OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS | 0.5               | How often, in seconds, the drain loop re-reads the non-durable pool's in-flight checkout count during the drain window to detect an early-drained pool and finish shutdown sooner. Has no effect once the drain window elapses. Helm value: `shutdown.drainPollIntervalSeconds`. |
| OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_SIZE | 100                 | Maximum size of the DURABLE channel pool, which serves durable-queue operations and DLQ browse against its own budget, independent of the non-durable pool. Deliberately generous relative to the handful of long-lived durable queues a deployment runs — never "small" — so durable operations never queue behind non-durable fan-out. Helm value: `rabbitmq.durable_channel_pool_size`. |
| OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_MIN_SIZE | 0               | Minimum number of pre-warmed channels in the durable pool; 0 leaves it empty until a durable checkout first needs it. Helm value: `rabbitmq.durable_channel_pool_min_size`. |
| OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_PREFETCH_COUNT | 500             | Base per-channel QoS for the durable pool. This setting is effectively INERT: every durable consumer applies OMNI_EVENTS_DURABLE_CONSUMER_PREFETCH_COUNT and every DLQ browse applies OMNI_EVENTS_DEAD_LETTER_BROWSE_PREFETCH_COUNT via per-checkout set_qos, and declaration-only channels host no consumers, so raising or lowering this has no broker-memory effect. Helm value: `rabbitmq.durable_channel_prefetch_count`. |
| OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_WAIT_MS | 500                   | How long, in milliseconds, a durable operation waits for a channel from the durable pool before it reports exhaustion. Helm value: `rabbitmq.durable_channel_wait_ms`. |
| OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_CLIENT_LIMIT | 1                 | Clients per channel on the DURABLE pool. A strict bound (unlike the non-durable pool's packing target): the durable pool never packs past this limit, and has no overflow band, shed tier, or memory cap. Helm value: `rabbitmq.durable_channel_client_limit`. |
| OMNI_EVENTS_CACHE_WRITER_BUFFER_CAP        | 10000                   | Max Ops (units: ops) queued in a single CacheWriter's buffer before drop-oldest kicks in. The auth-cache write-through batcher runs one buffer per non-durable queue and presenting principal, so a queue attached by two principals has two buffers, each with this cap.                                                                                                                              |
| OMNI_EVENTS_DEAD_LETTER_EXCHANGE_NAME      | "events.dlx"            | Name of the dead-letter exchange that durable queues route to when a message is rejected or expires. Declared by the service at startup.                                                                                                                          |
| OMNI_EVENTS_DEAD_LETTER_QUEUE_NAME         | "events.dlq"            | Name of the dead-letter queue bound to the dead-letter exchange, which captures dead-lettered messages from durable queues.                                                                                                                                       |
| OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS | 1209600000            | Per-message time-to-live (in milliseconds) for messages in the dead-letter queue, after which they expire. The dead-letter queue is a self-bounded drop-head queue with no dead-letter exchange of its own, so expired messages are simply discarded rather than dead-lettered onward. Defaults to 14 days. Must be >= 1. **Create-time only**: it is applied when the dead-letter queue is first declared on a broker that does not yet have it, and RabbitMQ queue arguments are immutable afterwards. Changing it on a deployment whose dead-letter queue already exists does NOT retune the live queue — the service logs one ERROR per (re)connect beginning `Dead-letter queue argument mismatch` and starts normally against the existing bounds. See [Retuning the dead-letter queue's bounds](#retuning-the-dead-letter-queues-bounds). |
| OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH   | 1000000                 | Maximum number of messages retained in the dead-letter queue. The dead-letter queue is a self-bounded drop-head queue with no dead-letter exchange of its own: when the limit is reached the oldest messages are dropped, so the dead-letter queue never blocks producers and never dead-letters onward. Must be >= 1. **Create-time only**, exactly as with OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS above: changing it on a deployment whose dead-letter queue already exists is inert apart from the `Dead-letter queue argument mismatch` ERROR at startup. See [Retuning the dead-letter queue's bounds](#retuning-the-dead-letter-queues-bounds). |

#### Local Environment Variables

For local evaluation, environment variables can be set in a `.env` file in the
project root. Create a file named `.env` containing the required environment
variable definitions.

Example `.env` file contents, for a local run without tokens (the service
refuses to start without an authentication mode it can use; see
[Authentication mode](#authentication-mode-principalsource)):

```
OMNI_EVENTS_PRINCIPAL_SOURCE=disabled
OMNI_EVENTS_REDIS_URI=redis://localhost:6379
```

### Capacity and admission tiers

Non-durable admission is tiered. A pod does not simply fill up and start refusing: it passes through four boundaries, and only the outer two refuse anything. Each boundary is moved by a different setting.

1. **Soft threshold — the packing target.** `OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_SIZE * OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT`. This is the density the pod is provisioned to pack at, and it is the number readiness and the channel-pool autoscaler are fed from. It refuses nobody. A pod at or above it reports `NOT_SERVING` on the gRPC health check — in fact it trips a little earlier, once free stream slots fall below `OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS` — and keeps admitting. Going unready removes the pod from the readiness-gated main Service (cluster-internal traffic) but not from the Envoy-facing Service, so proxied clients still reach it.
2. **Shed threshold — where re-routable streams are refused.** `OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD` (Helm value `rabbitmq.max_re_routable_streams_per_pod`). Above it, streams a client can retry onto another pod are refused, so the band above stays available to pinned traffic that scaling out cannot rescue. Re-routable means every REST/SSE stream, unconditionally, plus any gRPC consume stream carrying the `X-Via-Proxy` stamp. The value is clamped at admission into `[soft threshold, cap]`; left unset, it sits at the floor-division midpoint of that band — on a width-1 band that is the soft threshold itself.
3. **Overflow band — pinned streams keep being admitted.** Above the soft threshold and up to the cap (the shed threshold sits inside this band: re-routable streams are refused at that interior line, while pinned streams continue through the whole band), a stream arriving when every channel is already at `OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT` is packed onto the least-loaded open channel instead of being refused (`OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED`, non-durable pool only — the durable pool is strict by construction).
4. **Memory-safe stream cap — the refusal point.** Every non-durable stream is refused here, with the documented rejection contract: HTTP `503` on REST, `RESOURCE_EXHAUSTED` on gRPC, and the client is expected to back off and reconnect. The remedies are to raise `OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB` (with real broker RAM behind the higher number) or to lower `OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT`. **Adding pods does not relieve this tier** — the budget is fleet-wide and is apportioned across pods by `OMNI_EVENTS_SCALER_MAX_REPLICAS`, so a larger fleet divides the same broker memory into smaller per-pod caps. Note that all three of these settings — the budget, the prefetch, and the absolute override `OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD` — are exposed as chart values (`rabbitmq.broker_stream_memory_budget_mib`, `rabbitmq.channel_prefetch_count`, and `rabbitmq.max_non_durable_streams_per_pod` respectively), so a chart-based deployment sets them through the chart.

At the shipped defaults the picture is: soft threshold 800 streams (pool size 100 × client limit 8), and a computed cap of 3,342 streams per pod (a 1024 MiB budget at prefetch 15 — a 32,120 B worst case per stream, so 33,429 streams fleet-wide — apportioned across 10 replicas). The shed threshold, unset, lands at the midpoint of the 800–3,342 band, at 2,071.

**Do not send `X-Via-Proxy` from a direct client.** The stamp is injected by the chart on exactly one route — the proxied gRPC `ConsumeNonDurableEvents` route — and the service trusts it as it arrives. A pinned client that applies the header itself classifies its own streams as re-routable, and they will be shed at the shed threshold rather than being admitted up to the cap. This is an operator obligation, not a security control: the stamp can only be used to shed your own traffic, never to gain capacity.

**Deployments with nowhere to re-route.** A single-replica fleet, or a uniformly hot REST-heavy fleet where every pod is equally loaded, has no better pod for a shed stream to land on; shedding there just converts an admission into a client-visible error. For those deployments, set `OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD` equal to the cap. The tunable permits it, and it disables the shed tier so every class refuses at the cap and nowhere earlier.

**Watch for an inverted band.** Raising the prefetch far above its default at the shipped `client_limit=8` inverts the band: the packing product stays 800 streams while the cap collapses below it — at prefetch 500 the cap drops to 118 — in which case the shed tier collapses onto the cap and the pod never reaches its packing target. Lower the prefetch or raise the budget until the cap clears the packing product.

**The apportioning divisor tracks the fleet ceiling — check the arithmetic in both directions.** `OMNI_EVENTS_SCALER_MAX_REPLICAS` is templated into the service pods from the chart's fleet ceiling, so the per-pod cap moves inversely whenever that ceiling moves: a higher ceiling yields a smaller per-pod cap, a lower ceiling a larger one. Re-run the cap arithmetic below against your broker budget on any deployment that sets the ceiling away from its default, in either direction.

- **With autoscaling enabled**, the divisor is sourced from `channelPoolAutoscaling.maxReplicas`. A higher ceiling apportions the same fleet-wide budget into a proportionally smaller per-pod cap, so a pod sheds at a lower load. A deployment running a raised ceiling therefore begins shedding at lower per-pod stream counts than the same deployment at a lower ceiling — a raise is not free headroom, and the per-pod cap and shed threshold must be re-derived for whatever ceiling is in force. At `maxReplicas: 25` and otherwise shipped defaults the per-pod cap is 1,337 streams (33,429 fleet-wide ÷ 25) rather than the 3,342 a ceiling of 10 yields — still above the 800 soft threshold, so the band is not inverted, but the cap and the shed threshold both shrink proportionally. The band inverts only at a divisor high enough to pull the cap below 800 — roughly `maxReplicas` 42 or more at these defaults.
- **With autoscaling disabled**, the divisor is `replicaCount`. A chart-default single-replica deployment therefore holds the entire fleet budget in one pod, giving a computed per-pod cap roughly ten times the one a ceiling of 10 yields. That is the fleet-budget invariant working as intended — one pod may safely hold the whole budget — but verify that `OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB` actually matches your broker's memory, because on a single-replica deployment the whole budget is enforced against that one pod.
- **Changing the ceiling restarts the whole fleet.** The service Deployment carries a `checksum/config` annotation over the settings configmap, and the divisor lives in that configmap, so tuning `channelPoolAutoscaling.maxReplicas` rolls every service pod, not just the autoscaler pod. Consumer streams are sticky, so a roll means a mass reconnect across the fleet. Plan the change accordingly.

**Steering and transparent re-routing.** These are chart values on the streaming routes, not environment variables. They apply only to the long-lived streaming routes — the SSE endpoints and the four streaming gRPC RPCs (`ConsumeDurableEvents`, `ConsumeDurableEventsWithAcks`, `ConsumeNonDurableEvents`, and `ManageDeadLetteredEvents`) — and every key is overridable per protocol under `httpProxy.grpc.streaming.*` and `httpProxy.rest.streaming.*`. The unary calls, which ride the catch-all route, carry a separate and deliberately narrower retry policy described in the last two bullets.

- `httpProxy.streaming.loadBalancerPolicy.strategy` (default `RoundRobin`) selects how new streams are placed across ready pods. It is steering only and never changes admission. `RoundRobin` is do-no-harm and skew-neutral. `WeightedLeastRequest` is recommended **only** for deployments whose consume traffic is predominantly proxied or external, because Envoy counts only the streams it routed itself: in a fleet with many cluster-internal clients connecting directly, the pod holding the most invisible sticky streams looks emptiest and becomes a magnet for new external streams, which is strictly worse than `RoundRobin`. Use the per-pod stream counts as the deciding signal — visible sticky skew means keep `RoundRobin`.
- `httpProxy.streaming.retryPolicy.enabled` (default `true`) and `httpProxy.streaming.retryPolicy.count` (default `2`, i.e. up to three placement attempts) control re-routing a pod's capacity refusal. The retry *conditions* are deliberately not tunable — they mirror the service's error contract exactly — and `perTryTimeout` is deliberately never emitted, since Envoy applies it to the initial attempt too and any value shorter than a stream's lifetime would cut healthy long-lived streams. To turn retries off, set `enabled: false`; that omits the retry block entirely. Do not try to disable retries through `count`.
- The rendered `HTTPProxy` uses Contour's own retry fields (Contour v1.14 or later): `count`, `retryOn`, and `retriableStatusCodes`. On the gRPC proxy `retryOn` is `resource-exhausted`; on the REST proxy it is `retriable-status-codes` together with `retriableStatusCodes: [503]`.
- **What "transparent" does and does not mean here.** A retry is a fresh load-balancer selection, which *can* land on another pod — there is no retry-host predicate, so the same pod that just shed the stream may be selected again, and under `WeightedLeastRequest` it may even be preferred. A capacity refusal is re-routed without a client-visible error only when it arrives as a trailers-only `RESOURCE_EXHAUSTED` before any response has committed and the client's pre-admission request bytes fit within Envoy's roughly 1 MiB retry buffer. The service side of that precondition — refusing before anything commits — is covered by the test suite; the end-to-end re-dispatch is configured proxy behavior and is not verified against a running Envoy here. At fleet-wide saturation the bounded retries exhaust and the client sees the documented `503` / `RESOURCE_EXHAUSTED` and backs off, which is the intended outcome: the remaining band is reserved for pinned traffic.
- `httpProxy.unary.retryPolicy.enabled` (default `true`) and `httpProxy.unary.retryPolicy.count` (default `3`, i.e. up to four delivery attempts in total) govern the catch-all route, which carries every unary call — durable-queue create, update, and delete, on both the gRPC and the REST surface. The conditions are fixed at `connect-failure` and `refused-stream` and are not tunable, because they are exactly the two failures in which the request provably never reached an application: `connect-failure` is a TCP connect that failed or timed out, and `refused-stream` is an upstream `REFUSED_STREAM` reset, which is what a gRPC server's graceful GOAWAY produces for streams above its last-stream-id. Nothing broader is retried — no `5xx`, no `retriable-status-codes`, no `reset`, no `gateway-error` — because each of those can replay a request the service has already processed. As on the streaming routes, keys are overridable per protocol (`httpProxy.grpc.unary.*` and `httpProxy.rest.unary.*`), and `enabled: false` omits the retry block entirely and is the only way to turn the policy off; do not try to disable it through `count`. The attempts also run inside the catch-all route's own time budget: `httpProxy.timeoutPolicy.response` (default `10s`) bounds the whole request *including* its retries rather than each attempt separately, so it caps how much of the four-attempt sequence can actually be spent. Tightening that timeout shortens the sequence, and raising `count` without checking it against the timeout buys nothing.
- **What the unary retry does not cover.** The "transparent" caveats above apply here too: a retry is a fresh load-balancer selection with no retry-host predicate, so the same pod may be selected again, and the end-to-end re-dispatch is configured proxy behavior that is not verified against a running Envoy here. One residual gap is structural: the REST leg speaks HTTP/1.1, which has no `refused-stream` analogue, so a request the proxy dispatches onto a pooled keep-alive connection that the destination pod has already closed is not retried and surfaces to the client (Envoy records it with the `UC` response flag). A unary client may still retry a lone `503` itself, with one condition on create: update and delete are safe to repeat as-is — a repeated delete answers `NOT_FOUND` (REST `404`) when the first attempt did land, which is the success case, the queue is gone, and not a failure — and a create is safe to repeat only when the client supplied its own `queue_id`, which makes creation ensure-exists so the retry attaches to the queue the first attempt may already have made. Retrying a create that omitted `queue_id` leaves an orphaned queue behind, because the service generates a fresh id on each attempt — see [Choosing a durable queue id](#choosing-a-durable-queue-id).

### Graceful drain on scale-down and rollout

Consumer streams are sticky, so any event that deletes a pod — a scale-down, a
rolling restart, or a settings change that rolls the fleet (the
`checksum/config` annotation over the settings configmap, see [Capacity and
admission tiers](#capacity-and-admission-tiers)) — would otherwise force-drop
every live non-durable stream on that pod at once. Graceful drain turns that
hard drop into an orderly hand-off, and works with the reconnect-grace window
(`OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS`) as one story: drain finishes the
streams it can and lets the rest re-establish before their queues expire.

**What a terminating pod does.** On SIGTERM the pod first keeps serving
normally for `OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS` (Helm
`shutdown.settleDelaySeconds`; application default 0.0, chart default 15 s):
readiness still reports `SERVING`, new streams are still admitted, and nothing
is refused. That settle delay is the budget for the endpoint change a
terminating pod triggers to reach the clients that dial the readiness-gated
main Service directly, before the pod stops serving them. Setting it to `0`
skips the phase.

When the settle delay elapses the pod enters a draining state:
readiness flips to `NOT_SERVING` and the admission guard refuses *new*
non-durable streams (HTTP `503` on REST, `RESOURCE_EXHAUSTED` on gRPC), while
its already-admitted in-flight streams keep serving. It then waits up to
`OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS` (Helm `shutdown.drainWindowSeconds`,
default 30 s) for those in-flight streams to finish on their own — re-reading
the in-flight count every `OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS`
(Helm `shutdown.drainPollIntervalSeconds`, default 0.5 s) so it can close early
once the pool is empty — before the connector closes and any streams still held
are actively reclaimed. Meanwhile the streaming routes' ingress retry (see the
`httpProxy.streaming.retryPolicy` bullets under [Capacity and admission
tiers](#capacity-and-admission-tiers)) re-routes each refused new stream to
another ready pod, so callers hitting a draining pod land elsewhere rather than
seeing an error. As with the memory-safe cap, the retry-with-backoff response
contract is the REST/OpenAPI surface's alone; a gRPC `RESOURCE_EXHAUSTED` is
canonically retryable and you may configure gRPC clients to back off, but the
status itself carries no retry instruction.

**The settle delay and the grace-period invariant.** Two chart values bracket
the drain in time. `shutdown.settleDelaySeconds` (default 15) is the settle
phase above. The application applies it itself, on SIGTERM, so it needs no
Kubernetes feature gate and the chart installs on any Kubernetes version the
rest of the chart supports. `shutdown.terminationGracePeriodSeconds` (default
60) is the total budget Kubernetes allows from pod deletion to SIGKILL. That
budget is spent in order — the settle, then the up-to-`drainWindowSeconds`
drain, then a fixed 10 s post-drain server teardown — so the grace period
**must** cover all three:

```
terminationGracePeriodSeconds >= settleDelaySeconds + drainWindowSeconds + 10
```

The 10 s teardown term is fixed by the application rather than being a knob:
it is the budget for the server teardown that runs once the drain ends, and no
setting moves it. It applies even with the REST surface disabled, and even at
`settleDelaySeconds: 0`.

At the shipped defaults the invariant holds with margin: `60 >= 15 + 30 + 10 =
55`, leaving 5 s to absorb the client close that runs last (normally
sub-second). The chart enforces the invariant at render time and `fail`s the
template if it is violated, so an operator who lowers the grace period, or
raises `settleDelaySeconds` or `drainWindowSeconds`, must keep it true or the
render is rejected — the guard exists because a grace period shorter than the
sum lets the kubelet SIGKILL the pod mid-shutdown and lose the graceful
hand-off. The same render-time check rejects a negative or non-numeric
`settleDelaySeconds` or `drainWindowSeconds`, and a `drainPollIntervalSeconds`
that is not greater than zero.

**What the grace period does not cover.** The post-drain teardown also waits
for open REST/SSE connections to close, and that wait is unbounded, so the
invariant is a floor rather than a guarantee. An idle REST/SSE stream is not cut
when teardown begins: it ends at its next liveness-poll wake, up to roughly
`OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS` × 1.15 later (about 17 s at that
setting's default of 15). That setting is environment-variable only, so the
chart cannot compute the term and the 10 s teardown constant does not budget for
it. A deployment holding idle REST/SSE streams at drain end may therefore reach
the grace deadline during teardown. The drain itself has already finished by
then, so what a SIGKILL there costs is a TCP reset instead of a clean stream
end, plus a requeue with an incremented delivery count for any unacked durable
deliveries — which counts against `policyHook.policy.definition.deliveryLimit`.
Raise the grace period above the invariant's floor if clean SSE termination
matters to you. Closing the RabbitMQ connector can likewise block on an
unresponsive broker, which the default 5 s margin does not bound; a deployment
that has seen that should raise the grace period rather than rely on the margin.

**The proxied path keeps a short residual window, and the settle delay is not
its remedy.** Every `HTTPProxy` route targets the envoy-facing Service, which
publishes pod addresses regardless of readiness. Kubernetes marks every address
on that Service `ready`, and Contour selects endpoints on `ready` alone, so a
terminating pod stays in Envoy's endpoint set until the pod object is deleted —
which happens only after the container has exited. There is no ingress-side
endpoint change for the settle delay to cover, and raising
`shutdown.settleDelaySeconds` pushes back the app stop and the pod deletion by
the same amount, leaving that window exactly as wide as it was. No value closes
it. What covers it instead is proxy-side retry: the catch-all route's
`httpProxy.unary.retryPolicy` for unary calls and the streaming routes'
`httpProxy.streaming.retryPolicy` for streams, both described under [Capacity
and admission tiers](#capacity-and-admission-tiers). That remedy is partial: it
covers a refused connection on either leg and a refused stream on gRPC, but a
REST request dispatched onto a pooled connection the terminating pod had already
closed still reaches the client.

### Production deployment — broker sizing and high availability

This subsection covers sizing the RabbitMQ broker behind a production fleet and running it highly available. It complements [Capacity and admission tiers](#capacity-and-admission-tiers) above: that section governs how many streams a *pod* admits, while this one governs how much *broker* memory those streams — and the queues they leave behind — actually cost. The per-setting defaults are in the [Configuration](#configuration) table; this subsection is about how to combine them for a target load rather than a restatement of them.

**Broker requirements for the durable-queue bounds.** The bounds that protect the broker from unbounded durable queues — redelivery limit, message TTL, maximum length, overflow behavior, and the idle-queue TTL described under [Durable-queue lifecycle](#durable-queue-lifecycle) — are applied as a RabbitMQ policy by a Helm hook Job that runs on every install and upgrade. That puts three requirements on the broker:

- **The management HTTP API must be reachable from the cluster**, and its base URL must be configured (`policyHook.managementApi.baseUrl`). Do not assume port 15672: a self-hosted broker typically serves it on `http://<service>.<namespace>.svc.cluster.local:15672`, while a managed broker commonly serves it over HTTPS on 443. Certificates are verified; the call carries a broker credential.
- **A management credential, used at deploy time only.** Supply it as an existing Secret (`policyHook.managementApi.existingSecret`). The credential needs the `policymaker` tag plus access to the vhost — `administrator` is not required. The running service never receives it and keeps its own least-privilege AMQP credential.
- **RabbitMQ 4.1.5 or newer is recommended.** The queue-type scoping the policy uses needs 3.12 or newer. On 4.0.5 through 4.1.4 a negative redelivery limit triggers a broker bug (`rabbitmq-server#14202`) whose repair loop resets every matched queue's idle clock, so queues never reap; the chart refuses a negative `deliveryLimit` at render time for that reason, but 4.1.5+ removes the hazard entirely.

If the policy cannot be applied, **the Helm release fails** — deliberately, so a cluster never comes up silently unbounded. The Job retries transient broker failures internally and is bounded by `policyHook.backoffLimit` and `policyHook.activeDeadlineSeconds`.

**These three values are required — set them and create the Secret BEFORE any `helm install` or `helm upgrade`, or the release fails to render.** `policyHook.enabled` defaults to `true` and the three values above (`policyHook.managementApi.baseUrl`, `policyHook.managementApi.vhost`, `policyHook.managementApi.existingSecret.name`) have no defaults, so a release is **rejected at render time** until all three are supplied. Rendering successfully is not sufficient either: the hook Job runs as part of the upgrade and fails the release if the named Secret is missing from the namespace, the credential lacks the `policymaker` tag or vhost access, or the management API is unreachable — so create the Secret first and confirm the broker is reachable from the cluster. If you manage broker policies yourself, `policyHook.enabled: false` is the alternative, but read the escape-hatch warning below before choosing it — what it costs depends on whether the policy has already been applied to your broker.

**Managed brokers.** Managed RabbitMQ is supported on **dedicated-tier** plans (for example CloudAMQP's Sassy Squirrel and larger), where the management API is available and you manage policies freely. Shared/free tiers are not viable targets for this service regardless of the policy question: their channel caps sit below this service's default channel-pool sizing, their instance-wide message caps sit below the dead-letter queue's own default maximum length, and the provider applies its own queue-TTL policy to shared vhosts — which would displace this one, since at most one regular policy applies to a queue.

**Turning the hook off is not a tuning knob — and what it does depends on whether the policy was ever applied.** `policyHook.enabled: false` exists for operators who manage broker policies themselves, and it is the right setting for a restricted broker. The two cases behave very differently:

- **The policy was never applied** — a fresh cluster, or a broker where you removed the policy yourself. This is the cliff: durable queues carry **no maximum length, no message TTL, and no idle-queue TTL**, and the only thing bounding anything is RabbitMQ 4.x's built-in default redelivery limit of 20. An abandoned durable queue grows without limit and never reaps. Apply an equivalent policy by hand before running this way.
- **The hook is disabled after it had already been applied.** Nothing in the chart deletes the policy on disable. The last-applied `event-queue-limits` policy **stays live on the broker and keeps enforcing** every bound it carries, including reaping idle durable queues — frozen at whatever values it held when the hook was last enabled. Disabling the hook stops the chart from *updating* the policy; it does not remove it, and a client whose queue is reaped after you disabled the hook is seeing the still-live policy, not a bug. To actually remove it, delete it yourself: `rabbitmqctl clear_policy -p <vhost> event-queue-limits`, or the Policies page in the management UI.

In both cases, keep every bound for these queues in a **single** policy. A second regular policy matching the same queues **displaces** the first rather than merging with it, so a policy you add to supplement a still-live `event-queue-limits` will silently replace it — including dropping any bound you did not restate.

**Setting the idle-queue TTL relative to the message TTL.** Keep `expiresMs` **at or above** `messageTtlMs` (both default to 24 hours). A shorter idle window lets a queue be reaped while it still holds messages that had not yet expired — messages an absent client would otherwise have received on its next connect. This is a recommendation, not an enforced constraint; the chart renders either way. Note also that the idle-queue TTL *bounds* dead-letter churn rather than eliminating it: a freshly abandoned queue still fills to its maximum length and dead-letters overflow for up to one idle window before it is reaped.

**RabbitMQ high availability.** Run the broker as a 3-node quorum cluster. A stream is bound to the broker connection it opened on, so when a node fails every stream on that node ends. The two queue types recover differently: a durable queue survives the failure and replays its buffered messages once the client reconnects, while a non-durable queue does not — its events are lost unless the client reconnects within the reconnect-grace window (`OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS`, default 20 s), governed by the reconnect token. Size the surviving nodes to carry the reconnect wave a node failure produces. Note that RabbitMQ's default 0.6 memory high-watermark blocks *publishers*, not consumers: a broker at the watermark keeps delivering to existing consumer streams, so a memory alarm degrades ingestion (the Event Aggregation Service), not consumption.

**Broker memory sizing.** Size each node's RAM from:

```
RAM_node >= (B + (S / n) * (P * m + C)) / W
```

- `B` — broker baseline memory (idle is ~150 MB; a loaded cluster is dominated by durable quorum-queue backlogs, so measure it under real load).
- `S` — target concurrent non-durable streams across the fleet.
- `n` — number of broker nodes.
- `P` — non-durable prefetch (`OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT`, default 15).
- `m` — per-held-message cost, a **measured** ~1.5–1.8 kB (roughly 1 kB of broker metadata over a ~600 B event payload). This is about twice RabbitMQ's documented ">= 720 B" classic-metadata floor, so sizing from that floor under-provisions by ~2×.
- `C` — per-stream channel share, a **measured** ~5 kB (at ~8 streams per channel).
- `W` — the memory high-watermark fraction (default 0.6).

The `P * m` term is worst-case — every stream's whole prefetch window sitting delivered-but-unacked — and it is the dominant lever: a stream costs ~32 kB at prefetch 15 versus ~883 kB at prefetch 500. Because a ~600 B event fanned to N per-consumer queues is stored as **N copies** (RabbitMQ's shared message store only deduplicates bodies at or above its 4096 B embed threshold, and quorum queues never share), the `P * m` term is genuinely per-stream, not amortized across the fan-out.

**Reconnect-grace orphan backlog — a distinct line item.** On top of the live-stream term above, budget for the memory held by queues left behind by dropped non-durable streams during their reconnect grace. An orphaned queue holds no stream, so it is invisible to the per-pod stream cap, yet it stays bound to the topic exchange and keeps buffering every matching event for its whole grace window with nothing draining it:

```
orphaned queues * matching event rate * grace seconds * per-message cost
```

Worked example: a 12,000-stream fleet undergoing mass churn (a rolling restart, a reconnect wave), at a matching event rate of ~5 events/s per queue and the default 20 s grace, at ~1.8 kB/message, transiently holds 12,000 × 5 × 20 × 1.8 kB ≈ 2 GiB fleet-wide — several times the live prefetch term at the same fleet size. This memory is gentler than the prefetch term in two ways: it is **disk-pageable** (nothing holds those bodies in an unacked delivery window, so the broker can page them out under pressure) and **time-bounded** (each orphan self-deletes at its `x-expires` deadline, `OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS`). Non-durable queues deliberately carry no `x-max-length` — bounding an orphan's depth would also drop a live-but-slow consumer's backlog and break the reconnect token's "no missed events" guarantee — so this term must be provisioned for, not capped away. In practice this means `OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB` must sit **below** your real broker memory, leaving headroom for this backlog: the budget bounds only the live-stream prefetch term the per-pod cap divides.

**Fleet sizing.** Reaching a target fleet stream count is a values-only change, no code change. At stock density (soft threshold 800/pod, 720 usable in the scaling signal after the readiness reserve), ten pods carry 8,000 admitted / 7,200 usable — short of a 12,000-stream target. Two paths clear 12,000:

- **More pods:** raise `channelPoolAutoscaling.maxReplicas` to 17 (17 × 800 = 13,600 admitted, 17 × 720 = 12,240 usable — clears both bases). `maxReplicas` 15 hits 12,000 on the admitted basis alone (15 × 800), but only 10,800 usable, so 17 is the recommended value.
- **Denser packing:** raise `OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT` to 16 at the stock ten-pod fleet (soft threshold 1,600/pod → 16,000 admitted, 15,200 usable).

One gotcha: `OMNI_EVENTS_SCALER_MAX_REPLICAS` — the divisor that apportions the fleet-wide budget across pods — is sourced from `channelPoolAutoscaling.maxReplicas` when autoscaling is enabled and from `replicaCount` when it is disabled. A single-replica deployment with autoscaling off therefore divides the whole budget into one pod; confirm the budget matches your broker before scaling such a deployment by hand.

**Sizing tiers.** Three reference points, substituting into the formula above on a 3-node cluster (`n = 3`, `W = 0.6`, prefetch 15 so `P*m + C ≈ 32 kB/stream`). The live-stream term is small at prefetch 15 — measured broker baseline `B` and the orphan-grace transient dominate real RAM, so add both on top:

| Target fleet streams | Broker topology | Per-node RAM (formula) | `BROKER_STREAM_MEMORY_BUDGET_MIB` | Fleet scaling |
|---|---|---|---|---|
| ~1,000 | 3-node quorum | `(B + (1,000/3 × 32 kB)) / 0.6` ≈ `(B + ~11 MB) / 0.6` | ~256 (well below real RAM) | `maxReplicas` 10, `client_limit` 8 |
| ~12,000 | 3-node quorum | `(B + (12,000/3 × 32 kB)) / 0.6` ≈ `(B + ~129 MB) / 0.6` | ~1024 | `maxReplicas` 17 (or `client_limit` 16 at 10 pods) |
| ~30,000 | 3-node quorum, larger nodes | `(B + (30,000/3 × 32 kB)) / 0.6` ≈ `(B + ~321 MB) / 0.6` | raise above 1024 | `maxReplicas` 17+ with denser packing |

Per-tier band sanity: at each tier the per-pod cap must stay above that tier's soft threshold, or the band inverts. The ~30k tier binds hardest — the fleet cap at the shipped 1024 MiB budget and prefetch 15 is 33,429 streams, so a 30,000-stream target sits just under that ceiling and needs its budget (and the real broker RAM behind it) raised to leave margin.

**When the cap binds, never "add pods".** If pods refuse at the memory-safe cap under real load, raise broker RAM and `OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB` together, or lower `OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT`. Adding pods does not help: the budget is fleet-wide and is divided by the replica ceiling, so a larger fleet apportions the same broker memory into smaller per-pod caps.

### Bounding a hung durable consumer

A client that stops acknowledging messages — deadlocked, wedged on a slow
downstream, or simply stuck — holds its unacknowledged deliveries indefinitely, and
the queue behind it grows. A client that is *merely* unable to process a
particular message has a better option than withholding its acknowledgement: it
can nack the delivery tag and have the message requeued for another attempt, as
described under [Settling a durable delivery](#settling-a-durable-delivery). The timeout
below exists for the clients that do not — or cannot — settle at all. RabbitMQ
bounds those with a **consumer acknowledgement timeout**: when any single delivery goes unacknowledged for longer than the
timeout, the broker cancels the consumer, closes its channel, and requeues the
unacknowledged messages. The service detects the resulting consumer loss and ends
the client's stream with an error (see
[A consume stream ends when the broker drops your consumer](#a-consume-stream-ends-when-the-broker-drops-your-consumer)),
so the client reconnects rather than sitting on a queue nothing is consuming.

**The chart ships no timeout by default.** `policyHook.policy.definition.consumerTimeoutMs`
defaults to `null`, which omits the `consumer-timeout` key from the
`event-queue-limits` policy and leaves **the broker's own default in force — 30
minutes**, unless your server config says otherwise. Set the value only after
sizing it against your slowest client, per the arithmetic below.

This policy key is the only lever that can be retuned on a running cluster.
RabbitMQ accepts the ack timeout as a queue argument (immutable once the queue
exists), as this policy key, or in the server config — never as a
`basic.consume` argument, which the broker silently ignores.

**Sizing — read this before setting a value.** The timeout is applied **per
delivery**, and durable consumers prefetch `OMNI_EVENTS_DURABLE_CONSUMER_PREFETCH_COUNT`
(default 20) messages at a time, so the last message of a prefetch batch has
been sitting undelivered-to-the-client for the whole time the client spent
working through the preceding 19. The safe form is:

```
consumerTimeoutMs > OMNI_EVENTS_DURABLE_CONSUMER_PREFETCH_COUNT
                    × worst-case per-message processing time
```

Sized too tightly, every healthy-but-slow consumer is killed on a loop — a
continuous channel-kill/reconnect cycle, which is worse than the hung consumer
the timeout exists to catch. At the default prefetch of 20, a 60000 (60 s)
timeout would demand sub-3-second per-message processing from every client.

Upstream bounds constrain the range further: values below `60000` (60 s) are not
supported, values below `300000` (5 minutes) are not recommended, and enforcement
runs on a one-minute tick — so the effective timeout is your value rounded up to
the next minute. **Start at or above `300000`.** The key applies to quorum
queues, which is what the durable event queues are, so no additional scoping is
needed.

**The chart refuses anything below `60000` at render time.** A value under the
upstream hard floor fails `helm template` / `helm install` / `helm upgrade` with
a sizing-aware message, alongside the type check that rejects a non-numeric
value. The chart enforces that hard floor only because the broker does not:
`consumer-timeout: 0` is accepted by RabbitMQ and cancels every consumer on
every durable queue on its first delivery, while the policy hook still reports
success. The `300000` recommendation and the prefetch arithmetic above stay
**guidance**, not enforcement — the safe value depends on your clients'
per-message processing time, which the chart cannot see. `null` (the default)
remains valid and means "ship no `consumer-timeout` key".

### Retuning the dead-letter queue's bounds

The dead-letter queue is bounded separately from the durable queues covered under [Production deployment — broker sizing and high availability](#production-deployment--broker-sizing-and-high-availability), and by a different mechanism: `OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS` and `OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH` are **queue arguments** the service applies when it first declares the queue, not policy values, and RabbitMQ queue arguments are immutable once a queue exists. Changing either on a running deployment therefore does not retune the live queue. It is also not fatal: on each (re)connect the service logs a single ERROR beginning `Dead-letter queue argument mismatch`, naming the desired-but-unapplied values and these remedies, then completes startup against the existing queue, so dead-letter capture keeps working and pods do not crash-loop on a configuration change. Grep that prefix to detect the drift. The line's trailing `Broker error: …` text names the offending argument, and it need not be one of these two settings — if it names something else (`x-queue-type`, say, on a live queue that is a classic queue or was provisioned by another tool), no setting change or policy can clear it and only recreating the queue will. Two ways to act on it:

- **To lower a bound on the live queue**, apply a broker policy scoped to the dead-letter queue (default `events.dlq`, or whatever `OMNI_EVENTS_DEAD_LETTER_QUEUE_NAME` is set to). Where an argument and a policy both set a numeric bound, RabbitMQ takes the **lower** of the two, so a policy can tighten a bound but can never raise one above the value the queue was created with. **Omit `overflow` from that policy**: unlike the numeric keys it is policy-wins on quorum queues, so including it overrides the queue's own `drop-head` argument and silently changes how the dead-letter queue behaves when full. This policy does not collide with `event-queue-limits`, whose `^event-queue:` pattern does not match the dead-letter queue — but keep every dead-letter bound in this one policy, for the same displacement reason as under [Production deployment — broker sizing and high availability](#production-deployment--broker-sizing-and-high-availability).

  ```bash
  # Self-hosted broker, with node access:
  rabbitmqctl set_policy -p <vhost> dlq-limits '^events\.dlq$' '{"max-length":500000}' --apply-to quorum_queues

  # Any broker, including a managed one: the management HTTP API.
  # URL-encode the vhost in the path ("/" is %2F).
  curl -u "$RABBITMQ_USER:$RABBITMQ_PASSWORD" -X PUT \
    -H 'content-type: application/json' \
    -d '{"pattern":"^events\\.dlq$","definition":{"max-length":500000},"apply-to":"quorum_queues","priority":0}' \
    "https://<broker-host>/api/policies/<vhost-urlencoded>/dlq-limits"
  ```

  Or add the same policy on the Policies page in the management UI. On a managed broker you have no node access and so no `rabbitmqctl`; the management API and the UI are the only routes, and both are available on the dedicated-tier plans this service supports (see **Managed brokers** under [Production deployment — broker sizing and high availability](#production-deployment--broker-sizing-and-high-availability)).
- **To raise a bound** — or to clear a mismatch on an argument these settings do not control — the queue has to be recreated: drain it (browse and DISCARD or REPLAY the dead-letters you want to keep, through this service's dead-letter management API) and delete it in a maintenance window. The next time a pod connects, the service declares it fresh from the current settings. Deleting the queue has the same two reaches as the policy above — `rabbitmqctl delete_queue` needs node access, so on a managed broker use the management API or the Queues page in the management UI:

  ```bash
  # Self-hosted broker, with node access:
  rabbitmqctl delete_queue -p <vhost> events.dlq

  # Any broker, including a managed one: the management HTTP API.
  # URL-encode the vhost in the path ("/" is %2F, so the default vhost "/" is
  # written as %2F here — https://<broker-host>/api/queues/%2F/events.dlq).
  curl -u "$RABBITMQ_USER:$RABBITMQ_PASSWORD" -X DELETE \
    "https://<broker-host>/api/queues/<vhost-urlencoded>/events.dlq"
  ```

The chart does not ship a dead-letter-queue policy — the hook Job's policy governs the durable event queues only, and the dead-letter queue name is a service setting the chart does not template, so it has no name to anchor a pattern to. Apply the policy above by hand if you need it.

### Durable-queue admission

Durable operations — durable consume, durable-queue create/delete/update, and dead-letter browse/replay/discard, on both the gRPC and REST surfaces — are admitted against the **durable** channel pool's own budget, entirely separate from the non-durable pool. Heavy non-durable fan-out can therefore never 503 a durable operation. The four numbered tiers in [Capacity and admission tiers](#capacity-and-admission-tiers) apply to the non-durable path only: durable admission is single-tier and strict — no overflow band, no shed threshold, no memory cap, and no durable twin of `OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED`.

A durable operation is refused only when the durable pool itself is full, and only durable operations are refused there — never a non-durable stream. Because the durable pool default (`rabbitmq.durable_channel_pool_size`, 100) is deliberately generous relative to the handful — typically one to five — of long-lived durable queues a deployment runs, a full durable pool is a **configuration error, not a capacity limit**; the durable pool is never "small". The refusal contract is the same one the non-durable cap uses: HTTP `503` on REST, `RESOURCE_EXHAUSTED` on gRPC.

The retry-with-backoff advice is the REST/OpenAPI response contract's alone. A gRPC durable refusal carries `RESOURCE_EXHAUSTED` with the detail `Channel pool is exhausted` and no retry guidance in the status. `RESOURCE_EXHAUSTED` is canonically retryable, so you may configure gRPC clients to back off and retry as operator guidance — but the gRPC status itself does not carry that instruction.

### Permissions

Permission enforcement is controlled by two independent gates, split by which
backend the authorization check calls — not by event category:

- **`OMNI_EVENTS_PERMISSIONS_ENABLED`** (`permissions.enabled`, default `False`) —
  the permission-service gate. When true, requests authorized via the Permissions
  API (defined with `OMNI_EVENTS_PERMISSIONS_ENDPOINT`) are enforced before a
  client may begin consuming a given event_type. When false, those checks pass.
- **`OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED`** (`permissions.storage_permissions.enabled`,
  default `False`) — the storage-service gate. When true, requests authorized via
  the Storage API (the endpoints in `permissions.storage_permissions.endpoints`)
  are enforced. When false, those checks pass.

The two gates are fully independent: each may be enabled or disabled without the
other. Enforcement is never inferred from whether an endpoint is set — an
endpoint is only the target the check is made against once its own gate is on.

The split is by code path, not by event category. Storage events take different
paths depending on the queue type:

- **Durable-queue storage events** (`STORAGE_CREATED` / `STORAGE_DELETED`) are
  checked through the Permissions API, so they are governed by
  `permissions.enabled` (`OMNI_EVENTS_PERMISSIONS_ENABLED`).
- **Non-durable storage events** are checked through the Storage API, so they are
  governed by `permissions.storage_permissions.enabled`
  (`OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED`).

Each gate is validated against its own endpoint, with no OR-semantics across the
two: when `permissions.enabled` is true, `OMNI_EVENTS_PERMISSIONS_ENDPOINT` must
be configured; when `permissions.storage_permissions.enabled` is true, the storage
endpoints (`permissions.storage_permissions.endpoints`) must be configured. A
gate's endpoint(s) are inert when that gate is off. Enabling either gate also
requires a non-`disabled` `OMNI_EVENTS_PRINCIPAL_SOURCE` (there must be a principal
to authorize).

#### Permissions for Durable Queues

There must be appropriate policies in the permissions API for the
`event-consumer-service` and the following actions:

* create-durable-queues
* delete-durable-queues
* consume-durable-queues

These three actions cover four operations. Updating a queue's filters is
authorized by `create-durable-queues` — there is no separate update action — so
a principal that can create durable queues can also re-filter any existing one.

The actions are deployment-wide rather than per-queue: a principal granted
`consume-durable-queues` may consume from any durable queue whose `queue_id` it
knows, and `delete-durable-queues` likewise covers every durable queue. Grant
them to the service accounts that operate durable queues rather than to broad
user groups — see [Choosing a durable queue id](#choosing-a-durable-queue-id).

#### Permissions for Dead-Letter Management

There must be appropriate policies in the permissions API for the
`event-consumer-service` and the following action:

* inspect-and-replay-all-dlq

This action is required for an operator to browse dead-lettered messages and
replay, discard, or leave them via the management RPC (`ManageDeadLetteredEvents`).
A caller lacking the `event-consumer-service:inspect-and-replay-all-dlq`
permission is denied (`PERMISSION_DENIED`).

#### Storage Events With a Durable Queue

Durable-queue storage events are checked through the Permissions API, so they are
governed by the permission-service gate (`OMNI_EVENTS_PERMISSIONS_ENABLED`). There
must be appropriate policies in the permissions API for the
`event-consumer-service` and the following actions:

* consume-all-storage-create-events
* consume-all-storage-delete-events

#### Storage Events With a Non Durable Queue

Non-durable storage events are checked through the Storage API, so they are
governed by the independent storage-service gate
(`OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED`). See
[the section below](#fine-grained-permissions).

With the gate enabled, a non-durable stream receives a folder's storage events
when the principal presenting the stream's bearer token may list that folder
through the Storage API. The service checks by listing the folder with that
token. The stream receives the events of a folder the principal may list even
when that folder has no files yet, so the first file created in a new folder
is delivered. It receives none of the events of a folder the principal may
not list, and the service rechecks such a folder when the next event for it
arrives, and then at most once every 30 seconds while events for it keep
arriving.

When a listing does not authorize a folder, the service logs
`storage listing did not authorize a directory; its storage events are dropped until the recheck`.
The line is at `INFO` when the Storage API denied the listing (`403`), which
is an ordinary permission outcome. It is at `WARNING` when the Storage API
answered `400` for an address it does not serve, or `404` with anything other
than its empty-folder answer. A `WARNING` usually means that
`permissions.storage_permissions.endpoints`, or the routing in front of the
Storage API, does not match the storage deployment your resource ids point
at. The line does not name the folder.

The service counts the storage events it drops for such a folder and reports
the count once, in the `dropped_events` attribute. The next such line for the
folder carries it. When the folder leaves that state another way, for example
because a later listing authorizes it or the stream ends, the service instead
logs
`storage events were dropped for a directory the storage listing did not authorize`
with the count, unless no events were dropped. That line is at the same
levels, and at `WARNING` when the service cannot tell how the Storage API
answered. It does not name the folder either. The count is an OpenTelemetry
log attribute on both lines, exported when telemetry is enabled
(`OMNI_EVENTS_TELEMETRY_ENABLED`); the pod's stdout line shows only the
message.

#### Fine Grained Permissions

Fine-grained permissions may also be
needed, based on the messages for a given event_type. A YAML file is used to configure
fine-grained permissions. The path to the YAML file must be specified
in the `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH` environment variable.

##### Fine Grained Storage Permissions

For the built-in storage event types, the storage API will be used to check
for authorization.
For all other event types, the permissions api will be
used with a configurable per event-type "action" value.

The storage-service gate needs the list of storage service endpoints that may be
used for storage event type permissions checks. Configure these via the Helm
value `permissions.storage_permissions.endpoints`; the chart renders them into the
permissions YAML file as `storage_permissions_endpoints`, which the service loads
at startup. The endpoints are consulted only when the storage-service gate
(`OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED`) is enabled.
In the future, routing rules defined by the storage APIS will be used to determine
which endpoint to use for a specific event, based on its resource_id.

| setting name                              | description                                                                                                                                                                                                                                                    | default value |
|-------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------|
| permissions.storage_permissions.endpoints | The list of Storage API endpoints that may be used for storage event type permissions checks. Set as a Helm value; rendered into the permissions YAML file as `storage_permissions_endpoints`. Routing rules defined by the storage APIS will be used to determine which endpoint to use for a specific event, based on its resource_id. | Empty list    |

Example Helm values:

```yaml
permissions:
  storage_permissions:
    enabled: true
    endpoints:
      - https://azure-blob-api.example.com
      - https://s3-api.example.com
```

These endpoints render into the permissions YAML file as:

```yaml
storage_permissions_endpoints:
  - https://azure-blob-api.example.com
  - https://s3-api.example.com
```

##### Non-storage Fine-Grained Permissions

For each non-storage event type that requires fine-grained permissions checks,
the following settings in the YAML file must be specified:

| setting name | description                                         | default value |
|--------------|-----------------------------------------------------|---------------|
| event_type   | The name of the event type                          | None          |
| action       | The action to use when querying the permissions api | None          |

When the permissions API call is made while consuming each event, the call will
also include the resource_id from the event message. If resource is not required,
an unscoped/global policy can be used in the permissions service,
where the defined policy includes action but no resource.

Example YAML file contents:

```
event_permissions:
  - event_type: permissions.created
    action: read-permissions
  - event_type: thumbnail.created
    action: read-thumbnails
```

And this could work with the following example policies (where resource is optional):

```
permit(
  principal == Principal::"principal-id-goes-here",
  action == Action::"event-aggregation-service:read-permissions",
  resource == EventResourceId::"some/value/for/resource/id"
);
```

and

```
permit(
  principal == Principal::"principal-id-goes-here",
  action == Action::"event-aggregation-service:read-thumbnails",
);
```

**NOTE:** the resource_id from each event message will be used as the id for the
resource in the permissions API call, with a resource type of "EventResourceId"

## Building and reimplementing

This service is published source-available in the
`NVIDIA-Omniverse/usd-content-workspace` repository, under
`notifications/reference/event-consumer-service`. That folder carries the
reference implementation together with the customer-facing API material:

- `BUILDING.md`, the from-source build guide (`uv sync` / `uv run start`).
- `openapi.yaml`, `conformance_tests/`, and `AGENTS.md`, a guide to building a
  protocol-compatible implementation from the contract.
- `docs/`, the customer documentation for both notification services.

The consumer Protocol Buffer definitions are published separately, under
`notifications/api` in the same repository.

## License

This project is licensed under the Apache License, Version 2.0. See the
[LICENSE](LICENSE) file for the full license text.

## Contributing

This project is currently not accepting contributions.