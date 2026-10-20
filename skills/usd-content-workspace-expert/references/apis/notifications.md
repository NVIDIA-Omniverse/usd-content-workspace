# Notifications APIs

> **Applies to:** Publisher API (Event Aggregation Service) and Consumer API (Event Consumer Service) · **Released:** Aggregation 1.6.0 / Consumer 1.11.0; NGC contract resources `notifications-aggregation-api` and `notifications-consumer-api` `1.0.0-beta.3` · **This repo's snapshot:** Aggregation 1.7.0, Consumer 1.12.0 (each service's `pyproject.toml` and `Chart.yaml`); proto packages `v1beta` · **Sources:** [`notifications/api/protos/`](../../../../notifications/api/README.md), [aggregation `openapi.yaml`](../../../../notifications/reference/event-aggregation-service/openapi.yaml), [consumer `openapi.yaml`](../../../../notifications/reference/event-consumer-service/openapi.yaml), [consumer `README.md`](../../../../notifications/reference/event-consumer-service/README.md), [`docs/`](../../../../notifications/reference/event-consumer-service/docs/index.md), the [source skill package](../../NOTICE.md)

Wire behavior below is read from this snapshot's protos, OpenAPI files, and handlers. Confirm which service versions you are calling ([capture a deployment](../versions/capture-a-deployment.md)); rows marked "1.7.0" or "1.12.0" do not apply to a released 1.6.0 / 1.11.0 deployment. For client code, see [develop](../develop/custom-notifications-adapter/README.md). For deployment and operations, see [deploy](../deploy/notifications.md) and [operate](../operate/notifications.md).

## Contract coordinates

| Surface | Package / version | Definition in this repo | Endpoint (chart default) |
|---|---|---|---|
| Publisher gRPC | `nvidia.omniverse.notifications.publisher.v1beta.EventPublishingService` | [`event_publisher.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto) | `<aggregation-service>:50051` |
| Publisher REST | OpenAPI 3.1, `info.version: v1beta`, prefix `/api/v1beta` | [aggregation `openapi.yaml`](../../../../notifications/reference/event-aggregation-service/openapi.yaml) | `<aggregation-service>:8000` |
| Consumer gRPC | `nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService` | [`event_consumer.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto) | `<consumer-service>:50051` (standalone process: 50052) |
| Consumer REST + SSE | OpenAPI 3.1, `info.version: v1beta`, prefix `/api/v1beta` (also mounted, undocumented, at `/api/v1`) | [consumer `openapi.yaml`](../../../../notifications/reference/event-consumer-service/openapi.yaml) | `<consumer-service>:8000` |

