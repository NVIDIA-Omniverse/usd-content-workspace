# API Reference

This page provides detailed reference information for both the Event Aggregation Service (Publisher API) and Event Consumer Service (Consumer API).

## API Types

Both services provide dual APIs:

- **gRPC API**: Protocol Buffer-based, high-performance, strongly-typed
- **REST API**: HTTP/JSON-based, easier for testing and web clients

Use gRPC for production microservices and REST for web clients or when easier HTTP tooling is preferred.

## Event Aggregation Service (Publisher API)

The Event Aggregation Service accepts events from publishers and routes them to consumers.

### Base URLs

- **gRPC**: `your-aggregation-service.example.com:50051`
- **REST**: `https://your-aggregation-service.example.com`

### Authentication

Include a bearer token in all requests:

**gRPC**:
```python
metadata = [('authorization', 'Bearer YOUR_TOKEN')]
```

**REST**:
```
Authorization: Bearer YOUR_TOKEN
```

## Publisher gRPC API

### Service Definition

```protobuf
service EventPublishingService {
  rpc PublishEvent(PublishEventRequest) returns (PublishEventResponse);
  rpc BatchPublishEvents(BatchPublishEventsRequest) returns (BatchPublishEventsResponse);
}
```

### PublishEvent

Publishes a single event.

**Request**: `PublishEventRequest`

```protobuf
message PublishEventRequest {
  Event event = 1;
}

message Event {
  string event_type = 1;
  google.protobuf.Struct message = 2;
  google.protobuf.Timestamp occurred_at = 3;
  EventResource resource = 4;  // Optional
}

message EventResource {
  string resource_id = 1;
}
```

**Response**: `PublishEventResponse`

```protobuf
message PublishEventResponse {
  PublishingResult result = 1;
}

message PublishingResult {
  Event event = 1;
  bool success = 2;
  string failure_reason = 3;  // Only set if success is false
}
```

**Errors**:

| gRPC Status Code | Meaning |
|-----------------|---------|
| `UNAUTHENTICATED` | Invalid or expired authentication token |
| `PERMISSION_DENIED` | Not authorized to publish this event type |
| `INVALID_ARGUMENT` | Invalid event structure |
| `RESOURCE_EXHAUSTED` | Service temporarily overloaded - retry with backoff |
| `INTERNAL` | Server error |

**Example**:

```python
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2

# Create event
event = event_publisher_pb2.Event(
    event_type='myapp.user.created',
    message=message_struct,
    occurred_at=timestamp,
    resource=event_publisher_pb2.EventResource(
        resource_id='/users/123'
    )
)

# Publish
request = event_publisher_pb2.PublishEventRequest(event=event)
response = stub.PublishEvent(request, metadata=metadata)

if response.result.success:
    print("Published successfully")
```

### BatchPublishEvents

Publishes multiple events in parallel.

**Request**: `BatchPublishEventsRequest`

```protobuf
message BatchPublishEventsRequest {
  repeated Event events = 1;
}
```

**Response**: `BatchPublishEventsResponse`

```protobuf
message BatchPublishEventsResponse {
  repeated PublishingResult results = 1;
}
```

**Note**: Each event in the batch has its own result. Some may succeed while others fail.

**Example**:

```python
# Create multiple events
events = [event1, event2, event3]

# Batch publish
request = event_publisher_pb2.BatchPublishEventsRequest()
for event in events:
    request.events.append(event)
response = stub.BatchPublishEvents(request, metadata=metadata)

# Check individual results
for idx, result in enumerate(response.results):
    if result.success:
        print(f"Event {idx} published")
    else:
        print(f"Event {idx} failed: {result.failure_reason}")
```

## Publisher REST API

### Endpoints

#### POST /api/v1beta/events

Publishes a single event.

**Request Body**:

```json
{
  "event": {
    "event_type": "string",
    "message": {},
    "occurred_at": "2024-10-16T14:30:00Z",
    "resource": {
      "resource_id": "/path/to/resource"
    }
  }
}
```

**Response** (200 OK):

```json
{
  "result": {
    "event": { /* echo of event */ },
    "success": true
  }
}
```

**Response** (200 OK, but publish failed):

```json
{
  "result": {
    "event": { /* echo of event */ },
    "success": false,
    "failure_reason": "Unable to connect to message broker"
  }
}
```

**Error Responses**:

| Status Code | Description |
|------------|-------------|
| 400 | Invalid request format |
| 401 | Unauthenticated |
| 403 | Permission denied |
| 422 | Validation error |
| 503 | Service unavailable - retry |

**Example**:

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "event": {
      "event_type": "myapp.user.created",
      "message": {
        "user_id": "123",
        "username": "john_doe"
      },
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/users/123"
      }
    }
  }'
```

#### POST /api/v1beta/events/batch

Publishes multiple events in parallel.

**Request Body**:

```json
{
  "events": [
    {
      "event_type": "string",
      "message": {},
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/path/to/resource"
      }
    }
  ]
}
```

**Response** (200 OK):

```json
{
  "results": [
    {
      "event": { /* event 1 */ },
      "success": true
    },
    {
      "event": { /* event 2 */ },
      "success": false,
      "failure_reason": "Publish failed"
    }
  ]
}
```

**Example**:

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events/batch \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "events": [
      {
        "event_type": "myapp.file.uploaded",
        "message": {"file": "doc1.pdf"},
        "occurred_at": "2024-10-16T14:30:00Z"
      },
      {
        "event_type": "myapp.file.uploaded",
        "message": {"file": "doc2.pdf"},
        "occurred_at": "2024-10-16T14:30:01Z"
      }
    ]
  }'
```

## Event Consumer Service (Consumer API)

The Event Consumer Service allows clients to stream events with filtering.

### Base URLs

