# Permissions for publishers and consumers

> **Applies to:** Event Aggregation Service and Event Consumer Service authorization · **Released:** Aggregation 1.6.0 / Consumer 1.11.0 · **This repo's snapshot:** Aggregation 1.7.0 / Consumer 1.12.0 (no permission-model changes between them) · **Sources:** [aggregation `app/lib/permissions/permissions.py`](../../../../../notifications/reference/event-aggregation-service/app/lib/permissions/permissions.py), [consumer `app/lib/permissions/`](../../../../../notifications/reference/event-consumer-service/app/lib/permissions/permissions.py), [`notification_common/permissions/`](../../../../../notifications/reference/common/notification_common/permissions/permissions_client.py), [consumer `README.md`](../../../../../notifications/reference/event-consumer-service/README.md) → Permissions, [`docs/permissions.md`](../../../../../notifications/reference/event-consumer-service/docs/permissions.md)

Authorization is enforced only when the operator turns it on ([deploy → authentication and authorization](../../deploy/notifications.md#authentication-and-authorization)). Design your client for the enforced case. Policies are evaluated by the Permission Service (Cedar, PARC model); policy authoring and the Permission Service API are in [permissions API](../../apis/permissions.md) and [custom permissions adapter](../custom-permissions-adapter.md).

## How the services ask

Each check sends the caller's own bearer token, an action `{service: <calling service's pyproject name>, name: <action>}`, and optionally a resource, to the Permission Service at `/v1beta/authorization/`. Results are cached per token, action, and resource for `permissions.ttl_seconds` (600 s), so a revoked grant can keep working for up to that long. In Cedar the action appears as `Action::"<service>:<action>"`.

## Publishing

| Check | Action | Resource |
|---|---|---|
| Every publish (single or batch; each distinct type in a batch) | `event-aggregation-service:publish-event` | `EventType::"<event_type>"` |

```
permit(
  principal == Principal::"<publisher-principal-id>",
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"myapp.render.completed"
);
```

One policy per event type. The Storage Service's principal needs this for `omni.storage.created`, `.deleted`, `.dir_created`, `.dir_deleted` (or its configured `notificationClient.eventTypes`). The principal ID is the configured claim (`openId.principalIdClaim`, default `sub`) from the token, or the `id` from the User Info Service.

## Consuming

Gates (consumer): `permissions.enabled` covers the Permission Service checks; `permissions.storage_permissions.enabled` separately covers Storage API checks. Each is independent.

| Operation or event | Queue type | Check |
|---|---|---|
| `CreateDurableQueue`, **and** `UpdateDurableQueue` | durable | `event-consumer-service:create-durable-queues` |
| Consume (all durable consume RPCs and SSE) | durable | `event-consumer-service:consume-durable-queues` |
| `DeleteDurableQueue` | durable | `event-consumer-service:delete-durable-queues` |
| `ManageDeadLetteredEvents` (at stream open) | — | `event-consumer-service:inspect-and-replay-all-dlq` |
| `omni.storage.created` | durable | `event-consumer-service:consume-all-storage-create-events` |
| `omni.storage.deleted` | durable | `event-consumer-service:consume-all-storage-delete-events` |
| `omni.storage.dir_created`, `omni.storage.dir_deleted` | durable | **Never delivered when `permissions.enabled` is true** (see below) |
| Any `omni.storage.*` | non-durable | Storage API listing with the caller's token (storage gate) |
| Event type listed in `permissions.content` `event_permissions` | either | Configured action on `EventResourceId::"<resource_id>"` |
| Any other event type | either | No check — delivered |

All durable-queue grants are **deployment-wide**: a principal with `consume-durable-queues` can consume any queue whose id it knows; `delete-durable-queues` can delete any; `create-durable-queues` can re-filter any. Grant them to the service accounts that own durable queues, not to user groups. `inspect-and-replay-all-dlq` exposes every dead letter in the deployment.

```
permit(principal == Principal::"<thumbnail-service-id>",
       action in [Action::"event-consumer-service:create-durable-queues",
                  Action::"event-consumer-service:consume-durable-queues",
                  Action::"event-consumer-service:delete-durable-queues",
                  Action::"event-consumer-service:consume-all-storage-create-events"]);
```

Whether `action in [...]` lists are accepted depends on the Permission Service's Cedar schema; one `permit` per action is the conservative form.

**Durable directory events.** In this snapshot's code, a durable queue with `permissions.enabled: true` maps only `omni.storage.created` and `omni.storage.deleted` to actions; `dir_created` and `dir_deleted` fall through to the fine-grained path, which refuses storage types, so they are silently dropped. With `permissions.enabled: false` they are delivered. The source package's mapping of directory events to the create/delete actions does not match the code. A durable consumer that needs directory events under enforcement must derive them from file events.

## Storage events on non-durable streams

With the storage gate on, the consumer lists the event's folder through the Storage API (first endpoint in `permissions.storage_permissions.endpoints`) using the subscriber's token. A subscriber receives a folder's events if it may list that folder — including an empty or new folder from Consumer 1.12.0 — and none if the listing is denied. The service caches decisions (optionally in Redis with `redis.enabled`) and rechecks a denied folder on its next event, then at most every 30 s. Deletes are delivered if the subscriber could previously see the file or folder. If the endpoint points at the wrong Storage deployment, listings fail with `400`/`404` and the service logs a `WARNING`; events are dropped, not errored.

## Fine-grained permissions for your own event types

Configure in the consumer chart; the chart renders it to the file at `permissions.yaml_file_path` (default `/app/config/permissions.yaml`, env `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH`) and appends the storage endpoints itself:

```yaml
permissions:
  enabled: true
  content: |-
    event_permissions:
      - event_type: myapp.render.completed
        action: read-render
      - event_type: myapp.project.updated
        action: read-project
```

For each matching event, the consumer checks `event-consumer-service:<action>` on `EventResourceId::"<resource_id>"` with the subscriber's token, and drops the event if denied:

```
// Scoped
permit(principal == Principal::"<user-id>",
       action == Action::"event-consumer-service:read-project",
       resource == EventResourceId::"/projects/p1");
// Unscoped
permit(principal == Principal::"<admin-id>",
       action == Action::"event-consumer-service:read-project",
       resource);
```

Use the `event-consumer-service:` prefix. The consumer README's examples with `event-aggregation-service:read-…` are wrong: the check uses the consumer's own service name. Each delivered event costs a Permission Service call (cached), so high-rate types with per-resource policies add latency and load; consider coarser resources.

For authorization that depends on message contents, put your own service between end clients and the consumer: it consumes with a service account and filters per user, as the consumer README recommends.

## Testing

The conformance suites do not test authorization. Test with real tokens: an unauthorized publish returns `403`; a consumer without a grant receives nothing for that type (non-durable) or is refused at open (durable operations); a revoked grant stops working within the cache TTL.
