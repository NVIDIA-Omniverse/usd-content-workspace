# Publishing Events

This guide covers how to publish events to the Notifications Service using both the REST API and gRPC API.

## Overview

The Event Aggregation Service accepts events from publishers and routes them to consumers through RabbitMQ. Publishers can send individual events or batch multiple events for better performance.

## Event Structure

Every event consists of the following fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `event_type` | string | Yes | A unique identifier for the type of event (e.g., "storage.file.created") |
| `message` | JSON object | Yes | The event payload - structure is defined by you |
| `occurred_at` | timestamp | Yes | When the event occurred (ISO 8601 format) |
| `resource` | object | No | Optional resource identifier for filtering |

### Event Type

The `event_type` is a string that identifies what kind of event this is. **You define your own event types** - the Notifications Service doesn't validate or enforce any particular naming scheme.

**Recommended conventions**:
- Use dot notation: `service.entity.action` (e.g., "storage.file.created")
- Be consistent across your organization
- Document your event types
- Use lowercase
- Be specific enough to allow filtering but not so specific that you have too many types

Examples:
```
storage.file.created
storage.file.deleted
storage.directory.created
project.workflow.completed
user.permission.updated
thumbnail.generated
```

### Message

The `message` field contains the actual event payload as a JSON object. **The structure is entirely up to you** - define it based on what information consumers need.

**Important**: The Notifications Service does not validate or know anything about your message structure. It's a contract between your publishers and consumers.

**Best practices**:
- Document your message schema for each event type
- Include enough information for consumers to process the event
- Keep messages reasonably sized (avoid embedding large binary data)
- Use consistent field naming across event types
- Include identifiers that allow consumers to fetch more data if needed

Example message schemas:

```json
// storage.file.created
{
  "file_name": "document.pdf",
  "file_size": 1024576,
  "mime_type": "application/pdf",
  "uploaded_by": "user@example.com",
  "checksum": "sha256:abc123..."
}

// project.workflow.completed
{
  "workflow_id": "wf-12345",
  "workflow_name": "Rendering Pipeline",
  "status": "success",
  "duration_seconds": 3600,
  "output_location": "/renders/output.mp4"
}
```

### Occurred At

An ISO 8601 timestamp indicating when the event occurred. Always use UTC.

Format: `YYYY-MM-DDTHH:MM:SSZ`

Example: `2024-10-16T14:30:00Z`

### Resource (Optional)

The `resource` field contains a hierarchical identifier (like a file path) that consumers can use for filtering.

```json
{
  "resource_id": "/folder/subfolder/file.txt"
}
```

Resource IDs should be:
- Path-like (slash-separated)
- Hierarchical (broader to more specific)
- Consistent across your event types

Examples:
```
/projects/projectA/file.txt
/users/12345
/workspaces/ws-001/render/output.mp4
```

If you omit the resource field, it's equivalent to including a resource that has `resource_id=""`.

## Publishing a Single Event

### REST API

**Endpoint**: `POST /api/v1beta/events`

**Example**:

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "event": {
      "event_type": "storage.file.created",
      "message": {
        "file_name": "document.pdf",
        "file_size": 1024576,
        "mime_type": "application/pdf",
        "uploaded_by": "user@example.com"
      },
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/uploads/documents/document.pdf"
      }
    }
  }'
```

**Response (Success)**:

```json
{
  "result": {
    "event": {
      "event_type": "storage.file.created",
      "message": {
        "file_name": "document.pdf",
        "file_size": 1024576,
        "mime_type": "application/pdf",
        "uploaded_by": "user@example.com"
      },
      "occurred_at": "2024-10-16T14:30:00Z",
      "resource": {
        "resource_id": "/uploads/documents/document.pdf"
      }
    },
    "success": true
  }
}
```

**Response (Failure)**:

```json
{
  "result": {
    "event": { /* event details */ },
    "success": false,
    "failure_reason": "Unable to connect to message broker"
  }
}
```

### gRPC API

**RPC**: `PublishEvent`

**Python Example**:

```python
import grpc
from google.protobuf.timestamp_pb2 import Timestamp
from google.protobuf.struct_pb2 import Struct
from datetime import datetime
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-aggregation-service.example.com:50051',
    grpc.ssl_channel_credentials()
)
stub = event_publisher_pb2_grpc.EventPublishingServiceStub(channel)

# Create message
message = Struct()
message.update({
    'file_name': 'document.pdf',
    'file_size': 1024576,
    'mime_type': 'application/pdf',
    'uploaded_by': 'user@example.com'
})

# Create timestamp
timestamp = Timestamp()
timestamp.FromDatetime(datetime(2024, 10, 16, 14, 30, 0))

# Create event
event = event_publisher_pb2.Event(
    event_type='storage.file.created',
    message=message,
    occurred_at=timestamp,
    resource=event_publisher_pb2.EventResource(
        resource_id='/uploads/documents/document.pdf'
    )
)

# Publish
request = event_publisher_pb2.PublishEventRequest(event=event)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