- **gRPC**: `your-consumer-service.example.com:50052`
- **REST**: `https://your-consumer-service.example.com`

## Consumer gRPC API

### Service Definition

```protobuf
service EventConsumerService {
  rpc ConsumeNonDurableEvents(stream ConsumeNonDurableEventsRequest) 
      returns (stream ConsumeNonDurableEventsResponse);
  
  // Deprecated: use ConsumeDurableEventsWithAcks instead.
  rpc ConsumeDurableEvents(ConsumeDurableEventsRequest) 
      returns (stream ConsumeDurableEventsResponse);
  
  rpc ConsumeDurableEventsWithAcks(stream ConsumeDurableEventsWithAcksRequest) 
      returns (stream ConsumeDurableEventsWithAcksResponse);
  
  rpc CreateDurableQueue(CreateDurableQueueRequest) 
      returns (CreateDurableQueueResponse);
  
  rpc UpdateDurableQueue(UpdateDurableQueueRequest) 
      returns (UpdateDurableQueueResponse);
  
  rpc DeleteDurableQueue(DeleteDurableQueueRequest) 
      returns (DeleteDurableQueueResponse);
  
  rpc ManageDeadLetteredEvents(stream ManageDeadLetteredEventsRequest) 
      returns (stream ManageDeadLetteredEventsResponse);
}
```

### ConsumeNonDurableEvents

Streams events using a non-durable queue (bidirectional streaming).

Reattaching to the queue it names is what lets you reconnect after a drop and share one queue across parallel clients. When several clients share a queue this way, at most one of them may change its filters, and that client alone can follow the acknowledgement-driven replay recipe described below.

Pass back a token the server issued, verbatim. A token must be 1 to 200 printable ASCII characters — no spaces, no control characters, nothing non-ASCII — and one that is not is rejected with `INVALID_ARGUMENT`, with a message that names the field and the rule and quotes none of the token. The tokens this service issues are random identifiers well inside that rule, so echoing back what you were given always passes it. A token of legal shape that no longer names a live queue is reported as `NOT_FOUND` instead — see below. Omit the field entirely to start a fresh stream.

The stream returns `INTERNAL`, with the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.`, when the broker cancels this stream's consumer or kills the channel it lives on — reconnect with the `reconnect_token`, and the queue normally survives with its buffered backlog. That resumes the queue with the bindings it already holds, so a gRPC client that has changed filters mid-stream also re-sends the updates the server did not acknowledge — see the `filter_update_ack` description under **Response** below. `INTERNAL` does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `NOT_FOUND`, and the recovery for a queue that is gone applies. It returns `NOT_FOUND` when the queue is gone: a non-durable queue outlives its consumer only for the deployment's reconnect-grace window (20 seconds by default), after which a `reconnect_token` reconnect returns `NOT_FOUND` again and you should start a fresh stream with `filter_groups` and no token. See [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully).

**Request**: `ConsumeNonDurableEventsRequest` (stream)

```protobuf
message ConsumeNonDurableEventsRequest {
  repeated FilterGroup filter_groups = 1;
  optional string reconnect_token = 2;
  repeated FilterGroup previous_filter_groups = 3;  // Required when updating filters
}

message FilterGroup {
  string event_type = 1;
  repeated ResourceFilter filters = 2;
}

message ResourceFilter {
  enum FilterType {
    FILTER_TYPE_UNSPECIFIED = 0;
    FILTER_TYPE_EQ = 1;
    FILTER_TYPE_STARTS_WITH_LAZY = 2;
    FILTER_TYPE_STARTS_WITH_GREEDY = 3;
  }
  
  FilterType filter_type = 1;
  string resource_id = 2;
}
```

**Response**: `ConsumeNonDurableEventsResponse` (stream)

```protobuf
message ConsumeNonDurableEventsResponse {
  repeated Event events = 1;
  string reconnect_token = 2;
  bool supports_filter_update_acks = 3;
  FilterUpdateAck filter_update_ack = 4;
}

message FilterUpdateAck {
  repeated FilterGroup applied_filter_groups = 1;
  bool unchanged = 2;
}

message Event {
  string event_type = 1;
  string principal_identity = 2;
  google.protobuf.Timestamp occurred_at = 3;
  google.protobuf.Timestamp published_at = 4;
  google.protobuf.Struct message = 5;
}
```

`supports_filter_update_acks` is `true` on every response of a stream served by a server that acknowledges the filter sets its requests name. It describes the server serving this stream rather than the individual response, so read it from any response you receive, including the first one as soon as the stream opens. A server that sets it sends one response carrying a `filter_update_ack` for every request it acts on that names a filter set, including a request that required no bind or unbind call because it named the set you had already asked for. A server that does not set it may still act on the filter sets its requests name, but never reports doing so, so you cannot know which of your requests took effect.

`filter_update_ack` is present on a response that acknowledges a request whose filter set the server acted on, and absent on every other response. Receiving it means the server is done with that request: any bind and unbind calls the request led to have returned. It does not report how many calls that was, and the answer may be none. Its two fields:

- `applied_filter_groups` — a verbatim copy of the `filter_groups` of the request being acknowledged. The server holds no filter state and cannot read a queue's bindings back from the broker, so this echoes what you sent rather than reporting what the broker holds.
- `unchanged` — `true` when the acknowledged request's `filter_groups` equalled its `previous_filter_groups` **as you sent them**, so the server issued no bind or unbind call. `false` means the two differed as sent, or that the request named no previous set at all, as a fresh connect does. This is a statement about the request the server received, not about the queue: it does not confirm that the queue's bindings match `applied_filter_groups`, and a `false` value does not promise that any bind or unbind call was issued — a request that re-sends the same groups reordered, or with a group repeated, differs as sent and still changes no bindings.

The first response on the stream carries an acknowledgement when the opening request named a filter set the server acted on. A fresh connect, which names `filter_groups` and no `reconnect_token`, is acknowledged with `unchanged` `false`. A reconnect that names `filter_groups` and `previous_filter_groups` together is a filter update applied before the stream yields anything, and is acknowledged with `unchanged` set according to whether the two sets were equal as you sent them. A reconnect carrying only a `reconnect_token` names no set and gets no acknowledgement, because the server keeps no filter state and cannot report a set it was not told. A reconnect that names `filter_groups` without `previous_filter_groups` is not a resume either: it is an incomplete filter update and is rejected with `INVALID_ARGUMENT`, so filter groups the server does not apply are never acknowledged as applied. Two kinds of mid-stream request are never acknowledged: one that names neither `filter_groups` nor `previous_filter_groups`, which is ignored, and one that is rejected, which ends the stream with `INVALID_ARGUMENT`. Nor is an update still in flight when the stream ends with a terminal error, as it does when the broker cancels the consumer (see the `INTERNAL` and `NOT_FOUND` cases above): the stream ends and the update is never acknowledged. Acknowledgements are interleaved with event responses in no guaranteed order, but are ordered with respect to one another, in the order the updates were sent. See [Updating Filters Dynamically](consuming-events.md#grpc-api) for the recipe that uses them to resume a stream whose filters changed.

**Example**:

```python
# Create filter
filter_group = event_consumer_pb2.FilterGroup(
    event_type='myapp.user.created'
)
# No filters added - receive all events of this type

