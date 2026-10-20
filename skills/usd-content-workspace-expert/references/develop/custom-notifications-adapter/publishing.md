# Publishing events

> **Applies to:** Event Aggregation Service publisher API (`nvidia.omniverse.notifications.publisher.v1beta`) · **Released:** 1.6.0 (contract `notifications-aggregation-api:1.0.0-beta.3`) · **This repo's snapshot:** 1.7.0 (`notifications/reference/event-aggregation-service/pyproject.toml`) · **Sources:** [`event_publisher.proto`](../../../../../notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto), [aggregation `openapi.yaml`](../../../../../notifications/reference/event-aggregation-service/openapi.yaml), [aggregation `README.md`](../../../../../notifications/reference/event-aggregation-service/README.md), [`docs/publishing-events.md`](../../../../../notifications/reference/event-consumer-service/docs/publishing-events.md)

## Design your event contract first

- Name types `service.entity.action` in lowercase, specific enough to filter on, at most 5 dot-separated parts and 256 characters (e.g. `myapp.render.completed`).
- Make `resource_id` a slash-separated path from broad to specific (`/projects/p1/shots/s010/frame.exr`), at most 2048 characters, consistent across your types. Consumers filter on its prefix.
- Keep `message` small: at most 65536 bytes as Python `json.dumps` encodes it (non-ASCII becomes `\uXXXX`; gRPC numbers become doubles). Reference large payloads by URL or storage path. This cap is enforced from Aggregation 1.7.0; a 1.6.0 deployment accepts larger messages, so do not rely on that.
- Set `occurred_at` to when the thing happened (UTC), not when you publish.
- Publish the contract (types, payload schema, resource scheme) for consumers, and request a `publish-event` grant for each type ([permissions](permissions.md)).
- Do not reuse `omni.storage.*` names unless you emit exactly those Storage events: consumers' storage permission handling assumes Storage semantics.

## REST

```bash
curl -sS -X POST "https://event-aggregation.<domain>/api/v1beta/events" \
  -H "Authorization: Bearer $PUBLISHER_TOKEN" -H "Content-Type: application/json" \
  -d '{"event":{"event_type":"myapp.render.completed",
               "message":{"job":"r-42","frames":120},
               "occurred_at":"2026-10-12T14:30:00Z",
               "resource":{"resource_id":"/projects/p1/shots/s010"}}}'
# -> {"result":{"event":{...},"success":true}}
```

Batch: `POST /api/v1beta/events/batch` with `{"events":[...]}` (1–1000) returns `{"results":[...]}` in request order.

## gRPC

```python
import os, time, random, grpc
from google.protobuf.struct_pb2 import Struct
from google.protobuf.timestamp_pb2 import Timestamp
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2 as pb
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc as pb_grpc

channel = grpc.secure_channel("grpc.event-aggregation.<domain>:443", grpc.ssl_channel_credentials())
stub = pb_grpc.EventPublishingServiceStub(channel)

def metadata():
    return [("authorization", f"Bearer {os.environ['PUBLISHER_TOKEN']}")]

def make_event(event_type, payload, resource_id):
    msg = Struct(); msg.update(payload)
    ts = Timestamp(); ts.GetCurrentTime()
    return pb.Event(event_type=event_type, message=msg, occurred_at=ts,
                    resource=pb.EventResource(resource_id=resource_id))

def publish_batch(events, attempts=5):
    pending = list(events)
    for attempt in range(attempts):
        resp = stub.BatchPublishEvents(pb.BatchPublishEventsRequest(events=pending),
                                       metadata=metadata(), timeout=30)
        pending = [r.event for r in resp.results if not r.success]
        if not pending:
            return
        time.sleep(min(30, 2 ** attempt) + random.random())   # retry only failed entries
    raise RuntimeError(f"{len(pending)} events not published")
```

Re-sending the `event` echoed in a failed `PublishingResult` retries exactly that entry. A lost response leaves the outcome unknown; consumers must tolerate duplicates.

## Results and errors

A broker-side failure is **not** an error status: the RPC returns `OK` / HTTP `200` with `success: false` and a `failure_reason` for the affected entries. Always inspect every result.

| You see | Meaning | Do |
|---|---|---|
| `413` (REST), connection closed | Body over `rest.maxRequestBytes` (4 MiB default); refused before auth | Split the batch; open a new connection |
| `RESOURCE_EXHAUSTED` (gRPC) | Request over the 4 MiB gRPC inbound cap. The only `RESOURCE_EXHAUSTED` this API returns | Split; retrying unchanged will not help |
| `401` / `UNAUTHENTICATED` | Token unresolvable, expired, or principal claim empty. Checked before request validation | Refresh the token |
| `400` / `INVALID_ARGUMENT` | Empty batch | Fix |
| `422` / `INVALID_ARGUMENT` naming a field (e.g. `events[1].message`) | Schema or limit violation; the whole batch is refused, nothing published | Fix or split |
| `403` / `PERMISSION_DENIED` | Missing `publish-event` on one of the batch's event types | Request the grant |
| `200` + `success: false` | Broker or channel failure for that entry | Retry that entry with backoff |
| `500` / `INTERNAL` | Server error, including IdP unreachable under `jwt` | Retry later |

The publisher never returns `503` for overload (removed from the spec in 1.6.0). With channel overflow disabled by the operator, saturation shows up as `success: false` after `channel_wait_ms`, not as a status code.

## Batching guidance

For events averaging more than about 4 KiB, the 4 MiB request bound binds before the 1000-event limit. Publish in batches sized by bytes, keep one batch per logical flush, and use bounded concurrency; the service publishes a batch's events to the broker in parallel. The Storage Service, for reference, sends `BatchPublishEvents` with up to `notificationClient.batchSize` (100) events.