Both snapshot OpenAPI documents move their internal API-governance ID from a top-level `tags` entry into an `x-` extension under `info`, and have no top-level `tags` list (see line 11 of each service's `openapi.yaml`). This is document metadata only; no route, schema, or status code changed.

Released contracts: NGC resources [`notifications-aggregation-api`](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/notifications-aggregation-api) and [`notifications-consumer-api`](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/notifications-consumer-api) `1.0.0-beta.3` (entitlement required; see [artifact access](../deploy/artifact-access.md)). The previous published inventory used `1.0.0-beta.2`, which lacks `queue_id` on create and `nack_delivery_tags`. The public [HTML reference](https://docs.omniverse.nvidia.com/ovstorage/notification-api/1.0.0-beta.2/index.html) carries an independent documentation version. See [notification-protos](../components/notification-protos.md#version-mapping) for which fields arrived when; generate clients from the definitions matching the deployed service.

Resolve real endpoints from the rendered Services or from Discovery (`ListServices` types `notification-aggregation` and `notification-consumer`; see [Discovery API](discovery.md)), not from these defaults. Through Contour ingress, REST is at `https://<host>.<domain>` and gRPC at `https://grpc.<host>.<domain>` (default hosts `event-aggregation`, `event-consumer`).

## Authentication

Send `Authorization: Bearer <token>` on REST and `authorization: Bearer <token>` gRPC metadata on every call except the unauthenticated surfaces below. The services resolve the token before any handler runs; an unresolvable token is `401` / `UNAUTHENTICATED`, and a token whose configured principal claim (default `sub`) is missing or empty is refused. An unreachable IdP under `principalSource: jwt` is `500` / `INTERNAL`. Whether the signature is checked depends on the deployment's `principalSource`; see [deploy](../deploy/notifications.md#authentication-and-authorization).

Unauthenticated by design: `GET /api/v1beta/health` (both), `GET /api/v1beta/metrics/channel-pool` (consumer), `/docs`, `/redoc`, `/docs/oauth2-redirect`, `/openapi.json`, gRPC health, reflection, and Discovery `ListServices`.

## Event model

Published event (`publisher.v1beta.Event`, REST `event` object):

| Field | Type | Required | Limit (enforced on both APIs from Aggregation 1.7.0) |
|---|---|---|---|
| `event_type` | string | yes | ≤ 256 Unicode code points; ≤ 5 dot-separated parts (routing-key encoding, `routing_max_event_type_parts`) |
| `message` | `google.protobuf.Struct` / JSON object | yes | ≤ 65536 bytes as Python `json.dumps` default encoding (non-ASCII escaped as `\uXXXX`; gRPC numbers become doubles) |
| `occurred_at` | `Timestamp` / RFC 3339 string | yes | REST string ≤ 64 characters; gRPC seconds `0` counts as unset and is refused |
| `resource.resource_id` | string | no | ≤ 2048 code points; omitted resource = `""` |

The service does not validate `event_type` names or `message` schemas; they are a contract between publisher and consumers. Use `service.entity.action` names. `resource_id` should be a slash-separated, hierarchical path.

Consumed event (`consumer.v1beta.Event`, SSE `data`): `event_type`, `principal_identity` (the publisher's resolved principal), `occurred_at`, `published_at` (when the aggregation service published it), `message`. The consumed event does **not** carry `resource_id`; only `DeadLetteredEvent` surfaces it.

Built-in storage event types (constants in [`notification_common/storage/event_types.py`](../../../../notifications/reference/common/notification_common/storage/event_types.py), published by the Storage Service when `config.notificationClient` is enabled): `omni.storage.created`, `omni.storage.deleted`, `omni.storage.dir_created`, `omni.storage.dir_deleted`. Directory events are synthetic and best-effort. Only these exact strings get storage permission handling. See the [Storage events reference](../../../../storage/reference/storage-service/docs/helm/storage-backend-events.rst) and the [Storage API page](storage.md).

## Publisher API

| gRPC RPC | REST | Purpose |
|---|---|---|
| `PublishEvent(PublishEventRequest{event})` → `PublishEventResponse{result}` | `POST /api/v1beta/events` body `{"event": {...}}` → `{"result": {...}}` | One event |
| `BatchPublishEvents(BatchPublishEventsRequest{events})` → `BatchPublishEventsResponse{results}` | `POST /api/v1beta/events/batch` body `{"events": [...]}` → `{"results": [...]}` | 1–1000 events, published in parallel, results in request order |
| — | `GET /api/v1beta/health` → `{"status": "healthy"}` | Health |

`PublishingResult` is `{event, success, failure_reason}`. A broker-side failure is reported as `success: false` inside an HTTP `200` / gRPC `OK`. Check every result; retry only failed entries, with backoff, and make consumers tolerate duplicates.

Refusal order and codes (snapshot):

| Condition | REST | gRPC |
|---|---|---|
| Body over `rest.maxRequestBytes` (4 MiB) — checked before auth; response sets `Connection: close` | `413` | — |
| gRPC message over the transport's 4 MiB inbound cap — checked before auth | — | `RESOURCE_EXHAUSTED` (names no field) |
| Unresolvable token | `401` | `UNAUTHENTICATED` |
| Empty batch | `400` | `INVALID_ARGUMENT` |
| Schema or limit violation (names the field, e.g. `events[1].message`) | `422` | `INVALID_ARGUMENT` |
| No `publish-event` permission on an event type (all distinct types in a batch are checked first) | `403` | `PERMISSION_DENIED` |
| Server error | `500` | `INTERNAL` |

Aggregation 1.6.0 removed the `503` "channel pool exhausted" response from the spec; it was never produced. Do not build retry logic around `503` from the publisher. The spec also declares `429` on batch; the reference service has no rate limiter that produces it in this snapshot (unverified that any deployment front-end does).

**1.6.0 versus 1.7.0:** in 1.6.0 gRPC enforced none of the per-event limits and neither API capped `message` size, so a 1.6.0 deployment can accept requests a 1.7.0 deployment refuses with `422` / `INVALID_ARGUMENT`.

## Consumer API

### Operations

| gRPC RPC | Kind | REST | Notes |
|---|---|---|---|
| `ConsumeNonDurableEvents` | bidi stream | `GET /api/v1beta/events/stream` (SSE) | Ephemeral queue; reconnect token; mid-stream filter updates on gRPC |
| `ConsumeDurableEventsWithAcks` | bidi stream | — | Recommended durable consume; explicit `ack_delivery_tags` / `nack_delivery_tags` |
| `ConsumeDurableEvents` | server stream, **deprecated** | `GET /api/v1beta/events/stream-durable?queue_id=` (SSE) | Server acknowledges after a short delay; loss window on disconnect |
| `CreateDurableQueue` | unary | `POST /api/v1beta/queues/durable` | Optional `queue_id` (ensure-exists); returns `queue_id`, `created` |
| `UpdateDurableQueue` | unary | `PATCH /api/v1beta/queues/durable` body `{queue_id, current_filter_groups, new_filter_groups}` | gRPC response is empty; REST returns `{"success": true}` |
| `DeleteDurableQueue` | unary | `DELETE /api/v1beta/queues/durable?queue_id=` | gRPC response is empty; REST returns `{"success": true}`; repeat delete → `404` / `NOT_FOUND` |
| `ManageDeadLetteredEvents` | bidi stream | — (gRPC only) | Operator DLQ triage; permission `inspect-and-replay-all-dlq` |
| — | — | `GET /api/v1beta/metrics/channel-pool` | `{utilization_percent, channels_in_use, channels_total}` — units are stream slots, per pod |
| — | — | `GET /api/v1beta/health` | `{"status": "healthy"}` |

The former `POST /api/v1beta/events/ack-durable` was removed in Consumer 1.10.0, and REST durable SSE payloads no longer carry `session_id` or `delivery_tag`.

### Filters

`FilterGroup{event_type, filters[]}`; multiple groups subscribe to multiple types; filters within a group are OR'd; a group with no filters matches every event of that type. REST `filter_groups` is a URL-encoded JSON array (query value ≤ 65536 characters; ≤ 100 filters per group; create and update bodies allow ≤ 100 groups; `event_type` 1–1024 characters); REST `filter_type` values are lowercase (`eq`, `starts_with_lazy`, `starts_with_greedy`, `unspecified`), gRPC uses `FILTER_TYPE_*`.

| Filter type | `resource_id: "folder_1"` matches | With `resource_id: ""` |
|---|---|---|
| `EQ` (and `UNSPECIFIED`) | exactly `folder_1` | error |
| `STARTS_WITH_LAZY` | `folder_1/astronaut.usd`, not `folder_1/a/b.usd`, not `folder_10/x` | events with no or empty resource, and one-segment resources (`something`, `something/`) |
| `STARTS_WITH_GREEDY` | `folder_1/...` at any depth | every event of the type |

### SSE framing (REST streams)

```
event: connected
id: <reconnect-token>
data: {"message":"Connection established","reconnect_token":"<reconnect-token>"}

event: event
data: {"event_type":"omni.storage.created","principal_identity":"...","occurred_at":"...","published_at":"...","message":{...}}

event: error
data: {"message":"Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.","status_code":500,"grpc_code":...}
```

The stream status is already `200` when it opens, so later failures (`401`, `403`, `404`, `500`) arrive as an `error` event whose `data` is the shared `ErrorDetailsModel` (`message`, `status_code`, `grpc_code`), and the stream closes. Key client logic on `status_code`; the JSON encoding of `grpc_code` is unverified in this snapshot. Read `error` events even after a `200`. Event frames carry no `id`; only `connected` does. The source package's examples showing `event: message` and `event: reconnect_token` frames do not match this service.

### Non-durable reconnect and filter updates

- The reconnect token (gRPC `reconnect_token`, SSE `connected` `id`) names the queue. It is 1–200 printable ASCII characters; anything else is `422` / `INVALID_ARGUMENT`. Issued tokens are UUIDs.
- REST: send it in the `Last-Event-ID` header (browsers do this automatically). The `reconnect_token` query parameter is deprecated but still accepted; query strings end up in proxy logs.
- The queue survives a disconnect for `rabbitmq.non_durable_queue_ttl_seconds` (default 20 s). After that, a token reconnect is `404` / `NOT_FOUND`: open a fresh stream with `filter_groups` and no token. Events published while the queue was absent are lost.
- A filter update sends `filter_groups` **and** `previous_filter_groups`. A token-only reconnect resumes on existing bindings. `filter_groups` without `previous_filter_groups` on a reconnect is `INVALID_ARGUMENT`.
- gRPC (Consumer 1.11.0+): every response carries `supports_filter_update_acks: true`; each processed update gets one response with `filter_update_ack{applied_filter_groups, unchanged}` and no events. After a drop, reconnect with the token and replay, in order, each update not yet acknowledged. Only one client per queue may send updates.
- REST: the `connected` event after a filter-update reconnect is the confirmation.
- Several clients presenting one token share the queue and split its events; each is authorized with its own token.

### Durable queues

- `queue_id` matches `^[A-Za-z0-9._-]{1,200}$` on every durable operation (`422` / `INVALID_ARGUMENT` otherwise). Ids share one deployment-wide namespace; the broker queue is `event-queue:<queue_id>`. Use a UUID or `<service>-<purpose>`.
- Create is ensure-exists: an existing id is returned unchanged with its original filters and `created: false`, whoever created it. REST returns `200` when no id was supplied or the id existed, `201` when a supplied id created the queue.
- A queue with no attached consumer for `policyHook.policy.definition.expiresMs` (default 24 h) is deleted with its messages (not dead-lettered). Consume then returns `404` / `NOT_FOUND`; recover by calling create with the stored `queue_id` **and** `filter_groups`. Publishing and re-creating do not keep a queue alive.
- With-acks: the first request carries `queue_id`; later requests carry tags. Acks are processed before nacks; unknown or settled tags are skipped. A nack requeues for immediate redelivery. The delivery count is cumulative over the message's life (nack, disconnect, channel close); past `deliveryLimit` (10 as shipped) the message is dead-lettered. Durable prefetch is 20 per consumer.
- Message TTL (24 h) and max length (100000, `drop-head`) also dead-letter messages.

### Dead-letter triage (`ManageDeadLetteredEvents`)

Open with an empty `ManageDeadLetteredEventsRequest{}`; the server streams `dead_lettered_event{reason, origin_queue, delivery_count, dead_lettered_at, event, resource_id, delivery_tag}`; send `actions: [{delivery_tag, action}]` with `ACTION_REPLAY`, `ACTION_DISCARD`, or `ACTION_LEAVE`; each action gets one `action_outcome{delivery_tag, outcome, detail}` (`OUTCOME_REPLAYED`, `OUTCOME_DISCARDED`, `OUTCOME_LEFT`, `OUTCOME_ORIGIN_QUEUE_DELETED`, `OUTCOME_REPLAY_FAILED`). Reasons are `REASON_DELIVERY_LIMIT`, `REASON_EXPIRED`, `REASON_MAXLEN`. Tags are valid only on the live stream. Replay goes only to the recorded origin queue. Lacking the permission is `PERMISSION_DENIED` at stream open. Operational guidance: [operate](../operate/notifications.md#dead-letter-triage).

### Consumer error contract

| Condition | REST | gRPC | Client action |
|---|---|---|---|
| Unresolvable or expired token (at open, or mid-stream) | `401` | `UNAUTHENTICATED` | Refresh token, reconnect with same token / `queue_id` |
| Not permitted | `403` | `PERMISSION_DENIED` | Fix policy |
| Bad filters, bad id/token shape | `400` / `422` | `INVALID_ARGUMENT` | Fix request |
| Queue gone (reaped, deleted, non-durable grace expired) | `404` | `NOT_FOUND` | Durable: recreate with stored id + filters. Non-durable: fresh stream |
| Broker cancelled the consumer; queue normally survives | SSE `error` `500` | `INTERNAL` with the fixed message above | Reconnect; do not recreate. Detection can take up to ~15 s (×1.15 jitter) |
| Pod at stream capacity or draining; durable pool full | `503` | `RESOURCE_EXHAUSTED` | Back off and reconnect; ingress may already have retried |
| Idle connection cut by an L4 load balancer (AWS NLB/ALB, Azure LB idle timeouts, typically 60 s–4 min) | connection drop | `UNAVAILABLE` | Reconnect with token / `queue_id` |
| IdP unreachable | `500` | `INTERNAL` | Retry later |

A token that cannot be resolved on a pod at capacity is answered `401`, not `503`, and is not retried by ingress.

## Permissions summary

Action names are prefixed with the service name: `event-aggregation-service:publish-event` (resource `EventType::"<event_type>"`), and `event-consumer-service:` + `create-durable-queues` (also authorizes update), `consume-durable-queues`, `delete-durable-queues`, `inspect-and-replay-all-dlq`, `consume-all-storage-create-events`, `consume-all-storage-delete-events`, or a configured fine-grained action (resource `EventResourceId::"<resource_id>"`). Durable-queue permissions are deployment-wide, not per queue. Details and policy examples: [develop → permissions](../develop/custom-notifications-adapter/permissions.md); Permission Service API: [permissions](permissions.md).
