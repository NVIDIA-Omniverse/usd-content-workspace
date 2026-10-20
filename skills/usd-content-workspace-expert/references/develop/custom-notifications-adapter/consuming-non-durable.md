# Consuming with non-durable streams

> **Applies to:** Event Consumer Service `ConsumeNonDurableEvents` and `GET /api/v1beta/events/stream` · **Released:** 1.11.0 (contract `notifications-consumer-api:1.0.0-beta.3`) · **This repo's snapshot:** 1.12.0 (`notifications/reference/event-consumer-service/pyproject.toml`) · **Sources:** [`event_consumer.proto`](../../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto), [consumer `openapi.yaml`](../../../../../notifications/reference/event-consumer-service/openapi.yaml), [consumer `README.md`](../../../../../notifications/reference/event-consumer-service/README.md) (reconnect token, filter-update acks), [`docs/consuming-events.md`](../../../../../notifications/reference/event-consumer-service/docs/consuming-events.md), `app/rest/models/sse_events.py`

Use a non-durable stream when you only need events that happen while you are connected: live UI, dashboards, cache invalidation with a fallback refresh. Each stream gets its own queue. It survives a disconnect for the deployment's grace window (`rabbitmq.non_durable_queue_ttl_seconds`, default 20 s) so you can reconnect without gaps; after that it is gone.

## REST (Server-Sent Events)

```bash
FILTERS='[{"event_type":"omni.storage.created","filters":[{"filter_type":"starts_with_greedy","resource_id":"/projects/p1/"}]},
          {"event_type":"omni.storage.deleted","filters":[{"filter_type":"starts_with_greedy","resource_id":"/projects/p1/"}]}]'
curl -N -G "https://event-consumer.<domain>/api/v1beta/events/stream" \
  -H "Authorization: Bearer $CONSUMER_TOKEN" -H "Accept: text/event-stream" \
  --data-urlencode "filter_groups=$FILTERS"
```

Frames:

| `event:` | `id:` | `data:` |
|---|---|---|
| `connected` | the reconnect token | `{"message":"Connection established","reconnect_token":"<token>"}` |
| `event` | none | `{event_type, principal_identity, occurred_at, published_at, message}` |
| `error` | none | `{message, status_code, grpc_code}`; the stream then closes |

The HTTP status is `200` once the stream opens, so authorization expiry, queue loss, and broker consumer loss arrive as `error` frames. The source package's examples (`event: message`, `event: reconnect_token`) do not match these frames.

Python, using the `sseclient-py` package:

```python
import json, os, random, time, requests, sseclient

URL = "https://event-consumer.<domain>/api/v1beta/events/stream"
filters = [{"event_type": "omni.storage.created",
            "filters": [{"filter_type": "starts_with_greedy", "resource_id": "/projects/p1/"}]}]

def stream(token_for_reconnect=None):
    headers = {"Authorization": f"Bearer {os.environ['CONSUMER_TOKEN']}",
               "Accept": "text/event-stream"}
    params = {}
    if token_for_reconnect:
        headers["Last-Event-ID"] = token_for_reconnect      # resume on existing bindings
    else:
        params["filter_groups"] = json.dumps(filters)
    resp = requests.get(URL, headers=headers, params=params, stream=True, timeout=(10, None))
    resp.raise_for_status()
    return sseclient.SSEClient(resp)

token, backoff = None, 1
while True:
    try:
        for ev in stream(token).events():
            backoff = 1
            if ev.event == "connected":
                token = json.loads(ev.data)["reconnect_token"]
            elif ev.event == "event":
                handle(json.loads(ev.data))
            elif ev.event == "error":
                err = json.loads(ev.data)
                if err.get("status_code") == 404:           # queue gone: grace expired
                    token = None                             # fresh stream; refresh state from source
                    resync_from_source()
                break                                        # 401/403/500: reconnect (refresh token first on 401)
    except requests.RequestException:
        pass
    time.sleep(backoff + random.random()); backoff = min(backoff * 2, 30)
```

Rules:

