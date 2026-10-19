(thumbnail-service-notifications-service-notifications)=
# Service notifications

Thumbnail Service both consumes and publishes events through the **Notification Service**. It consumes storage lifecycle events from a durable queue (see {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>`), and it publishes three event types of its own: one that carries generation work to the worker, and two that tell any interested consumer that a thumbnail association was created or removed. This page describes what each published event is for, when it is emitted, the event schema on the wire, how consumers receive them, and how to enable and credential the publisher in the Helm chart.

Every outgoing event goes to the Notification Service's **Event Aggregation API** at `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT` (Helm `notifications.eventAggregation.endpoint`), authenticated with the shared `serviceIdentity` credentials; see {ref}`Authenticating to the Notification Service <thumbnail-service-notifications-authenticating-to-the-notification-service>` below.

(thumbnail-service-notifications-events-the-service-publishes)=
## Events the service publishes

| Event type | Purpose | Consumed by |
|---|---|---|
| `omni.thumbnail.generate` | Durable hand-off of **one thumbnail's worth of work** from the service to the worker: source, fully resolved specification, and the exact destination address. | `task-worker` only, through **WorkerEventQueue**. Not intended for third-party consumers. |
| `omni.thumbnail.associated` | A thumbnail has been associated with a target resource — either because the worker finished a generation, or because a caller registered a pre-generated thumbnail through `AssociateThumbnail`. | Clients that cache or render thumbnails, indexers, audit pipelines. |
| `omni.thumbnail.disassociated` | An association was removed — by the targeted `DisassociateThumbnail` API or by the cleanup that follows a storage `omni.storage.deleted` event. | The same consumers, to invalidate what they hold. |

Thumbnail Service is the **only** publisher of association events. The worker never publishes them: after uploading a thumbnail it calls `AssociateThumbnail` on the service, and the service emits `omni.thumbnail.associated` after its own metadata write commits. See {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>`.

(thumbnail-service-notifications-when-events-are-published)=
## When events are published

(thumbnail-service-notifications-generate-events)=
### Generate events

One `omni.thumbnail.generate` event is published per thumbnail to be produced — never a batch:

| Trigger | Events published |
|---|---|
| Eligible `omni.storage.created` delivery on StorageEventQueue | One per configured `THUMBNAIL_SIZES` entry, using the `THUMBNAIL_*` defaults for every non-dimension field. Every size is attempted even after one fails; any failure fails the batch, and a retryable failure returns the delivery to the queue for redelivery. See {ref}`Acknowledgement semantics <thumbnail-service-event-consumption-acknowledgement-semantics>`. |
| `POST /v1alpha/thumbnails` / `CreateThumbnails` with specifications | One per caller specification, after the source `Stat` succeeds. Publish failures are reported per specification in the response's `failed` list. |
| `POST /v1alpha/thumbnails` / `CreateThumbnails` with no specifications | One per configured `THUMBNAIL_SIZES` entry — identical to the event-driven path. |

The generate publish is **durable dispatch**, not best-effort: on the event-driven path the publish result decides whether the storage delivery is acknowledged, and on the API path a publish that fails is reported to the caller. A publish that times out is not proof that nothing was enqueued — the broker may have accepted it before the deadline.

(thumbnail-service-notifications-association-events)=
### Association events

| Operation | Event | Count |
|---|---|---|
| `POST /v1alpha/associations` / `AssociateThumbnail` succeeds (both metadata writes committed) | `omni.thumbnail.associated` | One |
| Worker-driven association after a successful generation (same RPC) | `omni.thumbnail.associated` | One per generated thumbnail |
| `DELETE /v1alpha/associations` / `DisassociateThumbnail` removes an entry | `omni.thumbnail.disassociated` | One; none when the thumbnail was not associated (idempotent no-op) |
| `omni.storage.deleted` cleanup sweep removes entries | `omni.thumbnail.disassociated` | One per fully removed entry |

Both association events are emitted **after** the metadata write is persisted, so a consumer that reads the association back on receipt sees the new state. Failed or rejected writes publish nothing.

```{note}
Association events are **best-effort**. The publish is dispatched from a detached task after the write commits, so the API caller receives its response regardless of whether the event was delivered. If the Notification Service is unreachable or rejects the publish, the failure is logged and the association is kept. When the publisher receives `Unauthenticated`, the service refreshes its service-identity token and retries the publish once before giving up.
```

When `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT` is not configured (Helm `notifications.eventAggregation.enabled: false`), association publishes are skipped with a warning. Generate publishes are **not** skippable: with no publisher configured, generation dispatch fails as a retryable error, so a storage delivery is never acknowledged for work that was not published.

(thumbnail-service-notifications-event-schema)=
## Event schema

Every event is the `Event` message of the Notification Service's publisher API, sent with the `PublishEvent` RPC. All three share the same envelope:

| Field | Type | Value |
|---|---|---|
| `event_type` | string | `omni.thumbnail.generate`, `omni.thumbnail.associated`, or `omni.thumbnail.disassociated`. |
| `occurred_at` | `google.protobuf.Timestamp` | Time at which the service published the event. |
| `message` | `google.protobuf.Struct` | JSON-shaped object whose fields depend on `event_type`; see below. |
| `resource` | `EventResource` | `resource_id` used for consumer-side filtering; which address it carries depends on `event_type`. |

| `event_type` | `resource.resource_id` |
|---|---|
| `omni.thumbnail.generate` | The **source** `resource_address` — the file being thumbnailed. This is what WorkerEventQueue's prefix filters match. |
| `omni.thumbnail.associated` | The **association target's** `resource_address`, so consumers can filter by the source resource they display. |
| `omni.thumbnail.disassociated` | The **thumbnail's** `resource_identity` (an empty string when the stored entry recorded none). |

Numbers in `message` are JSON numbers; every enum is an `UPPER_SNAKE_CASE` string. Resource addresses are the canonical (percent-encoded) form the service stores.

(thumbnail-service-notifications-omni-thumbnail-generate)=
### `omni.thumbnail.generate`

One event is exactly one thumbnail: one source, one fully resolved specification, one destination.

| Field | Type | Description |
|---|---|---|
| `schema_version` | integer | Currently `1`. A worker that receives a **newer** version than it supports withholds the acknowledgement so the message is redelivered or dead-lettered rather than dropped; an older or unparseable version is a permanent validation failure. |
| `source.resource_address` | string | Address of the file to thumbnail. Required. |
| `source.resource_identity` | string | Storage identity of that file, as resolved by the service. Required for `FILE` targets. |
| `destination_resource_address` | string | The exact, absolute address the worker writes to, rendered by the service from `THUMBNAIL_PATH_TEMPLATE`. The worker performs no derivation of its own. |
| `target_type` | string | `FILE` or `FOLDER` — the association target type the worker passes to `AssociateThumbnail`. |
| `specification` | object | Fully resolved image-to-image specification: `width`, `height`, `encoding_format`, `transform_type`, `crop_strategy`, `quality`, `preserve_aspect_ratio`. Every field is present; a caller-supplied and a configuration-derived specification are indistinguishable on the wire. |
| `traceparent`, `tracestate` | string (optional) | W3C trace context of the publishing span, so the worker's conversion joins the same trace. Omitted when no span is active. |

```json
{
  "event_type": "omni.thumbnail.generate",
  "occurred_at": "2026-09-11T12:34:56Z",
  "message": {
    "schema_version": 1,
    "source": {
      "resource_address": "https://acct.blob.core.windows.net/assets/photos/image.jpg",
      "resource_identity": "https://acct.blob.core.windows.net/assets/photos/image.jpg?version-id=v3"
    },
    "destination_resource_address": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png",
    "target_type": "FILE",
    "specification": {
      "width": 256,
      "height": 256,
      "encoding_format": "PNG",
      "transform_type": "FIT",
      "crop_strategy": "SMART",
      "quality": 85,
      "preserve_aspect_ratio": true
    },
    "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
  },
  "resource": {
    "resource_id": "https://acct.blob.core.windows.net/assets/photos/image.jpg"
  }
}
```

The worker accepts the legacy alias `crop_direction` for `crop_strategy` and ignores unknown fields, so additive changes do not require a new `schema_version`.

(thumbnail-service-notifications-omni-thumbnail-associated)=
### `omni.thumbnail.associated`

The `message` is the same value the service stores as association metadata (see {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>`), minus `create_time`:

| Field | Description |
|---|---|
| `thumbnail.resource_address`, `thumbnail.resource_identity` | The thumbnail resource. |
| `association_target.resource_address` | The resource the thumbnail belongs to. |
| `association_target.resource_identity` | Present for `FILE` targets; **omitted entirely** for `FOLDER` targets. |
| `target_type` | `FILE` or `FOLDER`. |
| `specification` | The flat, `type`-tagged specification after normalization: `type` (`IMAGE_TO_IMAGE` or `MANUAL`), `width`, `height`, `encoding_format`, and for `IMAGE_TO_IMAGE` also `transform_type`, `crop_strategy`, `quality`, `preserve_aspect_ratio`. |

```json
{
  "event_type": "omni.thumbnail.associated",
  "occurred_at": "2026-09-11T12:35:02Z",
  "message": {
    "thumbnail": {
      "resource_address": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png",
      "resource_identity": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png?version-id=v1"
    },
    "association_target": {
      "resource_address": "https://acct.blob.core.windows.net/assets/photos/image.jpg",
      "resource_identity": "https://acct.blob.core.windows.net/assets/photos/image.jpg?version-id=v3"
    },
    "target_type": "FILE",
    "specification": {
      "type": "IMAGE_TO_IMAGE",
      "width": 256,
      "height": 256,
      "encoding_format": "PNG",
      "transform_type": "FIT",
      "crop_strategy": "SMART",
      "quality": 85,
      "preserve_aspect_ratio": true
    }
  },
  "resource": {
    "resource_id": "https://acct.blob.core.windows.net/assets/photos/image.jpg"
  }
}
```

(thumbnail-service-notifications-omni-thumbnail-disassociated)=
### `omni.thumbnail.disassociated`

Same shape as the associated event **without** the `specification` field:

```json
{
  "event_type": "omni.thumbnail.disassociated",
  "occurred_at": "2026-09-11T13:00:00Z",
  "message": {
    "thumbnail": {
      "resource_address": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png",
      "resource_identity": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png?version-id=v1"
    },
    "association_target": {
      "resource_address": "https://acct.blob.core.windows.net/assets/photos/image.jpg",
      "resource_identity": "https://acct.blob.core.windows.net/assets/photos/image.jpg?version-id=v3"
    },
    "target_type": "FILE"
  },
  "resource": {
    "resource_id": "https://acct.blob.core.windows.net/thumbnails/475673a39cfd8a2fb955ee5c352c1082/256x256/image.jpg.auto.png?version-id=v1"
  }
}
```

A disassociated event says the **metadata** link was removed. Whether the thumbnail blob still exists depends on which path removed it — the targeted API leaves the blob in place, while the storage-deleted cleanup deletes blobs the service generated. See {ref}`Blob deletion policy <thumbnail-service-associations-blob-deletion-policy>`.

(thumbnail-service-notifications-consuming-the-events)=
## Consuming the events

Consumers register with the Notification Service's Consumer API, which owns the subscription model, durable queues, and delivery semantics. Two filters are useful in practice:

- **Filter by `event_type`** — subscribe to `omni.thumbnail.associated` and `omni.thumbnail.disassociated` to track association changes and ignore other traffic on the broker.
- **Filter by `resource.resource_id`** — a consumer that only displays a subtree can add a prefix filter on the association target's address, for example:

  ```json
  {
    "filter_type": "FILTER_TYPE_STARTS_WITH_GREEDY",
    "resource_id": "https://acct.blob.core.windows.net/assets/photos/"
  }
  ```

  For `omni.thumbnail.associated` the filter matches the **source** address. For `omni.thumbnail.disassociated` it matches the **thumbnail identity**, so a consumer that wants both for one subtree should filter on `event_type` and match the association target inside `message` itself.

`omni.thumbnail.generate` is an internal hand-off. Consuming it from anywhere other than the worker's WorkerEventQueue is unsupported: the worker's queue filters on the same source-address prefixes the service watches (`NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`), and a second consumer would compete for deliveries.

```{note}
An `omni.thumbnail.associated` event proves the association metadata exists. It does not prove the blob at `thumbnail.resource_address` is readable by the consumer's principal — read access to thumbnails is governed by the storage service, exactly as for any other object.
```

(thumbnail-service-notifications-enabling-notifications-in-the-helm-chart)=
## Enabling notifications in the Helm chart

The chart exposes publishing and consuming under the root `notifications` key:

```{code-block} yaml
:caption: thumbnail-values.yaml

notifications:
  eventAggregation:
    enabled: true
    endpoint: "event-aggregation-service:50051"
  eventConsumer:
    endpoint: "event-consumer-service:50051"
    resourcePrefixes:
      - "https://acct.blob.core.windows.net/assets/"
    storageEventQueue:
      enabled: true
```

| Value | Purpose |
|---|---|
| `notifications.eventAggregation.enabled` | When `true`, injects `NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT` so the service can publish generate and association events. Default `true`. With it `false`, association events are skipped with a warning and generation dispatch fails as retryable. |
| `notifications.eventAggregation.endpoint` | gRPC endpoint of the Notification Service's Event Aggregation API. Default `event-aggregation-service:50051`. Plaintext only — an `https://` value is refused at startup. |
| `notifications.eventConsumer.endpoint` | gRPC endpoint of the Consumer API, shared by StorageEventQueue (service) and WorkerEventQueue (worker). Default `event-consumer-service:50051`. |
| `notifications.eventConsumer.resourcePrefixes` | Storage-container URL prefixes. One greedy-prefix filter per entry on both durable queues, and the input to both derived queue ids. **Required** non-empty whenever a consumer is enabled. |
| `notifications.eventConsumer.storageEventQueue.enabled` | Enables the service's storage-event consumer. Default `true`. When `false`, the service receives `DISABLE_EVENT_CONSUMPTION=true`. |

At startup the service opens one gRPC channel to the aggregation endpoint and reuses it for every publish. Provisioning and consumption of the durable queues are described in {ref}`Durable queue self-provisioning <thumbnail-service-event-consumption-durable-queue-self-provisioning>`.

(thumbnail-service-notifications-authenticating-to-the-notification-service)=
### Authenticating to the Notification Service

Publishing and consuming are authorized actions on the Notification Service side, so every call carries the service-identity bearer token produced by the shared **`serviceIdentity`** configuration — client-credentials flow or access-token file. The `notifications` section has no credentials of its own. See {ref}`Outbound service identity <thumbnail-service-authentication-outbound-service-identity-serviceidentity>` for the two modes.

The principal the service presents needs these grants on the deployment that guards the Notification Service:

| Grant | Needed for |
|---|---|
| `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate` | Publishing generate work (and, in practice, the association events). |
| `event-consumer-service:create-durable-queues` | Self-provisioning StorageEventQueue and WorkerEventQueue on startup. |
| `event-consumer-service:consume-durable-queues` | Consuming both queues. |

When those grants are enforced by a Permission Service deployment, the publish permit looks like:

```text
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"omni.thumbnail.generate"
);
```

(thumbnail-service-notifications-troubleshooting)=
## Troubleshooting

- **No `omni.thumbnail.associated` events after associations succeed.** Check that `notifications.eventAggregation.enabled` is `true` and the endpoint is reachable. A skipped publish is logged with a warning naming the missing endpoint; a rejected publish is logged at error level with the failure reason. The association itself is still committed — read it back with `GET /v1alpha/thumbnails`.
- **`CreateThumbnails` returns `502` / `INTERNAL` with "channel unconfigured" or every specification in `failed`.** The aggregation endpoint is unset or unreachable. Generate publishes are not best-effort; fix the endpoint and retry — on the event-driven path the storage delivery is retried automatically.
- **Publishes fail with `Unauthenticated` repeatedly.** One refresh-and-retry is built in; persistent failures mean the service identity is rejected by the Notification Service. Check the token source described in {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>`.
- **Publishes fail with `PermissionDenied`.** The principal lacks `event-aggregation-service:publish-event` for `omni.thumbnail.generate`. Grant it on the Notification Service's authorization deployment.
- **Generate events publish fine but no thumbnails appear.** The worker's WorkerEventQueue may not exist, or its prefix filters may not match the source addresses. Compare the `-storage-` and `-worker-` queue ids in a queue listing: matching digests mean both components agree on `resourcePrefixes`. See {ref}`Queue id digest check <thumbnail-service-troubleshooting-queue-id-digest-check>`.
- **Consumers receive `omni.thumbnail.disassociated` for entries they never saw associated.** The storage-deleted cleanup emits one event per removed entry, including entries created before the consumer subscribed. Treat a disassociated event for an unknown thumbnail as a no-op.

(thumbnail-service-notifications-see-also)=
## See also

- {ref}`Storage event consumption <thumbnail-service-event-consumption-storage-event-consumption>` — the queue this service consumes, and how the generate publish decides the storage delivery's acknowledgement.
- {ref}`Thumbnail generation worker <thumbnail-service-generation-pipeline-thumbnail-generation-worker>` — the consumer of `omni.thumbnail.generate` and the `AssociateThumbnail` call that leads to `omni.thumbnail.associated`.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — the stored value the association events mirror.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — the `serviceIdentity` credentials used for every publish.
- {ref}`Observability <thumbnail-service-observability-observability>` — spans and log lines emitted around each publish.
