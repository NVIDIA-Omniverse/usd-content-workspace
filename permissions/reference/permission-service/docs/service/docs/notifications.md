(permission-service-notifications-service-notifications)=
# Service notifications

Permission Service can publish an event every time the set of effective policies for a `(principal, action, resource)` tuple changes, and every time service metadata (a service, its actions, or its resource types) is changed or removed. Events are delivered over gRPC to a **Notification Service** (Event Aggregation Service) so that any number of downstream consumers can react to permission and metadata updates without polling the Permission Service. This page describes what the integration is for, when events are emitted, the event schema on the wire, how consumers receive them, and how to enable and credential the publisher in the Helm chart.

Both sides of the integration are **disabled by default**. Until they are enabled, policy and metadata writes succeed without publishing anything, and a replica that did not handle a write keeps its cached decisions until they expire — see {ref}`Configuration <permission-service-notifications-configuration>` for the values that turn each side on.

For the policy management endpoints whose writes trigger these events, see {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>`. The outgoing gRPC calls reuse the shared `serviceIdentity` credentials configured in the Helm chart; see {ref}`Authenticating to the Notification Service <permission-service-notifications-authenticating-to-the-notification-service>` below.

(permission-service-notifications-why-publish-permission-change-events)=
## Why publish permission-change events

The decision returned by Permission Service is deterministic for a given set of policies and metadata, so callers frequently cache authorization results instead of asking the service on every request — see the evaluation cache inside the service itself in {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-4-decision-cache-lookup>`, and similar per-client caches that downstream services maintain for their own HTTP handlers. When a policy is created, updated, or deleted, those caches would otherwise keep returning the previous decision until their TTL expires.

The notification integration closes that gap by turning every policy write into an event on a shared message broker. Typical consumers are:

- **Permission UI and other admin tools** — refresh the policy or user view in real time as soon as an administrator changes a policy somewhere else, without waiting for the next refresh interval.
- **Services that cache Permission Service responses** — invalidate their own `(principal, action, resource)` entry as soon as the underlying policies change, so users see the new decision on their next request instead of after the cache TTL.
- **Audit and analytics pipelines** — record every policy change as a first-class event alongside the actions it affects, for downstream search, reporting, or alerting.

Consumers subscribe directly to the Notification Service; the Permission Service does not know or care how many consumers there are. The integration is optional — when it is disabled, every other part of the service continues to work, but caches downstream must rely on their own TTLs.

(permission-service-notifications-when-events-are-published)=
## When events are published

The service publishes two kinds of events: `omni.permissions.changed` for policy writes and `omni.permissions.meta.changed` for service-metadata writes. Both are emitted **after** the write is persisted and the local caches have been invalidated, so consumers that reach Permission Service in response to an event see the new state. Failed or rejected writes do **not** publish an event, and read endpoints never emit events.

The service publishes exactly one `omni.permissions.changed` event per successful **policy write**, for every one of the following endpoints:

| Endpoint                              | Trigger                                                        |
|---------------------------------------|----------------------------------------------------------------|
| `PUT /v1beta/policies/`               | One event for the created policy.                              |
| `PUT /v1beta/policies/batch/`         | One event per policy in the batch (skipped if the batch is empty). |
| `DELETE /v1beta/policies/{id}`        | One event for the deleted policy. The endpoint is idempotent — when the id did not exist no policy was removed and no event is emitted. |

The service publishes exactly one `omni.permissions.meta.changed` event per successful **metadata write**, for every one of the endpoints under `/v1beta/services/`:

| Endpoint                                                | `entity`        | `operation` |
|---------------------------------------------------------|-----------------|-------------|
| `PUT /v1beta/services/{service}/`                       | `service`       | `updated`   |
| `DELETE /v1beta/services/{service}/`                    | `service`       | `removed`   |
| `PUT /v1beta/services/{service}/actions/`               | `action`        | `updated`   |
| `PUT /v1beta/services/{service}/actions/{action}/`      | `action`        | `updated`   |
| `DELETE /v1beta/services/{service}/actions/{action}/`   | `action`        | `removed`   |
| `PUT /v1beta/services/{service}/resource-types/`        | `resource_type` | `updated`   |
| `PUT /v1beta/services/{service}/resource-types/{type}/` | `resource_type` | `updated`   |
| `DELETE /v1beta/services/{service}/resource-types/{type}/` | `resource_type` | `removed`   |

Deleting a service cascades to its actions and resource types in storage, but emits a single `service`/`removed` event for the whole service. The two bulk-replace endpoints (`PUT .../actions/` and `PUT .../resource-types/`) emit a single event with an empty `name`, since they replace the entire collection for the service. Metadata writes are only supported by the Postgres backend; in config-file mode these endpoints return `501 Not Implemented` and publish nothing.

```{note}
Notifications are **best-effort**. The event is dispatched asynchronously from a background task, so the API caller receives its success response as soon as the policy or metadata write has been persisted, regardless of whether the event was delivered. If the Notification Service is unreachable or rejects the publish, the failure is logged but the write is kept.
```

When the event publisher receives `Unauthenticated` from the Notification Service, the Permission Service refreshes its service credentials (see {ref}`Authenticating to the Notification Service <permission-service-notifications-authenticating-to-the-notification-service>`) and retries the publish once before giving up.

(permission-service-notifications-event-schema)=
## Event schema

Both events are serialized as the `Event` message from the Omniverse Notifications `publisher.v1beta` proto and sent over gRPC with the `PublishEvent` RPC. Every event shares the same envelope:

| Field           | Type                           | Value |
|-----------------|--------------------------------|-------|
| `event_type`    | string                         | `omni.permissions.changed` or `omni.permissions.meta.changed`. |
| `occurred_at`   | `google.protobuf.Timestamp`    | Wall-clock time at which the event was constructed (seconds since the Unix epoch, nanoseconds zero). |
| `message`       | `google.protobuf.Struct`       | JSON-shaped object whose fields depend on `event_type`, see below. |
| `resource`      | `EventResource`                | Optional resource identifier used for consumer-side filtering, see below. |

(permission-service-notifications-policy-change-events-omni-permissions-changed)=
### Policy-change events (`omni.permissions.changed`)

The `message` struct always carries three string fields, corresponding to the policy scope that was affected by the write:

| Field       | Description |
|-------------|-------------|
| `principal` | Bare principal id the policy was pinned to (for example, `alice`). Empty string when the policy's principal scope was unset (global). |
| `action`    | Action entity id in `service:name` form, for example `storage-service:read`. Empty string when the policy's action scope was unset. |
| `resource`  | Resource in Cedar form, for example `object::"/Projects/Scene.usd"`. Empty string when the policy's resource scope was unset. |

When the affected policy pinned a resource, `event.resource.resource_id` is populated with the **resource id** as stored by Permission Service (URL-encoded, without the Cedar type prefix) so that consumers can use the Notification Consumer API's `resource_filter` to subscribe only to events about resources they care about — see {ref}`Consuming the events <permission-service-notifications-consuming-the-events>`.

Example event emitted after `PUT /v1beta/policies/` stores a policy pinned to all three scopes:

```json
{
  "event_type": "omni.permissions.changed",
  "occurred_at": "2026-04-16T12:34:56Z",
  "message": {
    "principal": "alice",
    "action": "storage-service:read",
    "resource": "object::\"/Projects/Scene.usd\""
  },
  "resource": {
    "resource_id": "/Projects/Scene.usd"
  }
}
```

A policy without any pinned scope — for example `permit(principal, action, resource);` — produces an event with empty strings in all three `message` fields and no `resource` object:

```json
{
  "event_type": "omni.permissions.changed",
  "occurred_at": "2026-04-16T12:34:56Z",
  "message": {
    "principal": "",
    "action": "",
    "resource": ""
  }
}
```

An empty `message` field still signals that *something global* changed; consumers that cache Permission Service responses typically invalidate their whole cache when any dimension is empty, since a global permit or forbid can affect any tuple.

(permission-service-notifications-metadata-change-events-omni-permissions-meta-changed)=
### Metadata-change events (`omni.permissions.meta.changed`)

Metadata-change events describe a mutation to the service catalog — a service, one of its actions, or one of its resource types. The `message` struct carries four string fields:

| Field       | Description |
|-------------|-------------|
| `service`   | Name of the service whose metadata changed (for example, `storage-service`). Always present. |
| `entity`    | Which kind of metadata changed: `service`, `action`, or `resource_type`. |
| `operation` | `updated` for create/update writes (including bulk replaces), or `removed` for deletes. |
| `name`      | Name of the specific action or resource type affected. Empty string for service-level changes and for the bulk-replace endpoints, which affect the whole collection. |

`event.resource.resource_id` is always set to the affected `service` name so that consumers can use the Notification Consumer API's `resource_filter` to subscribe only to metadata changes for the services they care about — see {ref}`Consuming the events <permission-service-notifications-consuming-the-events>`.

Example event emitted after `PUT /v1beta/services/storage-service/resource-types/object/` updates a single resource type:

```json
{
  "event_type": "omni.permissions.meta.changed",
  "occurred_at": "2026-04-16T12:34:56Z",
  "message": {
    "service": "storage-service",
    "entity": "resource_type",
    "operation": "updated",
    "name": "object"
  },
  "resource": {
    "resource_id": "storage-service"
  }
}
```

Example event emitted after `DELETE /v1beta/services/storage-service/` removes a whole service (and cascades to its actions and resource types):

```json
{
  "event_type": "omni.permissions.meta.changed",
  "occurred_at": "2026-04-16T12:34:56Z",
  "message": {
    "service": "storage-service",
    "entity": "service",
    "operation": "removed",
    "name": ""
  },
  "resource": {
    "resource_id": "storage-service"
  }
}
```

(permission-service-notifications-consuming-the-events)=
## Consuming the events

Consumers register with the Notification Service's Consumer API. The Notification Service owns the subscription model, durable queues, and delivery semantics. Besides third-party consumers, the Permission Service is itself a first-party consumer of both event types for its own cache coherence — see {ref}`Consuming change events for cross-replica cache invalidation <permission-service-notifications-cross-replica-cache-invalidation>`. Two filters are useful in practice:

- **Filter by `event_type`** — subscribe to `omni.permissions.changed` to receive permission-change events only, or to `omni.permissions.meta.changed` to receive service-metadata changes only, ignoring the rest of the traffic on the broker.
- **Filter by `resource.resource_id`** — when a consumer only caches decisions for a specific subtree (for example, Storage Service caches for files under a given prefix), it can add a `resource_filter` such as:

  ```json
  {
    "filter_type": "FILTER_TYPE_STARTS_WITH_GREEDY",
    "resource_id": "/Projects/"
  }
  ```

  The consumer then receives events whose affected policy pinned a resource whose id starts with `/Projects/`, at any depth. The Consumer API also defines `FILTER_TYPE_STARTS_WITH_LAZY`, which does not match ids nested more than one level below the prefix, so use the greedy form to cover a whole subtree.

  Events whose policy did not pin a resource carry no `resource` field, which the Event Publishing API treats as `resource_id = ""`. A non-empty resource filter is not documented to match them, so a consumer that also needs these global invalidations adds a second filter with an empty `resource_id` to the same filter group (filters in a group are combined with OR). Per the Consumer API, `FILTER_TYPE_STARTS_WITH_LAZY` with an empty `resource_id` matches events without a resource id (and top-level ids only), while `FILTER_TYPE_STARTS_WITH_GREEDY` with an empty `resource_id` matches every event of the type.

Refer to the Notification Service's own documentation for the full Consumer API, including how to create durable queues, commit offsets, and configure backoff. For how to grant a consumer principal the right to consume from those queues inside Permission Service, see the `event-consumer-service` actions in {ref}`Actions and resources for Omniverse services <permission-service-policy-language-actions-and-resources-for-omniverse-services>`.

(permission-service-notifications-cross-replica-cache-invalidation)=
## Consuming change events (cross-replica cache invalidation)

Each Permission Service replica keeps its own in-memory evaluation cache (authorization decisions) and metadata cache (service catalog). When the deployment runs more than one replica, only the replica that handles a given write invalidates its own caches; the other replicas would keep serving stale decisions and metadata until their TTL expires. To close that gap, the service can act as a first-party consumer of the very events it publishes: it subscribes to both `omni.permissions.changed` and `omni.permissions.meta.changed` and invalidates its local caches whenever another replica performs a write.

The subscription is **non-durable**: cache invalidation is best-effort and the cache TTL already bounds staleness, so there is no value in queuing missed events while a replica is down. On startup, when `notifications.consumer.enabled` is `true`, the service opens a `ConsumeNonDurableEvents` stream to `event-consumer-service` with one `FilterGroup` per event type. Each group carries a single greedy resource filter (`FILTER_TYPE_STARTS_WITH_GREEDY` with an empty `resource_id`), which the Consumer API defines as matching every event of the type, whether or not it carries a resource id. The service dispatches each received event by `event_type`:

- **`omni.permissions.changed`** reconstructs the same `(principal, action, resource)` key the policy write path uses and invalidates just that entry in the evaluation cache. The `principal` (bare id), `action` (`service:name` entity id), and `resource` (Cedar `Type::"id"` form) fields are parsed back from the wire encoding described in {ref}`Event schema <permission-service-notifications-event-schema>`. When any scope dimension is empty (a global change) or a present field cannot be parsed, the replica falls back to flushing the whole evaluation cache.
- **`omni.permissions.meta.changed`** reads the `service` field and invalidates every entry for that service in both the metadata cache and the evaluation cache, mirroring the metadata write path. When `service` is missing, it flushes both caches.

The consumer reuses the same `serviceIdentity` credentials and token source as the publisher (no second token loop). Because `ConsumeNonDurableEvents` is long-lived while the bearer token is stamped once per stream, the consumer keeps the stream authenticated by **re-opening** it — proactively before the attached token expires, and reactively when the server returns `Unauthenticated` — and applies bounded exponential backoff to other errors or clean disconnects. Self-published events are not filtered out; a replica receiving its own event performs a harmless redundant invalidation. During any disconnect the cache TTL still bounds how long stale data can be served.

Enabling the consumer is independent of the publisher (`notifications.publisher.enabled`): a replica set that publishes but does not consume stays correct only at single-replica scale, so enable consumption whenever running more than one replica.

(permission-service-notifications-configuration)=
## Configuration

All notification settings live under the top-level `notifications` key, split into a `publisher` section (events this service sends) and a `consumer` section (events this service receives).

| Value                                                     | Default                                    | Purpose |
|-----------------------------------------------------------|--------------------------------------------|---------|
| `notifications.publisher.enabled`                         | `false`                                    | Master switch for publishing. When `false`, the service starts without an event publisher and every policy write silently skips notification. |
| `notifications.publisher.endpoint`                        | `http://event-aggregation-service:50051`   | gRPC endpoint of the Notification Service's Event Publishing API. Required when publishing is enabled. Use the in-cluster service name or the ingress URL for cross-cluster deployments. |
| `notifications.publisher.tls.enabled`                     | `false`                                    | Enables TLS on the outbound connection to the Event Publishing API. Use an `https://` endpoint so its hostname becomes the default peer name. |
| `notifications.publisher.tls.caCertificateSecretRef.name` | `""`                                       | Existing Kubernetes secret holding a PEM-encoded CA bundle used to verify the server certificate. Empty falls back to the platform's root certificate store. |
| `notifications.publisher.tls.caCertificateSecretRef.key`  | `ca.crt`                                   | Key inside that secret holding the CA bundle; also the projected file name inside the pod. |
| `notifications.publisher.tls.serverName`                  | `""`                                       | Overrides the TLS server name when the endpoint is an IP address or otherwise diverges from the certificate. |
| `notifications.consumer.enabled`                          | `false`                                    | Master switch for the consumer. When `false`, the service does not subscribe and replicas rely solely on cache TTL. Enable this whenever the deployment runs more than one replica. |
| `notifications.consumer.endpoint`                         | `http://event-consumer-service:50051`      | gRPC endpoint of the Notification Service's Consumer API. |
| `notifications.consumer.tls.*`                            | same as publisher                          | TLS settings for the consumer connection (`enabled`, `caCertificateSecretRef`, `serverName`), mirroring `notifications.publisher.tls`. |

The consumer values map to the `ENABLE_NOTIFICATION_CONSUMER`, `EVENT_CONSUMER_GRPC_ENDPOINT`, and `EVENT_CONSUMER_TLS_*` environment variables.

Neither section carries credentials of its own: publishing and consuming both reuse the shared `serviceIdentity` token described in {ref}`Authenticating to the Notification Service <permission-service-notifications-authenticating-to-the-notification-service>`.

(permission-service-notifications-configuring-notifications-in-the-helm-chart)=
### Configuring notifications in the Helm Chart

A deployment that both publishes changes and keeps its replicas in sync enables the two sections together:

```yaml
notifications:
  publisher:
    enabled: true
    endpoint: "http://event-aggregation-service:50051"
  consumer:
    enabled: true
    endpoint: "http://event-consumer-service:50051"
```

At startup, the service opens one gRPC channel to `notifications.publisher.endpoint` and reuses it for every published event. If the initial connection fails, a warning is logged and the service still starts — notifications are simply disabled for that process lifetime; restart the pod once the endpoint is reachable.

(permission-service-notifications-enabling-the-consumer)=
### Enabling the cross-replica consumer

Cross-replica cache invalidation is what `notifications.consumer.enabled` turns on: the replica subscribes to the Consumer API and invalidates its local caches when another replica performs a write. Without it, a replica that did not handle the write keeps serving its cached decisions until they expire.

When the Consumer API enforces authorization, the Permission Service principal needs the `event-consumer-service` consume permits described in {ref}`Actions and resources for Omniverse services <permission-service-policy-language-actions-and-resources-for-omniverse-services>`.

(permission-service-notifications-authenticating-to-the-notification-service)=
### Authenticating to the Notification Service

Publishing events is itself an authorized action (`Action::"event-aggregation-service:publish-event"` — see {ref}`event-aggregation-service <permission-service-policy-language-event-aggregation-service>`), so the Permission Service must present a bearer token on every outgoing gRPC call. That token is produced by the shared **`serviceIdentity`** machinery used for every outgoing call the service makes; the `notifications` section does not have its own credentials. See {ref}`Outbound service identity (serviceIdentity) <permission-service-authentication-outbound-service-identity-serviceidentity>` for the full reference on each mode.

`serviceIdentity` offers two mutually exclusive modes — enable exactly one whenever `notifications.publisher.enabled` or `notifications.consumer.enabled` is `true`, since both publishing and consuming authenticate through it:

- **Client-credentials flow** (`serviceIdentity.clientCredentials.enabled: true`). The service exchanges a client id plus a client secret at the configured OpenID token endpoint and caches the resulting access token, refreshing it when it expires or when the Notification Service returns `Unauthenticated`.

  ```yaml
  serviceIdentity:
    clientCredentials:
      enabled: true
      openIdConfigurationUri: "https://idp.example.com/.well-known/openid-configuration"
      clientId: "permission-service"
      clientScope: "notifications.publish"
      clientSecretRef:
        name: permission-service-client-credentials
        key: clientSecret
      # optional: extra headers/body params required by your IdP
      # extraHeaders:
      #   X-Tenant: "omniverse"
      # extraBodyParams:
      #   audience: "https://notifications.example.com"
  ```

  The referenced Kubernetes secret is mounted at `/etc/secrets/client-credentials/<key>` and the client secret is never written to a value that would end up in a ConfigMap. When `openIdConfigurationUri` is omitted under `clientCredentials`, the chart falls back to the top-level `openId.openIdConfigurationUri`.

- **Access-token file** (`serviceIdentity.accessTokenFile.enabled: true`). A sidecar container writes a pre-provisioned access token to a shared in-memory volume at `/etc/secrets/access-token/token`. The Permission Service reads that file and watches it for changes at the configured `pollInterval`.

  ```yaml
  serviceIdentity:
    accessTokenFile:
      enabled: true
      pollInterval: "15s"
      provisionerSidecar:
        container:
          name: token-provisioner
          image: registry.example.com/token-provisioner:1.0.0
          # the chart injects a `token-exchange` volume mount automatically
  ```

  Use this mode when an external provisioner (for example a workload identity mechanism) already produces short-lived tokens and you only need Permission Service to pick them up.

When neither mode is enabled, outgoing gRPC calls are made without a bearer token — accepted only by a Notification Service that does not require authentication (for example, in tests). In production both Permission Service **and** the Notification Service should have authentication turned on.

Regardless of the authentication mode, the principal the Permission Service presents must be allowed to publish through the Notification Service's own authorization. When that authorization is itself enforced by a Permission Service deployment, the principal needs a permit similar to:

```text
permit(
  principal == Principal::"permission-service",
  action == Action::"event-aggregation-service:publish-event",
  resource
);
```

Grant it through `database.init.policies` on the Permission Service that guards the Notification Service, exactly like any other service-to-service call — see {ref}`Granting Administrative Permissions <permission-service-deployment-granting-administrative-permissions>` for the mechanics.

(permission-service-notifications-troubleshooting)=
## Troubleshooting

- **No events arrive after a policy write.** Check that `notifications.publisher.enabled` is `true`, that the pod logs contain the `Connected to Event Aggregation Service` line emitted at startup, and that the policy write itself succeeded — `201 Created` for a new policy or `200 OK` for an update; failed writes never publish.
- **Publisher logs `Failed to publish <event_type> event: <error>`** (for example, `Failed to publish omni.permissions.changed event: ...`) at `warn` level. The Notification Service rejected the RPC or the channel broke. When the status code is `Unauthenticated`, the next publish automatically refreshes the service token; when it is `Unavailable`, the gRPC channel will reconnect on the next publish. Persistent errors indicate a deployment or authorization problem on the Notification Service side. The `notification.publish.count` metric (see {ref}`Notification metrics <permission-service-observability-notification-metrics>`) breaks these outcomes down by `outcome` so a dashboard or alert doesn't require grepping logs.
- **Consumers receive events but caches keep stale data.** Confirm the consumer's filter group also matches events with an empty `resource_id` (see {ref}`Consuming the events <permission-service-notifications-consuming-the-events>`); events from policies without a pinned resource are delivered without a `resource` field and are equivalent to `resource_id = ""` on the wire.
- **A replica keeps serving stale decisions after another replica writes.** The built-in cross-replica consumer is either disabled or disconnected on that replica. Check that `notifications.consumer.enabled` is `true`, that the pod logs contain the `Subscribed to permission and metadata change events` line, and that the `notification.consume.connected` gauge (see {ref}`Notification metrics <permission-service-observability-notification-metrics>`) reads `1`. When it is stuck at `0`, inspect the reconnect/`Unauthenticated` warnings and the debug invalidation logs; a persistent `0` usually points to an unreachable Consumer API or missing `event-consumer-service` consume permits.

(permission-service-notifications-see-next)=
## See next

- {ref}`Observability <permission-service-observability-observability>` — the metrics and logs that show whether publishing and consuming actually work.

Related pages:

- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` — the write endpoints that trigger notifications.
- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — the in-process evaluation cache that these events let downstream services mirror.
- {ref}`Permission Service Configuration <permission-service-deployment-permission-service-configuration>` — full Helm chart reference including `serviceIdentity`.
- {ref}`Authenticating with an OpenID Connect Identity Provider <permission-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — full reference for `serviceIdentity.clientCredentials` and `serviceIdentity.accessTokenFile` used to authenticate calls to the Notification Service.
- {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>` — `event-aggregation-service` and `event-consumer-service` actions used when authorizing publishers and consumers.