# Request stream
def request_stream():
    request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group)
    yield request

# Consume
responses = stub.ConsumeNonDurableEvents(request_stream(), metadata=metadata)

for response in responses:
    reconnect_token = response.reconnect_token  # Save for reconnection
    
    for event in response.events:
        print(f"Event: {event.event_type}")
        print(f"Message: {event.message}")
```

### ConsumeDurableEvents (Deprecated)

> **Deprecated**: Use [`ConsumeDurableEventsWithAcks`](#consumedurableeventswithacks) instead. This RPC acknowledges messages automatically after a short delay, which has a window for message loss on disconnect. It will be removed in a future release.

Streams events from a durable queue (server streaming). Messages are automatically acknowledged by the server after a short delay.

This RPC returns `NOT_FOUND` when the queue no longer exists — either the `queue_id` is wrong, or the queue was auto-deleted after going without an attached consumer for the deployment's idle window (24 hours by default); recover by calling `CreateDurableQueue` again with the same `queue_id` and the same `filter_groups`, then resuming. See [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime).

The stream returns `INTERNAL`, with the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.`, when the broker cancels this stream's consumer or kills the channel it lives on. Reconnect and resume consuming; do not recreate the queue. The queue normally survives with its buffered messages, and messages that were in flight unacknowledged are redelivered once you reconnect. `INTERNAL` does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `NOT_FOUND`, and the recovery for a queue that is gone applies. See [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully).

**Request**: `ConsumeDurableEventsRequest`

```protobuf
message ConsumeDurableEventsRequest {
  string queue_id = 1;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `queue_id` | `string` | Required. The id returned by `CreateDurableQueue`. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `INVALID_ARGUMENT` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`CreateDurableQueue`](#createdurablequeue). |

**Response**: `ConsumeDurableEventsResponse` (stream)

```protobuf
message ConsumeDurableEventsResponse {
  repeated Event events = 1;
}
```

**Example**:

```python
request = event_consumer_pb2.ConsumeDurableEventsRequest(
    queue_id='saved-queue-id'
)

responses = stub.ConsumeDurableEvents(request, metadata=metadata)

for response in responses:
    for event in response.events:
        process_event(event)
```

### ConsumeDurableEventsWithAcks

Streams events from a durable queue with explicit client acknowledgment (bidirectional streaming). The client settles each received message by sending its delivery tag back in a subsequent request: in `ack_delivery_tags` to acknowledge it, or in `nack_delivery_tags` to have it delivered again. A nacked message is requeued for another attempt; a message that is neither acknowledged nor nacked is redelivered once the client reconnects. Redelivery is bounded rather than indefinite — the delivery-count budget and the dead-letter queue it ends at are described below.

This RPC returns `NOT_FOUND` when the queue no longer exists — either the `queue_id` is wrong, or the queue was auto-deleted after going without an attached consumer for the deployment's idle window (24 hours by default); recover by calling `CreateDurableQueue` again with the same `queue_id` and the same `filter_groups`, then resuming. See [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime).

The stream returns `INTERNAL`, with the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.`, when the broker cancels this stream's consumer or kills the channel it lives on — an acknowledgement timeout on a slow consumer is the usual cause. Reconnect and resume consuming; do not recreate the queue. The queue normally survives with its buffered messages, and messages that were in flight unacknowledged are redelivered once you reconnect. `INTERNAL` does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `NOT_FOUND`, and the recovery for a queue that is gone applies. See [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully).

**Request**: `ConsumeDurableEventsWithAcksRequest` (stream)

```protobuf
message ConsumeDurableEventsWithAcksRequest {
  string queue_id = 1;                   // Required on first request; ignored after
  repeated int64 ack_delivery_tags = 2;  // Delivery tags to acknowledge
  repeated int64 nack_delivery_tags = 3; // Delivery tags to have delivered again
}
```