try:
    response = stub.PublishEvent(request, metadata=metadata)
    if response.result.success:
        print("Event published successfully")
    else:
        print(f"Failed: {response.result.failure_reason}")
except grpc.RpcError as e:
    print(f"RPC failed: {e.code()}: {e.details()}")
```

**Go Example**:

```go
package main

import (
    "context"
    "time"
    
    "google.golang.org/grpc"
    "google.golang.org/grpc/credentials"
    "google.golang.org/protobuf/types/known/structpb"
    "google.golang.org/protobuf/types/known/timestamppb"
    
    pb "nvidia/omniverse/notifications/publisher/v1beta"
)

func main() {
    // Setup connection
    creds := credentials.NewTLS(&tls.Config{})
    conn, err := grpc.Dial(
        "your-aggregation-service.example.com:50051",
        grpc.WithTransportCredentials(creds),
    )
    if err != nil {
        panic(err)
    }
    defer conn.Close()
    
    client := pb.NewEventPublishingServiceClient(conn)
    
    // Create message
    message, _ := structpb.NewStruct(map[string]interface{}{
        "file_name":   "document.pdf",
        "file_size":   1024576,
        "mime_type":   "application/pdf",
        "uploaded_by": "user@example.com",
    })
    
    // Create event
    event := &pb.Event{
        EventType:  "storage.file.created",
        Message:    message,
        OccurredAt: timestamppb.New(time.Now()),
        Resource: &pb.EventResource{
            ResourceId: "/uploads/documents/document.pdf",
        },
    }
    
    // Publish
    ctx := context.Background()
    // Add auth metadata
    ctx = metadata.AppendToOutgoingContext(ctx, "authorization", "Bearer "+YOUR_TOKEN)
    
    resp, err := client.PublishEvent(ctx, &pb.PublishEventRequest{
        Event: event,
    })
    
    if err != nil {
        panic(err)
    }
    
    if resp.Result.Success {
        println("Event published successfully")
    } else {
        println("Failed:", resp.Result.FailureReason)
    }
}
```

## Batch Publishing Multiple Events

When you need to publish multiple events, use batch publishing for better performance. Events in a batch are published in parallel.

### REST API

**Endpoint**: `POST /api/v1beta/events/batch`

**Example**:

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events/batch \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "events": [
      {
        "event_type": "storage.file.created",
        "message": {
          "file_name": "document1.pdf"
        },
        "occurred_at": "2024-10-16T14:30:00Z",
        "resource": {
          "resource_id": "/uploads/document1.pdf"
        }
      },
      {
        "event_type": "storage.file.created",
        "message": {
          "file_name": "document2.pdf"
        },
        "occurred_at": "2024-10-16T14:30:01Z",
        "resource": {
          "resource_id": "/uploads/document2.pdf"
        }
      },
      {
        "event_type": "storage.file.created",
        "message": {
          "file_name": "document3.pdf"
        },
        "occurred_at": "2024-10-16T14:30:02Z",
        "resource": {
          "resource_id": "/uploads/document3.pdf"
        }
      }
    ]
  }'
```

**Response**:

```json
{
  "results": [
    {
      "event": { /* event 1 */ },
      "success": true
    },
    {
      "event": { /* event 2 */ },
      "success": true
    },
    {
      "event": { /* event 3 */ },
      "success": false,
      "failure_reason": "Failed to publish to exchange"
    }
  ]
}
```

**Important**: Always check the `results` array - some events may succeed while others fail.

### gRPC API

**RPC**: `BatchPublishEvents`

**Python Example**:

```python
import grpc
from google.protobuf.timestamp_pb2 import Timestamp
from google.protobuf.struct_pb2 import Struct
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

# Setup channel and stub
channel = grpc.secure_channel(
    'your-aggregation-service.example.com:50051',
    grpc.ssl_channel_credentials()
)
stub = event_publisher_pb2_grpc.EventPublishingServiceStub(channel)

# Create multiple events
events = []
for i in range(3):
    message = Struct()
    message.update({
        'file_name': f'document{i+1}.pdf'
    })
    
    timestamp = Timestamp()
    timestamp.GetCurrentTime()
    
    event = event_publisher_pb2.Event(
        event_type='storage.file.created',
        message=message,
        occurred_at=timestamp,
        resource=event_publisher_pb2.EventResource(
            resource_id=f'/uploads/document{i+1}.pdf'
        )
    )
    events.append(event)

# Batch publish
request = event_publisher_pb2.BatchPublishEventsRequest()
for event in events:
    request.events.append(event)
metadata = [('authorization', f'Bearer {YOUR_TOKEN}')]

try:
    response = stub.BatchPublishEvents(request, metadata=metadata)
    
    # Check each result
    for idx, result in enumerate(response.results):
        if result.success:
            print(f"Event {idx+1} published successfully")
        else:
            print(f"Event {idx+1} failed: {result.failure_reason}")
            
except grpc.RpcError as e:
    print(f"RPC failed: {e.code()}: {e.details()}")
```

