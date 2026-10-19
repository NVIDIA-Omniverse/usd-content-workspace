# Consuming Events

This guide covers how to consume events from the Notifications Service using both the REST API and gRPC API, including detailed information about durable and non-durable queues.

## Overview

The Event Consumer Service allows clients to stream events in real-time. You can filter events by event type and resource ID, and choose between durable and non-durable consumption modes.

## Durable vs Non-Durable Queues

One of the most important decisions when consuming events is whether to use a **durable queue** or a **non-durable queue**. Understanding the difference is critical for building reliable applications.

### Non-Durable Queues

**What they are:**
- Temporary queues created automatically when you start consuming
- Only receive events that occur **while you are connected**
- Automatically deleted after disconnection (after a brief TTL)
- No setup required - just start streaming

**When to use:**
- Real-time notifications in UI applications
- Live dashboards and monitoring displays
- Event logging where missing some events is acceptable
- Prototyping and development
- Scenarios where events are only relevant "right now"

**Advantages:**
- Simple - no queue management needed
- Lightweight - no persistent storage
- Automatic cleanup
- Good for applications with many concurrent users (each gets their own temporary queue)

**Disadvantages:**
- **Events occurring while disconnected are lost**
- Not suitable for critical event processing

**Example use cases:**
```
✓ Showing "Someone just uploaded a file" notifications in a web UI
✓ Live activity feed in a dashboard
✓ Real-time log streaming during development
✗ Processing every file upload for thumbnail generation (use durable)
✗ Triggering critical workflows (use durable)
✗ Syncing data between services (use durable)
```

### Durable Queues

**What they are:**
- Persistent queues that must be explicitly created before use
- Store events even when you're disconnected
- Receive **all events from the time the queue was created**
- Should be explicitly deleted when no longer needed
- Deleted automatically by the service if left without a consumer for the deployment's idle window (24 hours by default) — see [Durable Queue Lifetime](#durable-queue-lifetime)

**When to use:**
- Critical event processing where no events can be missed
- Microservices that need to process every event
- Batch processing workflows
- Systems that may have downtime for deployments
- Event-driven architectures where events trigger actions

**Advantages:**
- **No events are lost** - all events since queue creation are preserved
- Can disconnect and reconnect without losing messages
- Events are queued up during downtime
- Reliable for building event-driven systems

**Disadvantages:**
- Requires explicit queue lifecycle management (create/delete)
- Consumes storage resources
- Need to persist **both the queue_id and the filter_groups** in your application configuration
- Must remember to delete queues when done
- Not consuming for longer than the idle window deletes the queue, and your client must recreate it
- A client-chosen `queue_id` must be unique across the whole deployment, must match `^[A-Za-z0-9._-]{1,200}$`, and any principal holding a durable-queue permission can reach a queue whose id it knows

**Example use cases:**
```
✓ Thumbnail generation service processing every file upload
✓ Notification delivery service sending emails for events
✓ Data synchronization between services
✓ Audit logging where every event must be recorded
✗ UI notifications (non-durable is simpler)
✗ Temporary development testing (non-durable is easier)
```

### Decision Matrix

| Scenario | Recommended Choice | Reason |
|----------|-------------------|--------|
| UI real-time notifications | **Non-durable** | Users only care about what's happening now |
| File processing pipeline | **Durable** | Must process every file |
| Live dashboard | **Non-durable** | Current state is what matters |
| Email notification service | **Durable** | Must send every notification |
| Development/testing | **Non-durable** | Easier, no cleanup needed |
| Production microservice | **Durable** | Can't miss events during restarts |
| Webhook relay | **Durable** | Must deliver all webhooks |
| Activity feed | **Non-durable** | Only recent activity matters |

### Key Principle

**If missing an event would break your application or cause data inconsistency, use a durable queue. If events are only relevant "in the moment," use a non-durable queue.**

## Consuming with Non-Durable Queues

### REST API

**Endpoint**: `GET /api/v1beta/events/stream`

The REST API uses Server-Sent Events (SSE) for streaming.

**Basic Example (No Filtering)**:

```bash
# Consume all events of all types
curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream" \
  -H "Authorization: Bearer YOUR_TOKEN"
```

**With Event Type Filtering**:

```bash
# Create filter groups as JSON
FILTERS='[
  {
    "event_type": "storage.file.created",
    "filters": []
  }
]'

# URL encode and pass as query parameter
curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -G --data-urlencode "filter_groups=$FILTERS"
```

**With Resource Filtering**:

```bash
# Only receive events for files in /uploads/ directory and subdirectories
FILTERS='[
  {
    "event_type": "storage.file.created",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "/uploads/"
      }
    ]
  }
]'

curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -G --data-urlencode "filter_groups=$FILTERS"
```

**Response Format**:

```
event: connected
id: my-queue-id
data: {"message":"Connection established","reconnect_token":"my-queue-id"}

event: event
data: {"event_type":"storage.file.created","principal_identity":"user@example.com","occurred_at":"2024-10-16T14:30:00Z","published_at":"2024-10-16T14:30:01Z","message":{"file_name":"doc.pdf"}}

event: event
data: {"event_type":"storage.file.created","principal_identity":"user2@example.com","occurred_at":"2024-10-16T14:31:00Z","published_at":"2024-10-16T14:31:01Z","message":{"file_name":"image.png"}}
```

The initial `connected` event includes the `reconnect_token` (which is the queue ID). Save this token for reconnection.

Presenting the token reattaches to the queue it names, which is what lets you reconnect after a drop and share one queue across parallel clients; consumers on the same queue split its events between them. When several clients share a queue this way, at most one of them may change its filters, and that client alone can follow the acknowledgement-driven replay recipe described under **Updating Filters Dynamically**.

The token identifies a queue. It is not a secret and it grants no access on its own: presenting it requires a valid bearer token, and every event delivered to you is authorized against your own permissions, so you receive only what you are entitled to receive whichever queue you attach to. What the token does is name the queue you join — and because consumers on one queue split its events, keeping the token inside the application that received it is what ensures only your own authorized consumers share, and split, that queue's events.

Send the token in the `Last-Event-ID` request header; the `reconnect_token` query parameter is deprecated in favour of it but is still accepted and carries the token the same way. The header carries every operation the query parameter does, including a filter update, so there is nothing you need the query parameter for. Prefer the header: a query parameter travels in the URL, and URLs are recorded by proxies, gateways, and browser history along the way, while a header is not.

A token you send back must be 1 to 200 printable ASCII characters — no spaces, no control characters, nothing non-ASCII — in the header and in the query parameter alike; anything else is refused with `422 Unprocessable Entity` (`INVALID_ARGUMENT` on gRPC), naming the field and the rule and quoting none of the token. The tokens this service issues are random identifiers well inside that rule, so echoing back what you were given always passes it.

**Python Example with SSE Client**:

```python
import sseclient
import requests
import json

url = "https://your-consumer-service.example.com/api/v1beta/events/stream"
headers = {
    "Authorization": f"Bearer {YOUR_TOKEN}",
    "Accept": "text/event-stream"
}

# Define filters
filters = [
    {
        "event_type": "storage.file.created",
        "filters": [
            {
                "filter_type": "starts_with_greedy",
                "resource_id": "/uploads/"
            }
        ]
    }
]

params = {"filter_groups": json.dumps(filters)}

# Stream events
response = requests.get(url, headers=headers, params=params, stream=True)
client = sseclient.SSEClient(response)

reconnect_token = None

for event in client.events():
    if event.event == "connected":
        connected_data = json.loads(event.data)
        reconnect_token = connected_data['reconnect_token']
        print(f"Connected, reconnect token: {reconnect_token}")
    elif event.event == "event":
        data = json.loads(event.data)
        print(f"Received: {data['event_type']}")
        print(f"Message: {data['message']}")
```

**Reconnecting After Disconnection**:

If your connection drops, use the reconnect token to reconnect to the same queue and avoid missing events. Send back the token the server issued, verbatim, in the `Last-Event-ID` header:

```bash
curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Last-Event-ID: abc123xyz"
```

A browser's built-in SSE reconnect sends this header for you, using the `id:` field of the last event it received.

**Updating Filters Dynamically (REST)**:

You can update filters on a non-durable queue while it continues to exist. This is useful when you need to change what events you're listening for without losing the queue. To update filters, make a new request carrying the token, the new `filter_groups`, and the `previous_filter_groups`.

