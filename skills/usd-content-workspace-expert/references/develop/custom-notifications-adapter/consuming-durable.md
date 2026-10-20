# Consuming with durable queues

> **Applies to:** Event Consumer Service durable-queue RPCs and REST routes · **Released:** 1.11.0 (contract `notifications-consumer-api:1.0.0-beta.3`; `queue_id` on create and `nack_delivery_tags` are not in beta.2 / Consumer 1.10.0) · **This repo's snapshot:** 1.12.0 · **Sources:** [`event_consumer.proto`](../../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto), [consumer `openapi.yaml`](../../../../../notifications/reference/event-consumer-service/openapi.yaml), [consumer `README.md`](../../../../../notifications/reference/event-consumer-service/README.md) (Choosing a durable queue id, Durable-queue lifecycle, Settling a durable delivery), [`docs/consuming-events.md`](../../../../../notifications/reference/event-consumer-service/docs/consuming-events.md)

Use a durable queue when your service must resume after restarts and process what arrived while it was down — the Thumbnail Service is the in-stack example. Durable queues are RabbitMQ quorum queues created ahead of time, consumed by id, and bounded by the deployment's `event-queue-limits` policy. Typical clients are service accounts with deployment-wide queue permissions ([permissions](permissions.md)).

## Lifecycle at a glance

1. **Choose a `queue_id`** — `^[A-Za-z0-9._-]{1,200}$`, unique across the deployment: a UUID or `<service-name>-<purpose>` (`thumbnailer-uploads`), never a generic word. Ids share one namespace; two callers using the same id share (and can delete) one queue.
2. **Persist `queue_id` and `filter_groups` together** as configuration. You need both to recreate the queue.
3. **Create (ensure-exists)** at startup, every time.
4. **Consume with acks**, settling each delivery.
5. **On `NOT_FOUND`**, go back to step 3 and reconcile from the source for the gap.
6. **Delete** only when decommissioning.

## Create (ensure-exists)

```bash
curl -sS -X POST "https://event-consumer.<domain>/api/v1beta/queues/durable" \
  -H "Authorization: Bearer $SERVICE_TOKEN" -H "Content-Type: application/json" \
  -d '{"queue_id":"myapp-renders",
       "filter_groups":[{"event_type":"myapp.render.completed",
                         "filters":[{"filter_type":"starts_with_greedy","resource_id":"/projects/"}]}]}'
# 201 {"queue_id":"myapp-renders","created":true}   new queue
# 200 {"queue_id":"myapp-renders","created":false}  existing queue, returned UNCHANGED with its original filters
```

gRPC: `CreateDurableQueue(CreateDurableQueueRequest(queue_id=..., filter_groups=[...]))` → `CreateDurableQueueResponse{queue_id, created}`. If `created` is false and your stored filters differ from what the queue has, change them with `UpdateDurableQueue`; create never rewrites filters. Omitting `queue_id` makes the server generate a UUID (REST `200`) — but then a retried create after a lost response orphans a second queue, so supply your own id.

**Consumer 1.10.0 (beta.2) clients** cannot pass `queue_id`; recreating a lost queue there returns a new id that must be persisted.

## Consume with explicit acknowledgement (gRPC)

`ConsumeDurableEventsWithAcks` is bidirectional. The first request carries `queue_id`; each response carries `events` and a `delivery_tag`; later requests carry `ack_delivery_tags` and/or `nack_delivery_tags`. Process on a worker, not on the response loop, and back off before a nack:

```python
import os, queue, threading, time, grpc
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc as pb_grpc

stub = pb_grpc.EventConsumerServiceStub(
    grpc.secure_channel("grpc.event-consumer.<domain>:443", grpc.ssl_channel_credentials()))
QUEUE_ID, FILTERS = load_queue_config()           # persisted together

work, settle = queue.Queue(), queue.Queue()

def worker():
    while (item := work.get()) is not None:
        tag, events = item
        try:
            for e in events:
                process(e)                       # idempotent: redelivery happens
            settle.put(([tag], []))
        except TransientError:
            time.sleep(retry_delay())            # nack redelivers immediately; wait first
            settle.put(([], [tag]))
        except PermanentError:
            record_poison(events)
            settle.put(([tag], []))              # ack to drop; do not burn the retry budget

def requests():
    yield pb.ConsumeDurableEventsWithAcksRequest(queue_id=QUEUE_ID)
    while (s := settle.get()) is not None:
        yield pb.ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=s[0], nack_delivery_tags=s[1])

def run_once():
    md = [("authorization", f"Bearer {os.environ['SERVICE_TOKEN']}")]
    stub.CreateDurableQueue(pb.CreateDurableQueueRequest(queue_id=QUEUE_ID, filter_groups=FILTERS), metadata=md)
    for resp in stub.ConsumeDurableEventsWithAcks(requests(), metadata=md):
        work.put((resp.delivery_tag, list(resp.events)))

threading.Thread(target=worker, daemon=True).start()
while True:
    try:
        run_once()
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.NOT_FOUND:
            reconcile_from_source()              # queue was reaped; create in run_once rebuilds it
        time.sleep(backoff_with_jitter())        # UNAUTHENTICATED: refresh token; INTERNAL/UNAVAILABLE/RESOURCE_EXHAUSTED: reconnect
```