- Send the token back in `Last-Event-ID`. The `reconnect_token` query parameter still works but is deprecated (URLs get logged). Tokens are 1–200 printable ASCII characters; send exactly what you received.
- A reconnect with a token and no `previous_filter_groups` resumes on the queue's existing bindings; with the header, any `filter_groups` on that request are ignored. With the deprecated query parameter, `filter_groups` without `previous_filter_groups` is rejected (`400`).
- To change filters, reconnect with the token **and** `filter_groups` (new) **and** `previous_filter_groups` (old). The `connected` frame on that response confirms the update was applied.
- A `404` (as status or `error` frame) means the queue is gone: open a fresh stream with `filter_groups`. Events published meanwhile are lost.
- A `500` "broker cancelled this stream's consumer" means reconnect with the same token; the queue normally survives.
- `503` means the pod is at capacity or draining; back off and reconnect (ingress may already have retried once or twice).
- Browsers' `EventSource` sends `Last-Event-ID` automatically on reconnect but cannot set an `Authorization` header; browser clients need a same-origin proxy or cookie-based edge auth that your deployment provides (not covered by this repo).

## gRPC (bidirectional)

The first request carries `filter_groups` (fresh) or `reconnect_token` (resume). Later requests on the same stream can update filters. Responses carry `events`, `reconnect_token`, and — from Consumer 1.11.0 — `supports_filter_update_acks` and, on acknowledgement responses only, `filter_update_ack`.

```python
import os, queue, grpc
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc as pb_grpc

stub = pb_grpc.EventConsumerServiceStub(
    grpc.secure_channel("grpc.event-consumer.<domain>:443", grpc.ssl_channel_credentials()))
md = [("authorization", f"Bearer {os.environ['CONSUMER_TOKEN']}")]

def fg(event_type, prefix):
    return pb.FilterGroup(event_type=event_type, filters=[pb.ResourceFilter(
        filter_type=pb.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY, resource_id=prefix)])

initial = [fg("omni.storage.created", "/projects/p1/")]
updates = queue.Queue()            # each item: (new_groups, previous_groups); None stops
# Every request that names a filter set is acknowledged once, in order -- including
# the fresh connect -- so track the opening request too.
outstanding = [(initial, [])]
confirmed = None

def requests_iter():
    yield pb.ConsumeNonDurableEventsRequest(filter_groups=initial)
    while (item := updates.get()) is not None:
        new, prev = item
        outstanding.append(item)
        yield pb.ConsumeNonDurableEventsRequest(filter_groups=new, previous_filter_groups=prev)

token = None
for resp in stub.ConsumeNonDurableEvents(requests_iter(), metadata=md):
    token = resp.reconnect_token or token
    if resp.HasField("filter_update_ack"):
        outstanding.pop(0)
        confirmed = list(resp.filter_update_ack.applied_filter_groups)
        continue
    for event in resp.events:
        handle(event)
```

After a drop: if the server advertised `supports_filter_update_acks`, reconnect with `reconnect_token=token` alone, then re-send each `outstanding` update in order, each naming the set it replaced. If it did not, open a fresh stream with your current `filter_groups` and no token. Only one client per queue may send filter updates. Replaying several outstanding updates is best-effort: intermediate sets unbind resources briefly.

Errors end the stream as gRPC statuses: `UNAUTHENTICATED` (refresh token, reconnect with token), `NOT_FOUND` (fresh stream), `INTERNAL` with the consumer-loss message (reconnect with token), `RESOURCE_EXHAUSTED` (back off), `INVALID_ARGUMENT` (bad filters, `EQ` on `""`, empty `event_type`, or `filter_groups` without `previous_filter_groups` on a reconnect).

## Sharing a queue and authorization

Several clients presenting the same token attach to one queue and **split** its events; that is for parallel workers, not fan-out. Each attached principal is authorized independently with its own token. The token is not a secret and grants nothing alone, but keep it inside your application so only your workers split your queue.

Storage events on non-durable streams are filtered by the caller's own Storage API permissions when the deployment enables `permissions.storage_permissions.enabled`: you receive a folder's events only if your token can list that folder. Other event types may be filtered by fine-grained policies. See [permissions](permissions.md).

**1.11.0 versus 1.12.0:** with storage permissions enabled, a 1.11.0 consumer could withhold events for folders the Storage API lists as empty (the first file in a new folder, the new folder's `dir_created`, the delete of a folder's last file). 1.12.0 delivers them. Clients that create folders and immediately watch them should not assume the 1.12.0 behavior on a 1.11.0 deployment.