Sending `previous_filter_groups` is what makes a request a filter update, whichever carrier the token arrives in — the `Last-Event-ID` header and the deprecated `reconnect_token` query parameter both work. A request that carries the token alone is a plain reconnect and resumes the stream on the filters it already had.

A browser's built-in SSE reconnect arrives at the same result by the header path. It replays the URL your stream was opened on and sends the token in the `Last-Event-ID` header; for a stream opened as a fresh connect that URL carries the original `filter_groups` and no `previous_filter_groups` — so the automatic reconnect resumes the stream as it was rather than being read as an incomplete filter update. If instead you opened the `EventSource` on a filter-update URL, the browser replays `previous_filter_groups` too and each automatic reconnect re-runs that same update, which is harmless: applying the same before-and-after pair to a queue already in that state changes nothing.

```bash
# Original filters we were using
PREV_FILTERS='[
  {
    "event_type": "storage.file.created",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "/uploads/"
      }
    ]
  }
]'

# New filters we want to use
NEW_FILTERS='[
  {
    "event_type": "storage.file.created",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "/uploads/"
      }
    ]
  },
  {
    "event_type": "storage.file.deleted",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "/uploads/"
      }
    ]
  }
]'

# Reconnect with updated filters
curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Last-Event-ID: abc123xyz" \
  -G --data-urlencode "filter_groups=$NEW_FILTERS" \
  --data-urlencode "previous_filter_groups=$PREV_FILTERS"
```

**Python Example with Filter Updates**:

```python
import sseclient
import requests
import json

def consume_with_filter_updates():
    url = "https://your-consumer-service.example.com/api/v1beta/events/stream"
    headers = {
        "Authorization": f"Bearer {YOUR_TOKEN}",
        "Accept": "text/event-stream"
    }
    
    # Initial filters
    current_filters = [
        {
            "event_type": "storage.file.created",
            "filters": [
                {"filter_type": "starts_with_greedy", "resource_id": "/uploads/"}
            ]
        }
    ]
    
    params = {"filter_groups": json.dumps(current_filters)}
    reconnect_token = None
    
    # Start consuming
    response = requests.get(url, headers=headers, params=params, stream=True)
    client = sseclient.SSEClient(response)
    
    for event in client.events():
        if event.event == "connected":
            connected_data = json.loads(event.data)
            reconnect_token = connected_data['reconnect_token']
        elif event.event == "event":
            data = json.loads(event.data)
            process_event(data)
            
            # Example: Update filters after some condition
            if should_update_filters():
                # Close current connection
                response.close()
                
                # Define new filters
                new_filters = [
                    {
                        "event_type": "storage.file.created",
                        "filters": [
                            {"filter_type": "starts_with_greedy", 
                             "resource_id": "/uploads/"}
                        ]
                    },
                    {
                        "event_type": "storage.file.deleted",
                        "filters": [
                            {"filter_type": "starts_with_greedy", 
                             "resource_id": "/uploads/"}
                        ]
                    }
                ]
                
                # Reconnect with updated filters. The token goes in the
                # Last-Event-ID header; sending previous_filter_groups is
                # what makes this an update rather than a plain reconnect.
                params = {
                    "filter_groups": json.dumps(new_filters),
                    "previous_filter_groups": json.dumps(current_filters)
                }
                reconnect_headers = {**headers,
                                     "Last-Event-ID": reconnect_token}
                
                response = requests.get(url, headers=reconnect_headers,
                                       params=params, stream=True)
                client = sseclient.SSEClient(response)
                
                # Update our tracking of current filters
                current_filters = new_filters
```

**Important Notes on Updating Filters (REST)**:

- When updating filters, you **must** provide both `filter_groups` (new filters) and `previous_filter_groups` (old filters). Providing `previous_filter_groups` is what marks the request as an update
- The reconnect token identifies which queue to update. Send it in the `Last-Event-ID` header (or in the deprecated `reconnect_token` query parameter); either carrier performs the update
- When the reconnect token is sent in the `Last-Event-ID` header without `previous_filter_groups`, any `filter_groups` on the request are ignored and the stream resumes on the filters it already had. Include `previous_filter_groups` to update the filters. That tolerance is specific to the header, which a browser's automatic reconnect replays: a token sent in the deprecated `reconnect_token` query parameter alongside `filter_groups` but without `previous_filter_groups` is an incomplete filter update and is rejected with `400`, naming the missing field
- The queue must already exist (you cannot update filters before making an initial consume request)
- If `filter_groups` equals `previous_filter_groups`, no update is performed
- There is no separate acknowledgement on this surface, and none is needed: a REST filter update is applied before the stream's `connected` event is sent, so the arrival of that event confirms the update. The gRPC surface, where updates are sent mid-stream, acknowledges each one it processes

### gRPC API

**RPC**: `ConsumeNonDurableEvents` (bidirectional streaming)

**Python Example**:

```python
import grpc
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-consumer-service.example.com:50052',
    grpc.ssl_channel_credentials()
)
stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)

# Define filters
filter_group = event_consumer_pb2.FilterGroup(
    event_type='storage.file.created'
)
resource_filter = event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
)
filter_group.filters.append(resource_filter)

# Create request stream
def request_stream():
    # Send initial request with filters
    request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group)
    yield request
    # Keep stream open (or send updated filters later)

# Start consuming
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]
responses = stub.ConsumeNonDurableEvents(request_stream(), metadata=metadata)

reconnect_token = None
server_acks_filter_updates = None  # Unknown until the first response arrives
confirmed_filters = None  # The set the server last acknowledged

try:
    for response in responses:
        # Save reconnect token for potential reconnection
        if response.reconnect_token:
            reconnect_token = response.reconnect_token

        if server_acks_filter_updates is None:
            # True on every response of a stream served by a server that
            # acknowledges the filter sets its requests name. It describes
            # the server, not this response, so reading it off the first
            # response is enough -- and it is what picks your recovery path.
            server_acks_filter_updates = response.supports_filter_update_acks
            if server_acks_filter_updates:
                print("Filter updates are acknowledged: after a drop, "
                      "reconnect with the token and replay every update "
                      "that was not acknowledged")
            else:
                print("Filter updates are not acknowledged: after a drop, "
                      "open a fresh stream with filter_groups and no "
                      "reconnect_token")

        # A response carrying filter_update_ack reports that the server acted
        # on the filter set named by one of your requests; it carries no
        # events. The first response carries one when the opening request
        # named a set, as this fresh connect does. applied_filter_groups is a
        # verbatim copy of that request's filter_groups, and unchanged is
        # True when its filter_groups equalled its previous_filter_groups as
        # you sent them, so the server issued no bind or unbind call.
        if response.HasField('filter_update_ack'):
            # Name this set in previous_filter_groups on your next update,
            # and treat every update you sent after it as outstanding until
            # its own acknowledgement arrives.
            confirmed_filters = list(
                response.filter_update_ack.applied_filter_groups
            )
            print(f"Filter update acknowledged: {confirmed_filters} "
                  f"(unchanged={response.filter_update_ack.unchanged})")
            continue

        # Process events
        for event in response.events:
            print(f"Event type: {event.event_type}")
            print(f"Occurred at: {event.occurred_at}")
            print(f"Message: {event.message}")
            print(f"Published by: {event.principal_identity}")
            print("---")
            
except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.UNAUTHENTICATED:
        print("Token expired - reconnect with new token")
        # Reconnect with new token and reconnect_token
```

**Reconnecting with Token**:

A reconnect that carries only the token resumes the queue with the bindings it already holds, and the first response carries no `filter_update_ack` — the server keeps no filter state and cannot report a set it was not told. A reconnect may instead carry the token together with both `filter_groups` and `previous_filter_groups`, which is a filter update applied before the stream yields anything and acknowledged on that first response. A reconnect that names `filter_groups` without `previous_filter_groups` is neither: it is an incomplete filter update and is rejected with `INVALID_ARGUMENT`. A client that has changed filters since the set the server last acknowledged replays the outstanding updates after reconnecting; see **Updating Filters Dynamically** below.

```python
def request_stream_reconnect(reconnect_token):
    request = event_consumer_pb2.ConsumeNonDurableEventsRequest(
        reconnect_token=reconnect_token
    )
    yield request

# Reconnect to same queue
responses = stub.ConsumeNonDurableEvents(
    request_stream_reconnect(reconnect_token), 
    metadata=new_metadata
)
```

**Updating Filters Dynamically**:

The bidirectional stream allows you to update filters while consuming. When sending an update request, you must include both the new filters in `filter_groups` and the previous filters in `previous_filter_groups`.