This is a sketch: in production give each stream its own work and settle queues, because tags from a previous stream are meaningless on a new one (the server skips them, and those messages are redelivered and processed again).

Settlement semantics (from the proto):

- Acks are processed before nacks; a tag in both is acknowledged. Unknown or already-settled tags are skipped and the stream stays open.
- A nack always requeues — there is no reject-and-drop — and redelivery is immediate, possibly on the same stream.
- Every redelivery (nack, disconnect, broker channel close) increments a delivery count that is never reset. Past the deployment's `deliveryLimit` (10 as shipped) the message is dead-lettered. A tight nack loop spends the whole budget in under a second.
- Delivery tags are only meaningful on the stream that issued them. After reconnecting, unsettled messages are redelivered with new tags.
- The broker delivers up to 20 unacked messages per consumer (`durable_consumer_prefetch_count`). If the operator sets a consumer timeout, a delivery left unacked longer than it gets the consumer cancelled; the stream ends with `INTERNAL`. Settle promptly.

## REST and the deprecated RPC

`GET /api/v1beta/events/stream-durable?queue_id=<id>` (SSE, frames `connected` / `event` / `error`) and gRPC `ConsumeDurableEvents` both have the server acknowledge each message after a short delay. A disconnect inside that window loses messages, and you cannot nack. Use them only where that loss is acceptable; prefer `ConsumeDurableEventsWithAcks`. The REST `POST /api/v1beta/events/ack-durable` endpoint was removed in Consumer 1.10.0.

## Update and delete

```bash
curl -sS -X PATCH "https://event-consumer.<domain>/api/v1beta/queues/durable" \
  -H "Authorization: Bearer $SERVICE_TOKEN" -H "Content-Type: application/json" \
  -d '{"queue_id":"myapp-renders","current_filter_groups":[...],"new_filter_groups":[...]}'
curl -sS -X DELETE "https://event-consumer.<domain>/api/v1beta/queues/durable?queue_id=myapp-renders" \
  -H "Authorization: Bearer $SERVICE_TOKEN"
```

Both return `{"success": true}` on REST; the gRPC `UpdateDurableQueueResponse` and `DeleteDurableQueueResponse` are empty messages (the source package showed a `success` field on gRPC — it does not exist). Update changes bindings without losing buffered messages; delete-and-recreate does lose them. A repeated delete returns `404` / `NOT_FOUND`, which means the queue is gone — the success outcome when retrying a delete. Update and delete are safe to retry on a lone `503`; create is safe to retry only with your own `queue_id`.

## Retention limits you must design for

| Bound | Default (operator-tunable via `policyHook.policy.definition`) | What happens |
|---|---|---|
| Idle expiry | 24 h with no attached consumer (`expiresMs`) | Queue and its messages deleted, **not** dead-lettered; consume returns `NOT_FOUND`. Publishing to it or re-creating it does not keep it alive |
| Message TTL | 24 h (`messageTtlMs`) | Dead-lettered (`REASON_EXPIRED`) |
| Max length | 100000, `drop-head` (`maxLength`, `overflow`) | Oldest dead-lettered (`REASON_MAXLEN`) |
| Delivery limit | 10 (`deliveryLimit`) | Dead-lettered (`REASON_DELIVERY_LIMIT`) |
| DLQ | 14 days, 1,000,000 messages, `drop-head` | Then permanently dropped |

The consuming RPCs do not expose these numbers; ask the operator for the values in force. A consumer that runs less often than the idle window (a weekly batch job) should either get a longer `expiresMs` from the operator or recreate on `NOT_FOUND` and reconcile. Dead letters are triaged by operators via `ManageDeadLetteredEvents` ([operate → dead-letter triage](../../operate/notifications.md#dead-letter-triage)); a replayed message returns to your queue and is redelivered to you, possibly as a duplicate.