The first message in the stream must include `queue_id` to identify the durable queue. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `INVALID_ARGUMENT` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`CreateDurableQueue`](#createdurablequeue). Subsequent messages carry `ack_delivery_tags` to acknowledge receipt of previously delivered events, and `nack_delivery_tags` to request redelivery of events the client failed to process. One request may carry both fields; acknowledgments are processed first, so a tag appearing in both is acknowledged. A tag the server does not recognize — already settled, or from an earlier connection — is skipped and does not end the stream.

Settle promptly. An unacknowledged message is redelivered when the client reconnects, and a nacked message is requeued and redelivered **immediately**, possibly on the same stream: the server applies no retry delay, so a client that wants one implements backoff before it nacks.

Every redelivery increments the message's delivery count, whichever way the message was requeued — by a nack, by a disconnect, or by the broker closing the stream's channel. That count is cumulative over the message's whole lifetime and is never reset, so it is a lifetime budget for the message rather than a per-nack one; nacking in a tight loop exhausts it in well under a second. A message whose delivery count passes the deployment's delivery limit is moved to the dead-letter queue rather than redelivered forever — as is one that outlives the queue's configured message TTL or is evicted because the queue reached its configured maximum length. The dead-letter queue is the destination for these poison, expired, and overflowed messages alike. See the [bounded redelivery and dead-letter queue](consuming-events.md#bounded-redelivery-and-the-dead-letter-queue) explanation in the consuming-events guide for the fuller treatment.

**Response**: `ConsumeDurableEventsWithAcksResponse` (stream)

```protobuf
message ConsumeDurableEventsWithAcksResponse {
  repeated Event events = 1;
  int64 delivery_tag = 2;  // Tag to send back to acknowledge or nack this message
}
```

**Example**:

```python
import queue

ack_queue = queue.Queue()

def request_stream(queue_id):
    # First request: identify the durable queue
    yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
        queue_id=queue_id
    )
    # Subsequent requests: send acknowledgments
    while True:
        tags = ack_queue.get()
        if tags is None:
            break
        yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
            ack_delivery_tags=tags
        )

responses = stub.ConsumeDurableEventsWithAcks(
    request_stream('saved-queue-id'), metadata=metadata
)

for response in responses:
    for event in response.events:
        process_event(event)
    # Acknowledge after processing
    ack_queue.put([response.delivery_tag])
```

To hand a message back for another attempt instead, send its `delivery_tag` in `nack_delivery_tags` — after a backoff delay, since redelivery is immediate. The consuming-events guide carries a worked example under [Step 2: Consume from Durable Queue](consuming-events.md#step-2-consume-from-durable-queue), and the full retry-budget treatment under [Bounded Redelivery and the Dead-Letter Queue](consuming-events.md#bounded-redelivery-and-the-dead-letter-queue).

### CreateDurableQueue

Creates a durable queue for persistent event storage.

This operation has ensure-exists semantics when you supply a `queue_id`: it creates the queue if no queue with that id exists, and is a no-op if a queue with that id already exists. The `created` flag in the response tells you which path the call took.

**A `queue_id` you supply must be unique across the whole deployment.** Queue ids share a single namespace. If a queue with your id already exists — whoever created it, and whatever `filter_groups` you send — this call attaches to that queue and returns it with its existing filters and `created` set to `false`. Two callers that pick the same id share one queue: each receives the other's events, and either can delete it. Use a UUID, or a namespaced convention such as `<service-name>-<purpose>`; never a bare generic word like `events`.

**Per-queue isolation is not enforced.** The three durable-queue permissions are deployment-wide rather than per-queue: a principal granted `consume-durable-queues` can consume from any durable queue whose id it knows, one granted `delete-durable-queues` can delete any of them, and `create-durable-queues` authorizes both creating a queue and updating the filters of any existing one. Treat a `queue_id` as configuration, not as a secret that protects a queue, and ask your operator to grant these permissions to the service accounts that consume events rather than to broad user groups.

The queue exists until you delete it with `DeleteDurableQueue`, or until it has gone without an attached consumer for the deployment's idle window (24 hours by default), at which point it is deleted along with any messages it still holds. Only an attached consumer keeps a durable queue alive — publishing to it and re-declaring it do not. Consuming from a queue deleted this way returns `NOT_FOUND`; recover by calling this RPC again with the same `queue_id` and the same `filter_groups`, since the queue's subscriptions are rebuilt from the filters. See [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime).

**Request**: `CreateDurableQueueRequest`

```protobuf
message CreateDurableQueueRequest {
  repeated FilterGroup filter_groups = 1;
  string queue_id = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `filter_groups` | `repeated FilterGroup` | Filters to apply to events. If omitted or empty, all events for an event type are added to the queue. |
| `queue_id` | `string` | Optional, client-specified queue identifier. An empty string means not supplied — the server generates one. When supplied, it must match `^[A-Za-z0-9._-]{1,200}$` (an id that fails this rule is rejected with `INVALID_ARGUMENT`) and it must be unique across the whole deployment — an id already in use attaches to that existing queue rather than creating one. |

**Response**: `CreateDurableQueueResponse`

```protobuf
message CreateDurableQueueResponse {
  string queue_id = 1;  // Save this, together with your filter_groups!
  bool created = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `queue_id` | `string` | The id of the durable queue. Persist this **together with the `filter_groups` you created it with** and send the id on all `ConsumeDurableEvents` calls. Both are needed to recreate the queue if it is auto-deleted after going unconsumed for the idle window, since its subscriptions are rebuilt from the filters — see [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime). |
| `created` | `bool` | `true` when this call created a new queue, `false` when an existing queue with that id was found and returned unchanged (a no-op). |

**Example**:

```python
filter_group = event_consumer_pb2.FilterGroup(
    event_type='myapp.file.uploaded'
)
resource_filter = event_consumer_pb2.ResourceFilter(
    filter_type=event_consumer_pb2.ResourceFilter.FILTER_TYPE_STARTS_WITH_GREEDY,
    resource_id='/uploads/'
)
filter_group.filters.append(resource_filter)

request = event_consumer_pb2.CreateDurableQueueRequest()
request.filter_groups.append(filter_group)

# Optional: supply your own queue_id for ensure-exists semantics. Omit it
# (leave it empty) to have the server generate one. A supplied id must be
# unique across the deployment, so namespace it as <service-name>-<purpose>.
request.queue_id = 'myapp-uploads-queue'

response = stub.CreateDurableQueue(request, metadata=metadata)
queue_id = response.queue_id

if response.created:
    print(f'Created new durable queue {queue_id}')
else:
    print(f'Reusing existing durable queue {queue_id}')

# IMPORTANT: Save queue_id AND the filter_groups it was created with to
# configuration — both are needed to recreate the queue if it is auto-deleted
# after going unconsumed for the idle window. See "Durable Queue Lifetime"
# (consuming-events.md#durable-queue-lifetime).
```

### UpdateDurableQueue

Updates filters on an existing durable queue.

This RPC is authorized by the `create-durable-queues` permission — the same one that authorizes creating a queue. There is no separate update permission, so a principal that can create durable queues can re-filter any existing one.

**Request**: `UpdateDurableQueueRequest`

```protobuf
message UpdateDurableQueueRequest {
  string queue_id = 1;
  repeated FilterGroup current_filter_groups = 2;
  repeated FilterGroup new_filter_groups = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `queue_id` | `string` | Required. The id of the durable queue to update. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `INVALID_ARGUMENT` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`CreateDurableQueue`](#createdurablequeue). |
| `current_filter_groups` | `repeated FilterGroup` | The filters currently in use on the queue. |
| `new_filter_groups` | `repeated FilterGroup` | The filters to use instead. |

**Response**: `UpdateDurableQueueResponse`

```protobuf
message UpdateDurableQueueResponse {}
```

### DeleteDurableQueue

Deletes a durable queue.

**Request**: `DeleteDurableQueueRequest`

```protobuf
message DeleteDurableQueueRequest {
  string queue_id = 1;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `queue_id` | `string` | Required. The id of the durable queue to delete. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `INVALID_ARGUMENT` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`CreateDurableQueue`](#createdurablequeue). |

**Response**: `DeleteDurableQueueResponse`

```protobuf
message DeleteDurableQueueResponse {}
```

**Example**:

```python
request = event_consumer_pb2.DeleteDurableQueueRequest(
    queue_id=queue_id
)

stub.DeleteDurableQueue(request, metadata=metadata)
```

### ManageDeadLetteredEvents

Operator-facing, bidirectional streaming RPC for inspecting and triaging dead-lettered events across the deployment. The server streams dead-lettered messages with triage metadata; the client replies with per-message actions (`ACTION_REPLAY`, `ACTION_DISCARD`, `ACTION_LEAVE`) keyed by a session-scoped delivery tag; the server streams back a correlated outcome report for each action.

**gRPC-only**: There is no REST endpoint for this RPC.

**Required permission**: `event-consumer-service:inspect-and-replay-all-dlq`. A caller lacking it is denied with `PERMISSION_DENIED`.

The stream returns `INTERNAL`, with the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.`, when the broker cancels this session's consumer or kills the channel it lives on. Reconnect to resume triage: the dead-letter queue and everything in it normally survive, and dead-lettered messages that were in flight unacknowledged are redelivered once you reconnect. `NOT_FOUND`, on the stream or on the reconnect, means the deployment's dead-letter queue is missing on the broker, which is an operator condition rather than something the caller can recover from.

This no-store bidirectional RPC is the permanent baseline DLQ-management surface; it requires no datastore beyond the message broker. It is the forward-compatible floor: future additive unary store-backed RPCs (list/filter/replay-by-id/stats) may be added later without changing this RPC. See [Managing the Dead-Letter Queue (Operators)](consuming-events.md#managing-the-dead-letter-queue-operators) in the consuming-events guide.

**Request**: `ManageDeadLetteredEventsRequest` (stream)

```protobuf
message ManageDeadLetteredEventsRequest {
  enum Action {
    ACTION_UNSPECIFIED = 0;
    ACTION_REPLAY = 1;   // Republish to origin queue (confirm-gated, no-loss)
    ACTION_DISCARD = 2;  // Ack and drop from the dead-letter queue
    ACTION_LEAVE = 3;    // Leave in place for later triage
  }

  message ActionEntry {
    int64 delivery_tag = 1;  // Session-scoped tag of the message to act on
    Action action = 2;       // Action to take on that message
  }

  repeated ActionEntry actions = 1;  // Empty on the first (opening) message
}
```

The first message opens the stream and carries no actions (the dead-letter queue is global, so there is no `queue_id`). Subsequent messages carry one or more `ActionEntry` items, each pairing a session-scoped `delivery_tag` with the `Action` to take, so multiple dead-lettered messages can be actioned in a single request.

**Request fields**:

| Field | Type | Description |
|-------|------|-------------|
| `actions` | `repeated ActionEntry` | Per-delivery-tag triage actions. Empty on the opening message. |
| `ActionEntry.delivery_tag` | `int64` | Session-scoped tag of the dead-lettered message, as received in a `dead_lettered_event`. Valid only within the live stream; not persisted or reused across reconnects. |
| `ActionEntry.action` | `Action` | `ACTION_REPLAY`, `ACTION_DISCARD`, or `ACTION_LEAVE`. |

**Response**: `ManageDeadLetteredEventsResponse` (stream)

Each response carries exactly one of two arms via a `oneof payload`: a `dead_lettered_event` (a browse item) or an `action_outcome` (a report correlated to a requested action by `delivery_tag`).

```protobuf
message ManageDeadLetteredEventsResponse {
  message DeadLetteredEvent {
    enum Reason {
      REASON_UNSPECIFIED = 0;
      REASON_DELIVERY_LIMIT = 1;  // Exceeded its redelivery limit
      REASON_EXPIRED = 2;         // Outlived the queue's configured message TTL
      REASON_MAXLEN = 3;          // Evicted when queue reached its maximum length
    }

    Reason reason = 1;
    string origin_queue = 2;                         // Origin durable queue name
    int64 delivery_count = 3;                        // Broker delivery attempts
    google.protobuf.Timestamp dead_lettered_at = 4;  // When it was dead-lettered
    Event event = 5;                                 // event_type, principal_identity, message
    string resource_id = 6;                          // Resource id of the event
    int64 delivery_tag = 7;                          // Session-scoped tag to act on
  }

  message ActionOutcome {
    enum Outcome {
      OUTCOME_UNSPECIFIED = 0;
      OUTCOME_REPLAYED = 1;              // Republished to origin queue; DLQ copy removed
      OUTCOME_DISCARDED = 2;             // Acked and dropped from the dead-letter queue
      OUTCOME_LEFT = 3;                  // Left in place for later triage
      OUTCOME_ORIGIN_QUEUE_DELETED = 4;  // Origin queue gone; DLQ copy left in place (no-loss)
      OUTCOME_REPLAY_FAILED = 5;         // Replay unconfirmed; DLQ copy left in place (no-loss)
    }

    int64 delivery_tag = 1;  // Matches the tag the client sent
    Outcome outcome = 2;
    string detail = 3;       // Human-readable context (e.g. missing queue name)
  }

  oneof payload {
    DeadLetteredEvent dead_lettered_event = 1;
    ActionOutcome action_outcome = 2;
  }
}
```

**`DeadLetteredEvent` (browse item) fields**:

| Field | Type | Description |
|-------|------|-------------|
| `reason` | `Reason` | Why the message was dead-lettered: `REASON_DELIVERY_LIMIT`, `REASON_EXPIRED`, or `REASON_MAXLEN`. |
| `origin_queue` | `string` | Name of the durable queue the message was dead-lettered from. |
| `delivery_count` | `int64` | Broker delivery attempts before dead-lettering. |
| `dead_lettered_at` | `Timestamp` | When the message was dead-lettered. |
| `event` | `Event` | The dead-lettered event (`event_type`, `principal_identity`, `message`). |
| `resource_id` | `string` | Resource id associated with the event. |
| `delivery_tag` | `int64` | Session-scoped tag the client sends back to act on the message. Live-stream-only. |

**`ActionOutcome` (report) fields**:

| Field | Type | Description |
|-------|------|-------------|
| `delivery_tag` | `int64` | Session-scoped tag identifying the message the outcome pertains to. |
| `outcome` | `Outcome` | `OUTCOME_REPLAYED`, `OUTCOME_DISCARDED`, `OUTCOME_LEFT`, `OUTCOME_ORIGIN_QUEUE_DELETED`, or `OUTCOME_REPLAY_FAILED`. |
| `detail` | `string` | Human-readable context (e.g. the missing origin queue name). |

Each action the operator sends produces exactly one correlated `action_outcome`, keyed by `delivery_tag`, so the operator can observe the result of every action. `ACTION_REPLAY` is confirm-gated and no-loss: the dead-letter copy is removed only after the publish to the origin queue is confirmed. A `ACTION_REPLAY` whose origin queue has been deleted does **not** lose the message — the server leaves the dead-letter copy in place (ACTION_LEAVE semantics) and reports an `OUTCOME_ORIGIN_QUEUE_DELETED` outcome to the operator rather than silently handling it; re-create the origin queue to obtain a viable replay target. A replay that cannot be confirmed reports `OUTCOME_REPLAY_FAILED` and likewise leaves the copy in place.

**Scope**: `ACTION_REPLAY` returns a message to the exact origin queue it dead-lettered from, taken from the message's own broker metadata — an operator cannot redirect a replay to another queue. The permission is coarse and grants deployment-wide dead-letter visibility and action by design; re-consume of a replayed message still runs the existing per-event authorization check.

## Consumer REST API

### Endpoints

#### GET /api/v1beta/events/stream

Streams events using a non-durable queue (Server-Sent Events).

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `filter_groups` | string (JSON) | No | URL-encoded JSON array of FilterGroup objects |
| `previous_filter_groups` | string (JSON) | No | URL-encoded JSON array of the previous FilterGroup objects (required when updating filters) |
| `reconnect_token` | string | No | **Deprecated — send the token in the `Last-Event-ID` header instead** (see below). Still accepted, and carries the token the same way. Token for reconnecting to the same queue after a disconnect, or for consuming from the same queue in parallel from multiple clients. Must be 1 to 200 printable ASCII characters. |

**Headers**:

| Header | Required | Description |
|--------|----------|-------------|
| `Last-Event-ID` | No | The recommended way to send a reconnect token back. Carries exactly the value the `reconnect_token` query parameter would, supports every operation it does — a filter update included — and takes the same 1-to-200-printable-ASCII rule. A browser's built-in SSE reconnect sends this header automatically. |

**Reconnecting and updating filters**: sending `previous_filter_groups` alongside `filter_groups` is what makes a request a filter update, whichever carrier brought the token. A request that carries the token alone resumes the stream on the filters it already has. A browser's automatic reconnect replays the URL the stream was opened on and sends the token in the `Last-Event-ID` header, so when that URL carried only the initial `filter_groups`, the header path resumes as-is.

The token identifies the queue. It is not a secret and grants no access on its own: presenting it requires a valid bearer token, and every delivered event is authorized against the presenting principal's own permissions. Consumers attached to one queue split its events between them, so keeping the token within the application that received it is what ensures only that application's own authorized consumers share those events.

**FilterGroup Structure**:

```json
[
  {
    "event_type": "string",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "/path/"
      }
    ]
  }
]
```

**Filter Types**:
- `eq`: Exact match
- `starts_with_lazy`: Shallow prefix match
- `starts_with_greedy`: Deep prefix match
- `unspecified`: Defaults to `eq`

**Response** (text/event-stream):

```
event: connected
id: my-queue-id
data: {"message":"Connection established","reconnect_token":"my-queue-id"}

event: event
data: {"event_type":"myapp.user.created","principal_identity":"user@example.com","occurred_at":"2024-10-16T14:30:00Z","published_at":"2024-10-16T14:30:01Z","message":{"user_id":"123"}}
```

The initial `connected` event includes the `reconnect_token` (which is the queue ID). Save this token for reconnection; it also lets multiple clients consume from the same queue in parallel, splitting its events between them.

**Errors on an open stream**: the HTTP response status is `200 OK` as soon as the stream opens, so an error that ends the stream is not an HTTP status — it is delivered as an SSE event named `error` whose `data` carries a `message` and the `status_code` it would have had, after which the stream closes. Handle the `error` event explicitly; a client that watches only for `event` receives no further events and no visible failure. The stream ends this way with `status_code` `500` and the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.` when the broker takes this stream's consumer away — reconnect with your `reconnect_token`, and the queue normally survives with its buffered backlog — and with `status_code` `404` when the queue itself is gone, in which case start a fresh stream from your `filter_groups` with no `reconnect_token`. The `500` does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `404`, and the recovery for a queue that is gone applies. See [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully).

**Example**:

```bash
# Simple consumption
curl -N https://your-consumer-service.example.com/api/v1beta/events/stream \
  -H "Authorization: Bearer YOUR_TOKEN"

# With filtering
FILTERS='[{"event_type":"myapp.user.created","filters":[]}]'
curl -N https://your-consumer-service.example.com/api/v1beta/events/stream \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -G --data-urlencode "filter_groups=$FILTERS"
```

#### GET /api/v1beta/events/stream-durable

Streams events from a durable queue (Server-Sent Events). Messages are acknowledged to the broker automatically by the server after a short delay. Clients do not need to acknowledge individual messages.

If your application requires explicit client-side acknowledgment of each message (e.g. to guarantee at-least-once processing), or needs to negatively acknowledge a message it failed to process so the broker redelivers it, use the gRPC [`ConsumeDurableEventsWithAcks`](#consumedurableeventswithacks) RPC instead. Its request carries both `ack_delivery_tags` and `nack_delivery_tags`. That RPC uses a bidirectional stream, which keeps the acknowledgment on the same connection and works correctly across multi-pod deployments — a delivery tag is scoped to the broker channel that issued it, so a separate REST call could not settle it reliably.

This endpoint returns `404` when the queue no longer exists — either the `queue_id` is wrong, or the queue was auto-deleted after going without an attached consumer for the deployment's idle window (24 hours by default); recover by creating the queue again with the same `queue_id` and the same `filter_groups`, then resuming. See [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime).

**Errors on an open stream**: once the stream has opened its HTTP response status is `200 OK`, so a status that arises after that point — including the `404` above, when the queue is deleted or reaped out from under a running stream — is not an HTTP status. It is delivered as an SSE event named `error` whose `data` carries a `message` and the `status_code` it would have had, after which the stream closes. Handle the `error` event explicitly; a client that watches only for `event` receives no further events and no visible failure. The stream also ends this way with `status_code` `500` and the message `Internal Server Error: The broker cancelled this stream's consumer. Reconnect to resume consuming.` when the broker takes this stream's consumer away — reconnect with the same `queue_id`, and do not recreate the queue; the queue normally survives with everything buffered in it. The `500` does not by itself confirm that the queue survived: if the queue no longer exists, the reconnect returns `404`, and the recovery for a queue that is gone applies. See [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully).

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `queue_id` | string | Yes | ID of the durable queue. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `422 Unprocessable Entity` before the stream opens. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`POST /api/v1beta/queues/durable`](#post-apiv1betaqueuesdurable). |

**Response** (text/event-stream):

```
event: connected
id: my-queue-id
data: {"message":"Connection established","reconnect_token":"my-queue-id"}

event: event
data: {"event_type":"myapp.user.created",...}

event: event
data: {"event_type":"myapp.user.updated",...}
```

**Example**:

```bash
curl -N https://your-consumer-service.example.com/api/v1beta/events/stream-durable?queue_id=my-queue-id \
  -H "Authorization: Bearer YOUR_TOKEN"
```

#### POST /api/v1beta/queues/durable

Creates a durable queue.

This endpoint has ensure-exists semantics when you supply `queue_id`: it creates the queue if no queue with that id exists, and is a no-op if a queue with that id already exists. The `created` field in the response tells you which path the call took. An invalid `queue_id` (one that fails the `^[A-Za-z0-9._-]{1,200}$` validation rule) is rejected with `422 Unprocessable Entity`.

**A `queue_id` you supply must be unique across the whole deployment.** Queue ids share a single namespace. If a queue with your id already exists — whoever created it, and whatever `filter_groups` you send — this endpoint attaches to that queue and returns it with its existing filters and `created` set to `false`. Two callers that pick the same id share one queue: each receives the other's events, and either can delete it. Use a UUID, or a namespaced convention such as `<service-name>-<purpose>`; never a bare generic word like `events`.

**Per-queue isolation is not enforced.** The three durable-queue permissions are deployment-wide rather than per-queue: a principal granted `consume-durable-queues` can consume from any durable queue whose id it knows, one granted `delete-durable-queues` can delete any of them, and `create-durable-queues` authorizes both creating a queue and updating the filters of any existing one. Treat a `queue_id` as configuration, not as a secret that protects a queue.

**Request Body**:

```json
{
  "filter_groups": [
    {
      "event_type": "string",
      "filters": [
        {
          "filter_type": "starts_with_greedy",
          "resource_id": "/path/"
        }
      ]
    }
  ],
  "queue_id": "myapp-uploads-queue"
}
```

`queue_id` is optional and client-specified. Omit it or send an empty string to have the server generate one. When supplied, it must match `^[A-Za-z0-9._-]{1,200}$` and must be unique across the deployment, as described above.

The HTTP status code signals which path the request took:

- `201 Created` — the request created a new durable queue (`created` is `true`). This is the case when you supply a `queue_id` that does not yet exist.
- `200 OK` — the request resolved to a queue that already existed (`created` is `false`), or you omitted `queue_id` and the server generated one (`created` is `true`).

**Response** (`201 Created`, created-new path):

```json
{
  "queue_id": "durable-queue-abc123",
  "created": true
}
```

`created` is `true` when this call created a new queue and `false` when an existing queue with the supplied `queue_id` was found and returned unchanged (a no-op). Branch on the status code to tell the two apart.

**Example**:

```bash
curl -X POST https://your-consumer-service.example.com/api/v1beta/queues/durable \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "filter_groups": [
      {
        "event_type": "myapp.file.uploaded",
        "filters": [
          {
            "filter_type": "starts_with_greedy",
            "resource_id": "/uploads/"
          }
        ]
      }
    ],
    "queue_id": "myapp-uploads-queue"
  }'
```

#### PATCH /api/v1beta/queues/durable

Updates filters on a durable queue.

This endpoint is authorized by the `create-durable-queues` permission — the same one that authorizes creating a queue. There is no separate update permission.

**Request Body**:

```json
{
  "queue_id": "string",
  "current_filter_groups": [ /* array of FilterGroup */ ],
  "new_filter_groups": [ /* array of FilterGroup */ ]
}
```

`queue_id` must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `422 Unprocessable Entity` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`POST /api/v1beta/queues/durable`](#post-apiv1betaqueuesdurable).

**Response** (200 OK):

```json
{
  "success": true
}
```

#### DELETE /api/v1beta/queues/durable

Deletes a durable queue.

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `queue_id` | string | Yes | ID of the queue to delete. It must match `^[A-Za-z0-9._-]{1,200}$`; an id that does not is rejected with `422 Unprocessable Entity` before the broker is contacted. Queue ids share one deployment-wide namespace, so this must be the id of a queue that was created by (or for) your client; see [`POST /api/v1beta/queues/durable`](#post-apiv1betaqueuesdurable). |

**Response** (200 OK):

```json
{
  "success": true
}
```

**Example**:

```bash
curl -X DELETE "https://your-consumer-service.example.com/api/v1beta/queues/durable?queue_id=my-queue-id" \
  -H "Authorization: Bearer YOUR_TOKEN"
```

## Error Handling

### Common HTTP Status Codes

| Code | Meaning | Action |
|------|---------|--------|
| 200 | Success | Process response |
| 400 | Bad Request | Check request format |
| 401 | Unauthorized | Refresh authentication token |
| 403 | Forbidden | Check permissions |
| 404 | Not Found | Check queue_id or endpoint |
| 422 | Validation Error | Check request validation |
| 500 | Internal Server Error | Retry after delay, contact support. On an SSE consume stream this status arrives inside an `error` event rather than as the response status (the response is already `200 OK`), and the stream then closes; `The broker cancelled this stream's consumer.` means reconnect and resume instead — see [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully) |
| 503 | Service Unavailable | Retry with exponential backoff |

### Common gRPC Status Codes

| Code | Meaning | Action |
|------|---------|--------|
| OK | Success | Process response |
| INVALID_ARGUMENT | Bad request | Check request format |
| UNAUTHENTICATED | Auth failed | Refresh token |
| PERMISSION_DENIED | No permission | Check permissions |
| NOT_FOUND | Resource not found | Check identifiers |
| RESOURCE_EXHAUSTED | Service overloaded | Retry with backoff |
| UNAVAILABLE | Connection lost (e.g. load balancer idle timeout) | Reconnect automatically |
| INTERNAL | Server error | Retry after delay. On a consume stream, `The broker cancelled this stream's consumer.` means reconnect and resume instead — see [Handle Disconnections Gracefully](consuming-events.md#2-handle-disconnections-gracefully) |

## Protocol Buffer Files

The complete protocol buffer definitions are available in the released archive or protobuf registry.

**Publisher Proto**: `nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto`

**Consumer Proto**: `nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto`

## OpenAPI Specifications

Full OpenAPI specifications are available in the released archive

## Rate Limits and Quotas

Currently, there are no enforced rate limits at the application level. However:

- The service may return `RESOURCE_EXHAUSTED` (gRPC) or `503` (HTTP) if temporarily overloaded
- Implement exponential backoff when you receive these errors
- For high-volume publishing, use batch publishing for better performance

## Versioning

The current API version is **v1beta**.

- APIs are under the `/api/v1beta/` path (REST) or `v1beta` package (gRPC)
- Beta APIs may change, but we will maintain backward compatibility where possible
- When the API reaches GA, it will be versioned as `v1`

## Service Health

Both services expose health check endpoints:

**gRPC**: Standard gRPC health checking protocol

**REST**: 
- `GET /health` - Service health status
- Returns 200 if healthy

## Metrics and Monitoring

Both services expose OpenTelemetry metrics and traces when telemetry is enabled via configuration.

## Next Steps

- Try the [Getting Started Guide](getting-started.md) for hands-on examples
- Learn about [Publishing Events](publishing-events.md)
- Learn about [Consuming Events](consuming-events.md)
- Understand [Permissions & Authorization](permissions.md)