A server that sets `supports_filter_update_acks` sends one response carrying a `filter_update_ack` for each update it processes. Receiving it means the server is done with that request: any bind and unbind calls the request led to have returned — it does not report how many calls that was, and the answer may be none. Track the updates you send and mark each one confirmed when its acknowledgement arrives: that is what lets you resume this stream with the `reconnect_token` after a drop, by re-sending in order each update you sent but did not see acknowledged, each naming in `previous_filter_groups` the set it replaced.

```python
import threading
import queue

# Track current filters so we can send them as previous_filter_groups
current_filters = []

def request_stream_dynamic(filter_queue, initial_filters):
    global current_filters
    
    # Send initial filters
    initial_request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
    for fg in initial_filters:
        initial_request.filter_groups.append(fg)
    current_filters = initial_filters
    yield initial_request
    
    # Wait for filter updates
    while True:
        new_filters = filter_queue.get()  # Blocks until new filters available
        if new_filters is None:  # Sentinel to stop
            break
            
        # Create update request with both new and previous filters
        update_request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
        for fg in new_filters:
            update_request.filter_groups.append(fg)
        for fg in current_filters:
            update_request.previous_filter_groups.append(fg)
        
        # Update our tracking of current filters
        current_filters = new_filters
        yield update_request

# Initial filters
initial_filter_group = event_consumer_pb2.FilterGroup(
    event_type='storage.file.created'
)
resource_filter = event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
)
initial_filter_group.filters.append(resource_filter)
initial_filters = [initial_filter_group]

filter_queue = queue.Queue()
responses = stub.ConsumeNonDurableEvents(
    request_stream_dynamic(filter_queue, initial_filters), 
    metadata=metadata
)

# Later, update filters to add a new event type
new_filter_group_1 = event_consumer_pb2.FilterGroup(
    event_type='storage.file.created'
)
new_filter_group_1.filters.append(event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
))

new_filter_group_2 = event_consumer_pb2.FilterGroup(
    event_type='storage.file.deleted'
)
new_filter_group_2.filters.append(event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
))

# Send updated filters - the request_stream_dynamic function will
# automatically include the previous filters
filter_queue.put([new_filter_group_1, new_filter_group_2])

# Reading the acknowledgements on the response side. Each acknowledgement
# names the update it confirms by echoing that request's filter_groups, so a
# client knows which of the updates it sent the server has processed.
for response in responses:
    if response.HasField('filter_update_ack'):
        ack = response.filter_update_ack
        # unchanged is True when the acknowledged request's filter_groups
        # equalled its previous_filter_groups as you sent them, so the server
        # issued no bind or unbind call.
        print(f"Applied: {list(ack.applied_filter_groups)} "
              f"unchanged={ack.unchanged}")
        continue

    for event in response.events:
        print(f"Event type: {event.event_type}")
```

**Important Notes on Updating Filters (gRPC)**:

- When updating filters, you **must** provide both `filter_groups` (new filters) and `previous_filter_groups` (old filters)
- If `filter_groups` equals `previous_filter_groups` as you sent them, the server issues no bind or unbind call, and the request is acknowledged with `unchanged` set to `true`
- The server uses `previous_filter_groups` to determine which bindings to remove when updating the queue
- `applied_filter_groups` is a verbatim copy of the acknowledged request's `filter_groups`: the server holds no filter state and cannot read the queue's bindings back from the broker, so it echoes what you sent rather than reporting what the broker holds
- `unchanged` describes the request the server received rather than the queue's state. It does not confirm that the queue's bindings match `applied_filter_groups`, and a `false` value does not promise that any bind or unbind call was issued — a request that re-sends the same groups reordered, or with a group repeated, differs as sent and still changes no bindings
- The first response on the stream carries an acknowledgement when the opening request named a filter set the server acted on: a fresh connect, acknowledged with `unchanged` `false`, and a reconnect naming `filter_groups` and `previous_filter_groups` together, acknowledged with `unchanged` set according to whether the two were equal as sent. A reconnect carrying only the token names no set and is not acknowledged, and a reconnect naming `filter_groups` without `previous_filter_groups` is rejected with `INVALID_ARGUMENT`
- A mid-stream request that names neither `filter_groups` nor `previous_filter_groups` is ignored and never acknowledged; a rejected update ends the stream with `INVALID_ARGUMENT` and is not acknowledged either. Nor is an update still in flight when the stream ends with a terminal error, as it does when the broker cancels the consumer (see [Handle Disconnections Gracefully](#2-handle-disconnections-gracefully)): the stream ends and the update is never acknowledged
- Acknowledgements are ordered with respect to one another, in the order the updates were sent, but an acknowledgement and the event responses around it are interleaved in no guaranteed order
- Replaying more than one unacknowledged update after a reconnect walks the queue back through each intermediate filter set in turn, and applying a set unbinds every resource that set drops, so an event published for a resource that an earlier replayed update unbinds is lost until a later replayed update binds it again
- Exactly one client per queue may change filters. Other clients sharing the `reconnect_token` may consume from the queue, but must not send filter updates: each client computes `previous_filter_groups` from acknowledgements that describe only its own updates, so two clients updating one queue walk its bindings to a set neither asked for
- Against a server that does not set `supports_filter_update_acks` there is no safe way to resume a changed filter set by token; open a fresh stream with `filter_groups` and no `reconnect_token` instead

## Consuming with Durable Queues

Durable queues require a three-step process: create, consume, delete.

### Step 1: Create a Durable Queue

This is typically done **once** during application setup or deployment, not in your regular application code.

#### REST API

**Endpoint**: `POST /api/v1beta/queues/durable`

The request body MAY include an optional `queue_id` field. When you omit it, the server generates a `queue_id` for you. When you supply it, the server creates the durable queue with your client-specified ID.

Queue creation is ensure-exists (idempotent): if you supply a `queue_id` that already exists, the call succeeds against the existing queue rather than returning an error. The server signals which path the request took through the HTTP status code:

- `201 Created` — the request created a new durable queue.
- `200 OK` — the request resolved to a queue that already existed; nothing was created.

Branch on the status code to tell the two apart. In both cases the response body returns the `queue_id`.

**Pick an id that is unique across the whole deployment.** Queue ids share a single namespace, so ensure-exists attaches you to an existing queue with your id no matter who created it or which `filter_groups` you send — you get that queue with its existing filters and `created: false`. Two callers that pick the same id share one queue: each receives the other's events, and either can delete it. Use a UUID, or a namespaced convention such as `<service-name>-<purpose>` (`thumbnailer-uploads`, say). Never use a bare generic word like `events`.

**Per-queue isolation is not enforced.** The three durable-queue permissions are deployment-wide rather than per-queue: a principal granted `consume-durable-queues` can consume from any durable queue whose id it knows, one granted `delete-durable-queues` can delete any of them, and `create-durable-queues` authorizes both creating a queue and updating the filters of any existing one. A `queue_id` is configuration, not a secret that protects a queue — ask your operator to grant these permissions to the service accounts that consume events rather than to broad user groups.

**Server-generated `queue_id`** (omit `queue_id` from the body):

```bash
curl -X POST "https://your-consumer-service.example.com/api/v1beta/queues/durable" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "filter_groups": [
      {
        "event_type": "storage.file.created",
        "filters": [
          {
            "filter_type": "starts_with_greedy",
            "resource_id": "/uploads/"
          }
        ]
      }
    ]
  }'
```

**Client-specified `queue_id`** (include `queue_id` in the body):

```bash
curl -i -X POST "https://your-consumer-service.example.com/api/v1beta/queues/durable" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "queue_id": "thumbnail-service",
    "filter_groups": [
      {
        "event_type": "storage.file.created",
        "filters": [
          {
            "filter_type": "starts_with_greedy",
            "resource_id": "/uploads/"
          }
        ]
      }
    ]
  }'
```

The `-i` flag prints the status line so you can see whether the queue was created (`201 Created`) or already existed (`200 OK`). A second identical call with the same `queue_id` succeeds and returns `200 OK`.

**Response**:

```json
{
  "queue_id": "durable-queue-abc123xyz",
  "created": true
}
```

**Important**: Save the `queue_id` **and** the `filter_groups` you created it with, together as one piece of configuration. You need the `queue_id` to consume and delete, and you need both to recreate the queue if it is auto-deleted after going unconsumed for the deployment's idle window — its subscriptions are rebuilt from the filters. See [Durable Queue Lifetime](#durable-queue-lifetime).

#### gRPC API

**RPC**: `CreateDurableQueue`

```python
import grpc
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-consumer-service.example.com:50052',
    grpc.ssl_channel_credentials()
)
stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)

# Define filters
filter_group = event_consumer_pb2.FilterGroup(
    event_type='storage.file.created'
)
resource_filter = event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
)
filter_group.filters.append(resource_filter)

# Create durable queue
request = event_consumer_pb2.CreateDurableQueueRequest()
request.filter_groups.append(filter_group)
# Optionally supply a client-specified queue_id. Omit it to have the server
# generate one. Creation is ensure-exists (idempotent): supplying a queue_id
# that already exists succeeds against the existing queue instead of erroring —
# whoever created it, and whatever filter_groups this call carries. The id must
# therefore be unique across the deployment: use a UUID or a namespaced
# <service-name>-<purpose> form, never a bare generic word.
request.queue_id = 'thumbnail-service'
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

response = stub.CreateDurableQueue(request, metadata=metadata)
queue_id = response.queue_id

# The created field mirrors the REST 201/200 distinction: it is True when this
# call created a new queue and False when an existing queue was returned unchanged.
if response.created:
    print(f"Created durable queue: {queue_id}")
else:
    print(f"Durable queue already existed: {queue_id}")
# IMPORTANT: Save this queue_id AND the filter_groups you created it with to
# your application config — you need both to recreate the queue if it is
# auto-deleted after going unconsumed for the idle window. See "Durable Queue
# Lifetime" (#durable-queue-lifetime).
```

### Step 2: Consume from Durable Queue

Now consume events using the queue_id. Your application can disconnect and reconnect using the same queue_id.

Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client. Pick ids as described under [Step 1](#step-1-create-a-durable-queue).

#### REST API

**Endpoint**: `GET /api/v1beta/events/stream-durable`

```bash
curl -N GET "https://your-consumer-service.example.com/api/v1beta/events/stream-durable?queue_id=durable-queue-abc123xyz" \
  -H "Authorization: Bearer YOUR_TOKEN"
```

**Python Example**:

Messages are acknowledged to the broker automatically by the server after a short delay. Clients do not need to acknowledge individual messages, and there is no way to ask for a message to be delivered again on this endpoint. If your application requires explicit client-side acknowledgment (e.g. to guarantee at-least-once processing), or needs to hand a message it failed to process back for another attempt, use the gRPC `ConsumeDurableEventsWithAcks` RPC instead.

```python
import sseclient
import requests
import json

url = "https://your-consumer-service.example.com/api/v1beta/events/stream-durable"
headers = {
    "Authorization": f"Bearer {YOUR_TOKEN}",
    "Accept": "text/event-stream"
}
params = {"queue_id": queue_id}

response = requests.get(url, headers=headers, params=params, stream=True)
client = sseclient.SSEClient(response)

for event in client.events():
    if event.event == "event":
        data = json.loads(event.data)
        process_event(data)
```

**Reconnecting**: Just use the same queue_id — no reconnect token needed:

```python
# After disconnection or application restart
response = requests.get(url, headers=headers, params=params, stream=True)
# Will continue from where you left off
```

#### gRPC API

There are two gRPC RPCs for consuming from durable queues:

- **`ConsumeDurableEventsWithAcks`** (recommended) — bidirectional streaming with explicit client acknowledgment. You settle each message yourself: acknowledge it once you have processed it, or negatively acknowledge (nack) it to have it delivered again. A message you neither acknowledge nor nack is redelivered when you reconnect. Redelivery is bounded, so this is at-least-once delivery up to a retry budget: a message that exhausts that budget (or the queue's configured message TTL or maximum length) is moved to a dead-letter queue rather than redelivered indefinitely. See [Bounded Redelivery and the Dead-Letter Queue](#bounded-redelivery-and-the-dead-letter-queue) below.
- **`ConsumeDurableEvents`** (deprecated) — server streaming with automatic server-side acknowledgment after a short delay. Has a window for message loss on disconnect.

**Using ConsumeDurableEventsWithAcks (recommended)**:

```python
import grpc
import queue
import threading
import time
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-consumer-service.example.com:50052',
    grpc.ssl_channel_credentials()
)
stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]


class TransientProcessingError(Exception):
    """Raised by your handler when the failure is worth another attempt."""


work_queue = queue.Queue()    # (delivery_tag, events) handed off by the stream
settle_queue = queue.Queue()  # (ack_tags, nack_tags) sent back to the service

RETRY_DELAY_SECONDS = 5  # grow this on repeated failures; stay inside the budget


def worker():
    """Process events and settle them — off the response loop."""
    while True:
        item = work_queue.get()
        if item is None:
            break
        delivery_tag, events = item
        try:
            for event in events:
                process_event(event)  # your handler; may raise
        except TransientProcessingError:
            # Wait BEFORE handing the message back: redelivery is immediate,
            # so nacking without a delay retries at full speed. The wait runs
            # here, on the worker, and never on the response loop.
            time.sleep(RETRY_DELAY_SECONDS)
            settle_queue.put(([], [delivery_tag]))
        else:
            settle_queue.put(([delivery_tag], []))


def request_stream(queue_id):
    # First request: identify the durable queue
    yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
        queue_id=queue_id
    )
    # Subsequent requests: settle previously delivered messages
    while True:
        settlement = settle_queue.get()
        if settlement is None:
            break
        ack_tags, nack_tags = settlement
        yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
            ack_delivery_tags=ack_tags,
            nack_delivery_tags=nack_tags,
        )


threading.Thread(target=worker, daemon=True).start()

try:
    responses = stub.ConsumeDurableEventsWithAcks(
        request_stream(queue_id), metadata=metadata
    )

    # The response loop only hands work off; it never processes and never waits.
    for response in responses:
        work_queue.put((response.delivery_tag, list(response.events)))

except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.UNAUTHENTICATED:
        # Refresh token and reconnect with same queue_id
        print("Token expired, reconnecting...")
```

The retry delay is the client's responsibility, and it belongs on the worker: the
service redelivers a nacked message immediately, and a `time.sleep` on the
response loop would stall every other delivery on the stream while one message
waits. See [Don't Block the Event Stream](#4-dont-block-the-event-stream).

**Asking for a message to be delivered again**

Send the message's `delivery_tag` in `nack_delivery_tags` instead of `ack_delivery_tags` when your handler cannot process it right now — a downstream service is unavailable, a dependency is rate-limiting you, a resource is temporarily locked. The message goes back on the queue and is delivered again.

Three properties to design around:

- **The retry is immediate**, and it may arrive on the stream you nacked from — often within milliseconds. The service applies no delay of its own, so **back off before you nack**. Without a delay you are retrying in a tight loop.
- **Nacking always returns the message to the queue.** There is no "reject and drop" — acknowledging is how you drop a message you do not want.
- **Each nack spends part of a fixed retry budget** that is shared with every other kind of redelivery. See [Bounded Redelivery and the Dead-Letter Queue](#bounded-redelivery-and-the-dead-letter-queue) below; it is the most important thing to understand before you nack in production.

Sending the same tag in both `ack_delivery_tags` and `nack_delivery_tags` on one request acknowledges it — the acknowledgment wins. Sending a tag the service does not recognize (one you already settled, or one from a previous connection) is skipped and leaves your stream running.

**Using ConsumeDurableEvents (deprecated)**:

```python
# Consume from durable queue (auto-ack, deprecated)
request = event_consumer_pb2.ConsumeDurableEventsRequest(
    queue_id=queue_id  # Use the saved queue_id
)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

try:
    responses = stub.ConsumeDurableEvents(request, metadata=metadata)
    
    for response in responses:
        for event in response.events:
            print(f"Event type: {event.event_type}")
            print(f"Message: {event.message}")
            # Process event
            
except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.UNAUTHENTICATED:
        # Refresh token and reconnect with same queue_id
        print("Token expired, reconnecting...")
```

### Durable Queue Lifetime

A durable queue ends in one of two ways: you delete it with `DeleteDurableQueue`, or the service deletes it after it has gone without an attached consumer for the deployment's **idle window** — 24 hours by default, configurable by the operator. When a queue is deleted this way, any messages it still holds are discarded; they are not moved to the dead-letter queue.

This is consistent with the message TTL that already applies to durable queues: an absent consumer's messages age out after the same window anyway, so what the idle window removes is the empty queue itself.

**What keeps a queue alive**

Only an **attached consumer** does. The idle clock starts when your consumer disconnects or cancels.

| Action | Keeps the queue alive? |
|--------|------------------------|
| Consuming (`ConsumeDurableEvents` / `ConsumeDurableEventsWithAcks`) | **Yes** — for as long as the stream is attached |
| Events being published to the queue | No — a queue nobody consumes from is idle even while filling up |
| Calling `CreateDurableQueue` again on an existing queue | No — re-declaring does not renew the lease |
| Deleting and recreating the queue | Yes, but you lose the buffered messages |

A client that consumes continuously, or that reconnects within the window, is never affected.

**Recovering from an auto-deleted queue**

Consuming from a queue that was deleted fails loudly rather than returning an empty stream — `404` on REST, `NOT_FOUND` on gRPC. Recover by recreating the queue with the same ID and resuming:

```python
try:
    for response in stub.ConsumeDurableEvents(request, metadata=metadata):
        handle(response)
except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.NOT_FOUND:
        # The queue was deleted after going unconsumed for the idle window.
        # Recreate it with the SAME id and the SAME filters, then resume.
        stub.CreateDurableQueue(
            CreateDurableQueueRequest(
                queue_id=saved_queue_id,
                filter_groups=saved_filter_groups,
            ),
            metadata=metadata,
        )
        # Retry the consume with saved_queue_id.
```

Because `CreateDurableQueue` is ensure-exists, the same call is safe whether or not the queue still exists — it recreates a missing queue and is a no-op against a surviving one. Events published while the queue was gone are not recoverable.

**Persist your filters, not just your queue ID**

The queue's event subscriptions are rebuilt from `filter_groups` when the queue is created. An application that stored only its `queue_id` cannot fully recover a deleted queue — it would recreate the queue with whatever filters happen to be in the code path at that moment. Store the `queue_id` and its `filter_groups` together as one piece of configuration.

**If your consumer is idle by design**

A consumer that legitimately runs on a schedule longer than the idle window — a weekly batch job, say — should either ask the operator to raise the idle window for that deployment, or simply follow the recreate-on-`NOT_FOUND` path above at the start of each run.

### Bounded Redelivery and the Dead-Letter Queue

When you consume from a durable queue with `ConsumeDurableEventsWithAcks`, a message you do not acknowledge comes back to be processed again. There are three ways that happens: you nack it, your connection drops before you settle it, or the broker takes your stream's consumer away and returns your unsettled messages to the queue.

Redelivery is bounded. Each message carries a **delivery count**, and the service delivers a message only until that count passes the deployment's configured limit. This gives you at-least-once delivery up to a retry budget, not redelivery forever.

> **The retry budget belongs to the message, not to the attempt.** The delivery count is cumulative over the message's entire life and is never reset — not by reconnecting, not by a successful delivery that you later nack, not by starting a new stream. Nacks, disconnects, and broker-side requeues all draw down the same budget. A nack is redelivered immediately, so a handler that nacks in a tight loop while a downstream service is down can spend a message's whole budget in **under a second** and dead-letter work that was only briefly unprocessable. Always back off between attempts.

A message leaves the durable queue and is moved to a dead-letter queue (DLQ) when any of the following happens:

- Its delivery count passes the configured limit (for example, a consistently failing "poison" message that is never successfully acknowledged).
- It outlives the queue's configured message TTL.
- It is evicted when the queue reaches its configured maximum length.

The practical implications for a consumer are:

- **A poison message will not block the stream indefinitely.** A message your handler can never process is redelivered only a finite number of times and is then dead-lettered, so it stops being redelivered and the rest of the stream continues to flow.
- **At-least-once is qualified.** Settle each message promptly. A message that is never successfully acknowledged within its budget is dead-lettered rather than redelivered again, so design your handlers to make forward progress within it.
- **Nack deliberately, and with backoff.** Nacking is the right response to a *transient* failure, and the wrong response to a permanent one — a message your handler will never accept is better acknowledged and recorded on your side than retried until the deployment dead-letters it. When you do nack, delay first: the budget is small, and retrying at full speed is how you exhaust it.
- **The durable queue is bounded.** A slow or stalled consumer cannot grow the queue without limit — once the queue reaches its configured maximum length, the oldest overflow messages are dead-lettered rather than retained indefinitely.

The concrete redelivery limit, message TTL, and maximum-length values are configured by the deployment and are not surfaced to consumers through the consuming RPCs; ask your operator for the limit in force if you are tuning a retry strategy against it. Operators with the appropriate permission can inspect, replay, discard, or leave dead-lettered messages through a dedicated management RPC — see [Managing the Dead-Letter Queue (Operators)](#managing-the-dead-letter-queue-operators) below.

### Managing the Dead-Letter Queue (Operators)

The sections above describe how messages *arrive* in the dead-letter queue (DLQ). This section describes how an operator inspects and triages them.

**Who this is for**: This is an operator-facing capability, not a per-consumer one. It is gated by the `event-consumer-service:inspect-and-replay-all-dlq` permission. A caller that lacks this permission is denied with `PERMISSION_DENIED`.

**gRPC-only**: Dead-letter management is exposed only over gRPC via the `ManageDeadLetteredEvents` RPC. There is no REST/SSE equivalent.

#### The browse-and-act model

`ManageDeadLetteredEvents` is a bidirectional streaming RPC. The session works as a live browse-and-act loop:

1. The client opens the stream with an initial (empty) request.
2. The server streams dead-lettered messages to the client, one per `dead_lettered_event` response, each carrying triage metadata.
3. The client replies with per-message actions — `ACTION_REPLAY`, `ACTION_DISCARD`, or `ACTION_LEAVE` — keyed by the message's session-scoped `delivery_tag`.
4. For each action the operator sends, the server streams back an `action_outcome` response reporting what happened, correlated by the same `delivery_tag`.

The dead-letter queue is global rather than scoped to a single durable queue, so a single management session surfaces dead-lettered messages from across the deployment.

**No datastore required.** This bidirectional browse-and-act stream is the permanent baseline DLQ-management surface. It works against the message broker alone and requires no additional datastore. It is the forward-compatible floor for dead-letter management: future additive unary store-backed RPCs (for example, listing dead-lettered events with a server-side cursor, filtering them, replaying a specific event by id, or reporting aggregate statistics) may be added later without changing this RPC. Those additive capabilities are not available today — do not depend on them.

#### Triage metadata

Each `dead_lettered_event` carries metadata to support triage:

- **`reason`** — why the message was dead-lettered: `REASON_DELIVERY_LIMIT` (exceeded its redelivery limit), `REASON_EXPIRED` (outlived the queue's configured message TTL), or `REASON_MAXLEN` (evicted when the queue reached its configured maximum length).
- **`origin_queue`** — the name of the durable queue the message was dead-lettered from.
- **`delivery_count`** — the number of times the broker attempted delivery before dead-lettering.
- **`dead_lettered_at`** — the timestamp when the message was dead-lettered.
- **`event`** — the dead-lettered event itself, carrying its `event_type`, `principal_identity`, and `message` payload.
- **`resource_id`** — the resource id associated with the dead-lettered event.
- **`delivery_tag`** — the session-scoped tag used to act on this message (see the caveat below).

#### Actions: ACTION_REPLAY, ACTION_DISCARD, ACTION_LEAVE

The operator replies with an action per `delivery_tag`:

- **`ACTION_REPLAY`** — republishes the message to its origin queue. Replay is confirm-gated and no-loss: the dead-letter copy is removed only after the publish to the origin queue is confirmed. A replay that cannot be confirmed never drops the message — the dead-letter copy is left in place and the server reports an `OUTCOME_REPLAY_FAILED` outcome.
- **`ACTION_DISCARD`** — acknowledges the message and drops it from the dead-letter queue.
- **`ACTION_LEAVE`** — leaves the message in place in the dead-letter queue for later triage.

#### Action-outcome reports

The server-to-client stream carries two kinds of message, modeled as a response `oneof payload` with two arms: a `dead_lettered_event` (a browse item, described above) and an `action_outcome` (a report). Every action the operator sends produces exactly one correlated `action_outcome`, keyed by `delivery_tag`, so the operator can observe the result of every action rather than relying on server-side logs.

An `action_outcome` carries:

- **`delivery_tag`** — the session-scoped tag identifying which dead-lettered message the outcome pertains to (matches the tag the operator sent).
- **`outcome`** — one of `OUTCOME_REPLAYED`, `OUTCOME_DISCARDED`, `OUTCOME_LEFT`, `OUTCOME_ORIGIN_QUEUE_DELETED`, or `OUTCOME_REPLAY_FAILED`.
- **`detail`** — a human-readable note with additional context (for example, the name of the missing origin queue for an `OUTCOME_ORIGIN_QUEUE_DELETED` outcome).

The `outcome` values are:

- **`OUTCOME_REPLAYED`** — the message was successfully republished to its origin queue and the dead-letter copy was removed after the publish confirm.
- **`OUTCOME_DISCARDED`** — the message was acknowledged and dropped from the dead-letter queue.
- **`OUTCOME_LEFT`** — the message was left in place in the dead-letter queue for later triage.
- **`OUTCOME_ORIGIN_QUEUE_DELETED`** — an `ACTION_REPLAY` could not proceed because the origin queue no longer exists (see below). This outcome is no-loss.
- **`OUTCOME_REPLAY_FAILED`** — an `ACTION_REPLAY` could not be confirmed. This outcome is no-loss: the dead-letter copy is left in place exactly as for an `ACTION_LEAVE`, so the message is never dropped on a failed replay.

**Origin-queue-deleted handling.** A `ACTION_REPLAY` of a message whose origin queue has been deleted does **not** lose the message. The replay cannot proceed because there is no viable target, so the server leaves the dead-letter copy in place (exactly the `ACTION_LEAVE` semantics) and reports an `OUTCOME_ORIGIN_QUEUE_DELETED` outcome on the stream — the condition is reported to the operator, not silently swallowed. To obtain a viable replay target, re-create the origin queue and replay the message again.

**Scope.** `ACTION_REPLAY` returns a message to the exact queue it dead-lettered from. The target comes from the message's own broker metadata rather than from the request, so a replay cannot be redirected to another queue. The `event-consumer-service:inspect-and-replay-all-dlq` permission is coarse: it grants deployment-wide dead-letter visibility and action by design; finer-grained scoping is deferred. When a replayed message is subsequently re-consumed, it still runs the existing per-event authorization check.

**Session delivery tags are live-only.** A `delivery_tag` is valid only within the live stream that produced it. Tags are not persisted and must not be reused across reconnects. If the stream drops, re-open a new management session; the server re-presents the still-dead-lettered messages with fresh tags.

#### gRPC Python Example

```python
import grpc
import queue
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-consumer-service.example.com:50052',
    grpc.ssl_channel_credentials()
)
stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)
# The caller's token must carry the
# event-consumer-service:inspect-and-replay-all-dlq permission.
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

action_queue = queue.Queue()

def request_stream():
    # First request opens the stream and carries no actions.
    yield event_consumer_pb2.ManageDeadLetteredEventsRequest()
    # Subsequent requests carry per-delivery-tag actions.
    while True:
        entries = action_queue.get()
        if entries is None:
            break
        request = event_consumer_pb2.ManageDeadLetteredEventsRequest()
        request.actions.extend(entries)
        yield request

responses = stub.ManageDeadLetteredEvents(request_stream(), metadata=metadata)

Request = event_consumer_pb2.ManageDeadLetteredEventsRequest
Response = event_consumer_pb2.ManageDeadLetteredEventsResponse

for response in responses:
    # The response oneof has two arms: dead_lettered_event and action_outcome.
    arm = response.WhichOneof('payload')

    if arm == 'dead_lettered_event':
        dle = response.dead_lettered_event
        print(f"Dead-lettered: tag={dle.delivery_tag} "
              f"reason={Response.DeadLetteredEvent.Reason.Name(dle.reason)} "
              f"origin_queue={dle.origin_queue} "
              f"event_type={dle.event.event_type} "
              f"resource_id={dle.resource_id}")

        # Decide what to do and reply, keyed by the session delivery tag.
        entry = Request.ActionEntry(
            delivery_tag=dle.delivery_tag,
            action=Request.ACTION_REPLAY,
        )
        action_queue.put([entry])

    elif arm == 'action_outcome':
        outcome = response.action_outcome
        name = Response.ActionOutcome.Outcome.Name(outcome.outcome)
        print(f"Outcome for tag={outcome.delivery_tag}: {name} "
              f"({outcome.detail})")

        if outcome.outcome == Response.ActionOutcome.Outcome.OUTCOME_ORIGIN_QUEUE_DELETED:
            # The message was NOT lost: the dead-letter copy is left in place.
            # Re-create the origin queue, then replay again.
            print("Origin queue is gone; re-create it before replaying.")
```

### Step 3: Delete Durable Queue

When your application no longer needs the queue (e.g., during decommissioning), delete it to free resources.

Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client. Pick ids as described under [Step 1](#step-1-create-a-durable-queue).

#### REST API

**Endpoint**: `DELETE /api/v1beta/queues/durable`

```bash
curl -X DELETE "https://your-consumer-service.example.com/api/v1beta/queues/durable?queue_id=durable-queue-abc123xyz" \
  -H "Authorization: Bearer YOUR_TOKEN"
```

**Response**:

```json
{
  "success": true
}
```

#### gRPC API

**RPC**: `DeleteDurableQueue`

```python
request = event_consumer_pb2.DeleteDurableQueueRequest(
    queue_id=queue_id
)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

response = stub.DeleteDurableQueue(request, metadata=metadata)
print("Queue deleted successfully")
```

### Updating Durable Queue Filters

You can update filters on an existing durable queue without deleting it.

Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client. Pick ids as described under [Step 1](#step-1-create-a-durable-queue).

#### REST API

**Endpoint**: `PATCH /api/v1beta/queues/durable`

```bash
curl -X PATCH "https://your-consumer-service.example.com/api/v1beta/queues/durable" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "queue_id": "durable-queue-abc123xyz",
    "current_filter_groups": [
      {
        "event_type": "storage.file.created",
        "filters": []
      }
    ],
    "new_filter_groups": [
      {
        "event_type": "storage.file.created",
        "filters": []
      },
      {
        "event_type": "storage.file.deleted",
        "filters": []
      }
    ]
  }'
```

#### gRPC API

**RPC**: `UpdateDurableQueue`

```python
request = event_consumer_pb2.UpdateDurableQueueRequest(
    queue_id=queue_id
)
for fg in current_filters:
    request.current_filter_groups.append(fg)
for fg in new_filters:
    request.new_filter_groups.append(fg)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

response = stub.UpdateDurableQueue(request, metadata=metadata)
```

## Resource Filtering

### Filter Types

Three types of resource filters are available:

#### 1. EQ (Exact Match)

Matches only exact resource_id:

```python
{
    "filter_type": "eq",
    "resource_id": "/projects/project1/file.txt"
}

# Matches: /projects/project1/file.txt
# No match: /projects/project1/file2.txt
# No match: /projects/project1/sub/file.txt
```

#### 2. STARTS_WITH_LAZY (Shallow Prefix Match)

Matches resources at the same level or one level deep:

```python
{
        "filter_type": "starts_with_lazy",
    "resource_id": "/projects/project1/"
}

# Matches: /projects/project1/file.txt
# No match: /projects/project1/subfolder/file.txt (too deep)
# No match: /projects/project10/file.txt (not a prefix match)
```

#### 3. STARTS_WITH_GREEDY (Deep Prefix Match)

Matches resources at any depth under the prefix:

```python
{
            "filter_type": "starts_with_greedy",
    "resource_id": "/projects/project1/"
}

# Matches: /projects/project1/file.txt
# Matches: /projects/project1/subfolder/file.txt
# Matches: /projects/project1/a/b/c/d/file.txt
# No match: /projects/project10/file.txt
```

### Empty Resource ID

Special behavior for `resource_id=""`:

```python
# STARTS_WITH_GREEDY with empty resource_id: receive ALL events of this type
{
            "filter_type": "starts_with_greedy",
    "resource_id": ""
}

# STARTS_WITH_LAZY with empty resource_id: receive events with no resource 
# or single-level resources
{
        "filter_type": "starts_with_lazy",
    "resource_id": ""
}

# EQ with empty resource_id: ERROR (not allowed)
{
    "filter_type": "eq",
    "resource_id": ""
}
```

### Multiple Filters (OR Logic)

Multiple filters within a FilterGroup use OR logic:

```python
# Receive events from /uploads/ OR /shared/
{
    "event_type": "storage.file.created",
    "filters": [
        {
            "filter_type": "starts_with_greedy",
            "resource_id": "/uploads/"
        },
        {
            "filter_type": "starts_with_greedy",
            "resource_id": "/shared/"
        }
    ]
}
```

### Multiple Event Types

Multiple FilterGroups let you subscribe to multiple event types:

```python
[
    {
        "event_type": "storage.file.created",
        "filters": [
            {"filter_type": "starts_with_greedy", "resource_id": "/uploads/"}
        ]
    },
    {
        "event_type": "storage.file.deleted",
        "filters": [
            {"filter_type": "starts_with_greedy", "resource_id": "/uploads/"}
        ]
    }
]
```

## Error Handling

### Authorization Errors During Streaming

**Important**: Authorization errors can occur **at any time** during streaming, not just at connection time.

For example:
- Your authentication token expires while streaming
- Permissions change while you're connected

When this happens, the stream will terminate with a 401 (UNAUTHENTICATED) or 403 (PERMISSION_DENIED) error.

**Handling in REST/SSE**:

```python
import time

def consume_with_auto_reconnect(url, get_token_func, queue_id=None):
    """
    Consume events with automatic reconnection on auth errors.
    Messages are acknowledged automatically by the server.
    """
    while True:
        try:
            token = get_token_func()  # Get fresh token
            headers = {
                "Authorization": f"Bearer {token}",
                "Accept": "text/event-stream"
            }
            
            params = {}
            if queue_id:
                params["queue_id"] = queue_id
            
            response = requests.get(url, headers=headers, params=params, stream=True)
            
            if response.status_code == 401 or response.status_code == 403:
                print("Auth error, refreshing token...")
                time.sleep(1)
                continue
            
            client = sseclient.SSEClient(response)

            for event in client.events():
                if event.event == "event":
                    data = json.loads(event.data)
                    process_event(data)
                    
        except requests.exceptions.RequestException as e:
            print(f"Connection error: {e}, reconnecting...")
            time.sleep(5)
```

**Handling in gRPC**:

```python
import queue

def consume_with_retry(stub, queue_id, get_token_func):
    """
    Consume with automatic reconnection on auth errors.
    Uses ConsumeDurableEventsWithAcks for explicit acknowledgment.
    """
    while True:
        try:
            token = get_token_func()
            metadata = [('authorization', f'Bearer {token}')]
            ack_queue = queue.Queue()

            def request_stream():
                yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
                    queue_id=queue_id
                )
                while True:
                    tags = ack_queue.get()
                    if tags is None:
                        break
                    yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
                        ack_delivery_tags=tags
                    )

            responses = stub.ConsumeDurableEventsWithAcks(
                request_stream(), metadata=metadata
            )
            
            for response in responses:
                for event in response.events:
                    process_event(event)
                ack_queue.put([response.delivery_tag])
                    
        except grpc.RpcError as e:
            if e.code() in [grpc.StatusCode.UNAUTHENTICATED, 
                          grpc.StatusCode.PERMISSION_DENIED]:
                print("Auth error, refreshing token and reconnecting...")
                time.sleep(1)
            elif e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                print("Service overloaded, backing off...")
                time.sleep(5)
            elif e.code() == grpc.StatusCode.UNAVAILABLE:
                print("Connection lost, reconnecting...")
                time.sleep(1)
            else:
                print(f"Error: {e.code()}: {e.details()}")
                time.sleep(5)
```

### Other Common Errors

| HTTP Status / gRPC Code | Meaning | Action |
|------------------------|---------|--------|
| 400 / INVALID_ARGUMENT | Invalid filter_groups | Check your request format |
| 422 / INVALID_ARGUMENT | A `queue_id` that does not match `^[A-Za-z0-9._-]{1,200}$`, or a reconnect token that is not 1 to 200 printable ASCII characters (no spaces, control characters, or non-ASCII), whether it arrives as the `reconnect_token` query parameter, in the `Last-Event-ID` header, or in the gRPC request field. Applies on every operation that accepts one, not just creation | Correct the value — the request is rejected before it reaches the broker. Send a reconnect token back exactly as the server issued it |
| 404 / NOT_FOUND | Queue not found (durable) — either the queue_id is wrong, or the queue was auto-deleted after going unconsumed for the idle window | Verify queue_id, then recreate the queue with the same id and filters — see [Durable Queue Lifetime](#durable-queue-lifetime) |
| 500 / INTERNAL — `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.` | The broker cancelled this stream's consumer, or killed the channel it lived on — commonly an acknowledgement timeout on a slow consumer, or a broker-side leader change. Applies to every consume stream: durable, non-durable, and dead-letter management | Reconnect and resume consuming; do **not** recreate the queue. The queue normally survives with its buffered messages, and messages that were in flight unacknowledged are redelivered after you reconnect. This error does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `NOT_FOUND` / `404`, and the recovery for a queue that is gone applies — see [Handle Disconnections Gracefully](#2-handle-disconnections-gracefully) |
| 503 / RESOURCE_EXHAUSTED | Service temporarily overloaded | Retry with exponential backoff |
| — / UNAVAILABLE | Connection lost (e.g. load balancer idle timeout) | Reconnect automatically |

## Best Practices

### 1. Choose the Right Queue Type

See the [decision matrix](#decision-matrix) above.

### 2. Handle Disconnections Gracefully

```python
# Non-durable: Save reconnect_token
reconnect_token = None
for response in responses:
    if response.reconnect_token:
        reconnect_token = response.reconnect_token

# Use it to reconnect
request = ConsumeNonDurableEventsRequest(reconnect_token=reconnect_token)

# Durable: Just use the same queue_id
request = ConsumeDurableEventsWithAcksRequest(queue_id=saved_queue_id)
```

A non-durable reconnect resumes the queue with the bindings it already holds, so
the token alone is enough only while your filter set still matches the last set
the server acknowledged. A gRPC client that changes filters mid-stream tracks
which of its updates were acknowledged and, after reconnecting with the token,
re-sends in order each update it sent but did not see acknowledged, each naming
in `previous_filter_groups` the set it replaced — see
[Updating Filters Dynamically](#grpc-api). If the server does not set
`supports_filter_update_acks`, open a fresh stream with `filter_groups` and no
`reconnect_token` instead of resuming.

**A stream also ends when the broker drops your consumer.** Every consume
stream — durable and non-durable, over gRPC (with or without client
acknowledgment) and over REST/SSE alike, and the dead-letter management
stream — is attached to its queue by a consumer on the
broker. The broker can take that consumer away underneath an otherwise healthy
connection: when the queue is deleted, and when it kills the channel the
consumer lives on (on durable queues, an acknowledgement timeout on a slow
consumer is the usual cause). The stream ends with an error when this happens,
so handle it: a client that ignores the error and keeps waiting on the stream
receives no further events. The check runs while the stream is idle, so the
error can arrive up to about 15 seconds after the loss on a default deployment.

Two errors are distinguished, because they call for opposite recoveries. How
each one reaches you depends on the protocol: on gRPC the stream terminates
with the status code shown; on REST/SSE the HTTP response status was already
`200 OK` when the stream opened, so the same condition arrives as an SSE event
named `error` whose `data` carries the `message` and the matching
`status_code`, and the stream then closes. An SSE client must therefore watch
for an event named `error`, not only for a broken connection.

| Error | Meaning | Action |
|-------|---------|--------|
| `500` / `INTERNAL`<br>`Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.` | The consumer attached to the queue was lost; the queue normally survives | **Reconnect — do not recreate the queue.** Consume again with the same `queue_id` (durable) or `reconnect_token` (non-durable). If that reconnect returns `404` / `NOT_FOUND`, follow the row below. |
| `404` / `NOT_FOUND` | The queue itself no longer exists | **Recreate, then resume.** Durable: call `CreateDurableQueue` with the same `queue_id` **and** `filter_groups`. Non-durable: see below |

Reconnecting after `INTERNAL` costs you nothing: the queue normally survives
with everything buffered in it, and **messages that were in flight
unacknowledged are redelivered once you reconnect**. `INTERNAL` does not by
itself confirm that the queue survived: if it no longer exists, the reconnect
returns `NOT_FOUND`, and the row above applies. Process events
idempotently so those redeliveries are safe — see [Process Events
Idempotently](#3-process-events-idempotently).

`NOT_FOUND` has one extra nuance for non-durable consumers. A non-durable queue
outlives its consumer only for the deployment's brief reconnect-grace window
(20 seconds by default). Within that window your `reconnect_token` still
resolves and you resume with the buffered backlog intact. Once the window
lapses the queue is gone and reconnecting with the token returns `NOT_FOUND`
again — **start a fresh stream with your `filter_groups` and no
`reconnect_token`**, which creates a new queue. Events published while the
queue was gone are not recoverable.

```python
try:
    for response in responses:
        handle(response)
except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.INTERNAL:
        # The consumer was lost; the queue normally survives — reconnect.
        # If the reconnect returns NOT_FOUND, take the NOT_FOUND branch below.
        # Unacknowledged messages are redelivered.
        reconnect()
    elif e.code() == grpc.StatusCode.NOT_FOUND:
        # The queue is gone. Durable: recreate it with the saved queue_id
        # AND filter_groups. Non-durable: start a fresh stream from
        # filter_groups, dropping the reconnect_token.
        recreate_then_reconnect()
```

The REST/SSE equivalent reads the same two outcomes off the `error` event:

```python
for event in client.events():
    if event.event == "event":
        handle(json.loads(event.data))
    elif event.event == "error":
        details = json.loads(event.data)
        if details["status_code"] == 500:
            # The consumer was lost; the queue normally survives — reconnect.
            # If the reconnected stream reports 404, take the 404 branch below.
            reconnect()
        elif details["status_code"] == 404:
            # The queue is gone. Durable: recreate it with the saved queue_id
            # AND filter_groups. Non-durable: start a fresh stream from
            # filter_groups, dropping the reconnect_token.
            recreate_then_reconnect()
        break  # the stream closes after an error event either way
```

For durable queues, the `NOT_FOUND` path is the same one you take when a queue
is auto-deleted after going without a consumer for longer than the idle window.
See [Durable Queue Lifetime](#durable-queue-lifetime).

### 3. Process Events Idempotently

Events may be delivered more than once — for durable queues, a message you nack or leave unsettled is redelivered until its delivery count passes the configured limit, after which it is dead-lettered. Design your event handlers to be idempotent so a redelivered message is safe to process again:

```python
def process_event(event):
    event_id = get_event_id(event)
    
    # Check if already processed
    if redis.exists(f"processed:{event_id}"):
        print(f"Event {event_id} already processed, skipping")
        return
    
    # Process event
    do_work(event)
    
    # Mark as processed
    redis.set(f"processed:{event_id}", "1", ex=86400)  # 24h TTL
```

### 4. Don't Block the Event Stream

Process events quickly or hand them off to a worker:

```python
import threading
import queue

event_queue = queue.Queue()

def event_processor():
    """Worker thread to process events."""
    while True:
        event = event_queue.get()
        if event is None:
            break
        process_event_slowly(event)

# Start worker
worker = threading.Thread(target=event_processor)
worker.start()

# Consume events quickly
for response in responses:
    for event in response.events:
        event_queue.put(event)  # Don't block the stream
```

### 5. Monitor Queue Depth (Durable Queues)

If you're not consuming fast enough, the durable queue will grow. The queue is bounded — it does not grow without limit. Once it reaches its configured maximum length, overflow messages are dead-lettered rather than retained (see [Bounded Redelivery and the Dead-Letter Queue](#bounded-redelivery-and-the-dead-letter-queue)), so a sustained backlog can lead to dropped (dead-lettered) messages. Monitor queue depth and scale your consumers before the backlog approaches the maximum length.

### 6. Delete Durable Queues When Done

Always delete durable queues when your application is decommissioned:

```python
# In your application shutdown handler
def cleanup():
    stub.DeleteDurableQueue(
        DeleteDurableQueueRequest(queue_id=queue_id),
        metadata=metadata
    )
```

This matters most for applications that roll their queue ID — for example, one
queue per deployed generation. An abandoned queue keeps accumulating and
dead-lettering events until the idle window deletes it, so deleting it yourself
on rollover is still the right thing to do; the idle window is a backstop, not a
substitute. See [Durable Queue Lifetime](#durable-queue-lifetime).

### 7. Use Appropriate Filters

Don't receive more events than you need:

```python
# Bad - receiving all events then filtering in code
bad_filter_group = FilterGroup(event_type="storage.file.created")
# No filters - receiving everything
for response in responses:
    for event in response.events:
        if event.message['file_name'].endswith('.pdf'):
            process(event)  # Wasting bandwidth

# Better - filter by resource if possible
better_filter_group = FilterGroup(
    event_type="storage.file.created"
)
resource_filter = ResourceFilter(
    filter_type=FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id="/pdfs/"  # Only get events from /pdfs/
)
better_filter_group.filters.append(resource_filter)
```

## Complete Example: Thumbnail Generation Service

Here's a complete example of a service that generates thumbnails for uploaded images using a durable queue with explicit acknowledgment:

```python
import grpc
import logging
import queue
import time
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

class ThumbnailService:
    def __init__(self, consumer_endpoint, auth_token):
        self.consumer_endpoint = consumer_endpoint
        self.auth_token = auth_token
        self.queue_id = None
        
    def setup(self):
        """Create durable queue - call once during deployment."""
        channel = grpc.secure_channel(
            self.consumer_endpoint,
            grpc.ssl_channel_credentials()
        )
        stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)
        
        # Create queue for image upload events
        filter_group = event_consumer_pb2.FilterGroup(
            event_type='storage.file.created'
        )
        resource_filter = event_consumer_pb2.ResourceFilter(
            filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
            resource_id='/images/'
        )
        filter_group.filters.append(resource_filter)
        
        request = event_consumer_pb2.CreateDurableQueueRequest()
        request.filter_groups.append(filter_group)
        metadata = [('authorization', f'Bearer {self.auth_token}')]
        
        response = stub.CreateDurableQueue(request, metadata=metadata)
        self.queue_id = response.queue_id
        
        logger.info(f"Created durable queue: {self.queue_id}")
        # SAVE THIS QUEUE_ID AND ITS FILTER_GROUPS TO YOUR CONFIG — you need
        # both to recreate the queue if it is auto-deleted after going
        # unconsumed for the idle window. See "Durable Queue Lifetime"
        # (#durable-queue-lifetime).

    def _request_stream(self, ack_queue):
        """Generate requests for the bidirectional stream."""
        # First request: identify the durable queue
        yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
            queue_id=self.queue_id
        )
        # Subsequent requests: send acknowledgments
        while True:
            tags = ack_queue.get()
            if tags is None:
                break
            yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
                ack_delivery_tags=tags
            )

    def run(self, queue_id):
        """Run the service - consumes events and generates thumbnails."""
        self.queue_id = queue_id
        
        while True:
            try:
                channel = grpc.secure_channel(
                    self.consumer_endpoint,
                    grpc.ssl_channel_credentials()
                )
                stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)
                metadata = [('authorization', f'Bearer {self.auth_token}')]
                ack_queue = queue.Queue()
                
                logger.info("Starting to consume events...")
                responses = stub.ConsumeDurableEventsWithAcks(
                    self._request_stream(ack_queue), metadata=metadata
                )
                
                for response in responses:
                    for event in response.events:
                        self.process_event(event)
                    # Acknowledge after processing
                    ack_queue.put([response.delivery_tag])
                        
            except grpc.RpcError as e:
                logger.error(f"gRPC error: {e.code()}: {e.details()}")
                if e.code() in [grpc.StatusCode.UNAUTHENTICATED, 
                              grpc.StatusCode.PERMISSION_DENIED]:
                    # Refresh token and reconnect
                    self.auth_token = self.get_fresh_token()
                    logger.info("Refreshed auth token, reconnecting...")
                else:
                    logger.error("Connection failed, retrying in 5s...")
                    time.sleep(5)
                    
    def process_event(self, event):
        """Process a single file created event."""
        try:
            file_path = event.message.get('file_path', '')
            logger.info(f"Processing file: {file_path}")
            
            # Generate thumbnail
            self.generate_thumbnail(file_path)
            
            logger.info(f"Thumbnail generated for: {file_path}")
            
        except Exception as e:
            logger.error(f"Error processing event: {e}")
            # Don't raise - continue processing other events
            
    def generate_thumbnail(self, file_path):
        """Generate thumbnail for the given file."""
        # Implementation here
        pass
        
    def get_fresh_token(self):
        """Get a fresh auth token."""
        # Implementation here
        pass

# Usage
if __name__ == "__main__":
    service = ThumbnailService(
        consumer_endpoint="your-consumer-service.example.com:50052",
        auth_token="YOUR_TOKEN"
    )
    
    # One-time setup (or load from config)
    # service.setup()
    
    # Run service with saved queue_id
    service.run(queue_id="saved-queue-id-from-config")
```

## Next Steps

- Understand [Permissions & Authorization](permissions.md) for consuming events
- Review the [API Reference](api-reference.md) for complete API documentation
- Learn about [Publishing Events](publishing-events.md)