## Error Handling

### Common Error Codes

| HTTP Status / gRPC Code | Meaning | Action |
|------------------------|---------|--------|
| 401 / UNAUTHENTICATED | Authentication failed | Check your token is valid and not expired |
| 403 / PERMISSION_DENIED | Not authorized to publish this event type | Check permissions configuration |
| 400 / INVALID_ARGUMENT | Invalid request format | Check your event structure matches the schema |
| 503 / RESOURCE_EXHAUSTED | Service temporarily overloaded | Retry with exponential backoff |
| 500 / INTERNAL | Server error | Check service health, contact support |

### Retry Logic

Implement retry logic with exponential backoff for transient failures:

```python
import time
from typing import Callable

def publish_with_retry(
    publish_func: Callable,
    max_retries: int = 3,
    initial_delay: float = 1.0
) -> bool:
    """
    Retry publishing with exponential backoff.
    """
    delay = initial_delay
    
    for attempt in range(max_retries):
        try:
            response = publish_func()
            
            if response.result.success:
                return True
            else:
                print(f"Publish failed: {response.result.failure_reason}")
                return False
                
        except grpc.RpcError as e:
            if e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                if attempt < max_retries - 1:
                    print(f"Service overloaded, retrying in {delay}s...")
                    time.sleep(delay)
                    delay *= 2  # Exponential backoff
                else:
                    print("Max retries exceeded")
                    return False
            else:
                # Don't retry for other errors
                print(f"Non-retriable error: {e.code()}")
                return False
    
    return False

# Usage
def publish():
    return stub.PublishEvent(request, metadata=metadata)

success = publish_with_retry(publish)
```

## Best Practices

### 1. Choose Appropriate Event Granularity

**Too granular**:
```python
# Publishing separate events for every field change
publish_event("user.email.changed", {...})
publish_event("user.name.changed", {...})
publish_event("user.phone.changed", {...})
```

**Better**:
```python
# Single event for the update with changed fields in message
publish_event("user.updated", {
    "user_id": "123",
    "changed_fields": ["email", "name", "phone"],
    "new_values": {...}
})
```

### 2. Include Timestamps Accurately

Use the actual time the event occurred, not when you're publishing it:

```python
# Good
occurred_at = user_created_timestamp

# Bad
occurred_at = datetime.utcnow()  # Time of publishing, not event occurrence
```

### 3. Use Resource IDs Consistently

Establish conventions for resource IDs across your organization:

```python
# Good - consistent path structure
"/users/{user_id}"
"/projects/{project_id}/files/{file_id}"
"/workspaces/{workspace_id}"

# Bad - inconsistent
"user-123"
"project_456/file789"
"/workspace:ws-001"
```

### 4. Batch When Possible

If publishing multiple events at once, use batch publishing:

```python
# Good - batch publish
events = [create_event(file) for file in uploaded_files]
batch_publish(events)

# Bad - individual publishes in a loop
for file in uploaded_files:
    publish_event(create_event(file))  # Slower
```

### 5. Handle Partial Batch Failures

When batch publishing, some events may succeed while others fail:

```python
response = stub.BatchPublishEvents(request, metadata=metadata)

failed_events = [
    result.event for result in response.results 
    if not result.success
]

if failed_events:
    # Log failures, retry, or handle them appropriately
    log_failed_events(failed_events)
    # Maybe retry just the failed events
    retry_publish(failed_events)
```

### 6. Keep Messages Reasonably Sized

Don't embed large payloads in events:

```python
# Bad - embedding large binary data
publish_event("image.processed", {
    "image_data": base64_encode(large_image)  # Don't do this!
})

# Good - reference the data
publish_event("image.processed", {
    "image_url": "https://storage.example.com/images/123.jpg",
    "thumbnail_url": "https://storage.example.com/thumbs/123.jpg",
    "size_bytes": 1024576
})
```

### 7. Document Your Events

Maintain documentation of your event types and message schemas:

```python
"""
Event Type: storage.file.created

Description: Published when a new file is uploaded to storage

Message Schema:
{
    "file_name": str,           # Name of the file
    "file_size": int,           # Size in bytes
    "mime_type": str,           # MIME type of the file
    "uploaded_by": str,         # Email of uploader
    "checksum": str,            # SHA-256 checksum (optional)
    "metadata": dict            # Additional metadata (optional)
}

Resource ID Format: /path/to/file
Example: /uploads/2024/10/document.pdf
"""
```

## Performance Considerations

- **Batch publishing**: Use for multiple events (faster than individual publishes)
- **Connection pooling**: Reuse gRPC channels/connections instead of creating new ones for each publish
- **Async publishing**: If your language/framework supports it, publish asynchronously
- **Message size**: Keep messages under 100KB for optimal performance

## Next Steps

- Learn how to [Consume Events](consuming-events.md)
- Understand [Permissions & Authorization](permissions.md) for publishing
- Review the [API Reference](api-reference.md) for complete API details

