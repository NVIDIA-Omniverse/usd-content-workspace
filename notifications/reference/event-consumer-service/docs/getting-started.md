# Getting Started

This guide will help you quickly set up and start using the Notifications Service to publish and consume events.

## Prerequisites

- Access to a deployed Notifications Service (Event Aggregation and Event Consumer services)
- Authentication credentials (JWT token or similar)
- For gRPC: Protocol Buffer compiler and gRPC libraries for your language
- For REST: HTTP client library (or just curl for testing)

## Authentication

Both services require authentication. Include your bearer token in requests:

**REST**: Include the `Authorization` header:
```
Authorization: Bearer YOUR_TOKEN_HERE
```

**gRPC**: Include the token in metadata:
```python
# Python example
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]
```

## Quick Example: Publish and Consume an Event

Let's walk through a complete example of publishing an event and consuming it.

### Step 1: Publish an Event (REST)

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "event": {
      "event_type": "myapp.user.created",
      "message": {
        "user_id": "12345",
        "username": "john_doe",
        "email": "john@example.com"
      },
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/users/12345"
      }
    }
  }'
```

**Response**:
```json
{
  "result": {
    "event": {
      "event_type": "myapp.user.created",
      "message": {
        "user_id": "12345",
        "username": "john_doe",
        "email": "john@example.com"
      },
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/users/12345"
      }
    },
    "success": true
  }
}
```

### Step 2: Consume Events (REST with SSE)

Open a streaming connection to receive events:

```bash
curl -N https://your-consumer-service.example.com/api/v1beta/events/stream \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -G --data-urlencode 'filter_groups=[{
    "event_type": "myapp.user.created",
    "filters": []
  }]'
```

You'll receive Server-Sent Events as they occur:

```
event: event
data: {
  "event_type": "myapp.user.created",
  "principal_identity": "publisher@example.com",
  "occurred_at": "2024-10-16T14:30:00Z",
  "published_at": "2024-10-16T14:30:01Z",
  "message": {
    "user_id": "12345",
    "username": "john_doe",
    "email": "john@example.com"
  }
}
```

## Quick Example: gRPC

### Step 1: Generate Client Code

First, obtain the proto files from the released archive or protobuf registry.

Generate client code for your language:

```bash
# Python example
python -m grpc_tools.protoc \
  -I./protos \
  --python_out=. \
  --grpc_python_out=. \
  nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto \
  nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto
```

### Step 2: Publish an Event (gRPC)

```python
import grpc
from google.protobuf.timestamp_pb2 import Timestamp
from google.protobuf.struct_pb2 import Struct
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

# Create channel with credentials
credentials = grpc.ssl_channel_credentials()
channel = grpc.secure_channel(
    'your-aggregation-service.example.com:50051',
    credentials
)
stub = event_publisher_pb2_grpc.EventPublishingServiceStub(channel)

# Create event
message = Struct()
message.update({
    'user_id': '12345',
    'username': 'john_doe',
    'email': 'john@example.com'
})

timestamp = Timestamp()
timestamp.GetCurrentTime()

event = event_publisher_pb2.Event(
    event_type='myapp.user.created',
    message=message,
    occurred_at=timestamp,
    resource=event_publisher_pb2.EventResource(
        resource_id='/users/12345'
    )
)

# Publish event
request = event_publisher_pb2.PublishEventRequest(event=event)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]
response = stub.PublishEvent(request, metadata=metadata)

print(f"Published successfully: {response.result.success}")
```

### Step 3: Consume Events (gRPC)

```python
import grpc
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

# Create channel with credentials
credentials = grpc.ssl_channel_credentials()
channel = grpc.secure_channel(
    'your-consumer-service.example.com:50052',
    credentials
)
stub = event_consumer_pb2_grpc.EventConsumerServiceStub(channel)

# Create filter groups
filter_group = event_consumer_pb2.FilterGroup(
    event_type='myapp.user.created'
)
# No filters added - receive all events of this type

# Create request stream
def request_stream():
    request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group)
    yield request

# Consume events
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]
responses = stub.ConsumeNonDurableEvents(request_stream(), metadata=metadata)

for response in responses:
    # Process each batch of events
    for event in response.events:
        print(f"Received event: {event.event_type}")
        print(f"Message: {event.message}")
        print(f"Occurred at: {event.occurred_at}")
```

## Common Patterns

### 1. Define Your Event Types

Before implementing, define your event type naming convention:

```python
# Example event type definitions
USER_CREATED = "myapp.user.created"
USER_UPDATED = "myapp.user.updated"
USER_DELETED = "myapp.user.deleted"
FILE_UPLOADED = "myapp.file.uploaded"
WORKFLOW_COMPLETED = "myapp.workflow.completed"
```

### 2. Define Your Message Schemas

Document the structure of messages for each event type:

```python
# Example: myapp.user.created message schema
{
    "user_id": str,      # Required: Unique user identifier
    "username": str,     # Required: User's username
    "email": str,        # Required: User's email
    "created_by": str,   # Optional: Who created this user
    "metadata": dict     # Optional: Additional metadata
}
```

### 3. Handle Errors

Both publishing and consuming can fail. Always handle errors appropriately:

```python
# Publishing with error handling
try:
    response = stub.PublishEvent(request, metadata=metadata)
    if response.result.success:
        print("Event published successfully")
    else:
        print(f"Failed to publish: {response.result.failure_reason}")
except grpc.RpcError as e:
    if e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
        # Retry with exponential backoff
        print("Service temporarily unavailable, retrying...")
    elif e.code() == grpc.StatusCode.PERMISSION_DENIED:
        print("Permission denied - check your authorization")
    else:
        print(f"Unexpected error: {e.details()}")
```

## Next Steps

Now that you've published and consumed your first events:

- Learn more about [Publishing Events](publishing-events.md) including batch publishing
- Explore [Consuming Events](consuming-events.md) including durable queues and filtering
- Understand [Permissions & Authorization](permissions.md) to secure your events
- Review the [API Reference](api-reference.md) for complete API documentation

## Troubleshooting

### "Permission denied" errors

Make sure you have the appropriate permissions configured. See the [Permissions](permissions.md) guide.

### "Service unavailable" errors

The service may be temporarily overloaded. Implement retry logic with exponential backoff.

### Events not being received

- Check that your filter groups match the event types being published
- Verify your authentication token is valid
- Ensure the publisher is successfully publishing events

### Connection drops

For non-durable queues, use the `reconnect_token` to reconnect to the same queue. That resumes the queue with the bindings it already holds, so a gRPC client that has changed filters mid-stream also re-sends the updates the server did not acknowledge — see [Updating Filters Dynamically](consuming-events.md#grpc-api). For durable queues, reconnect with the same `queue_id`. If that reconnect returns `NOT_FOUND`, the queue was deleted after going unconsumed for the deployment's idle window — recreate it with the same `queue_id` and `filter_groups` and resume. See [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime).

