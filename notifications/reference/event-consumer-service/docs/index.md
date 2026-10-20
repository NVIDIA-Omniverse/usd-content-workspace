# Omniverse Notifications APIs Developer Guide

The Notifications Service is a distributed event messaging system that enables real-time event publishing and consumption across NVIDIA Omniverse services. It consists of two main components working together to provide a scalable, permission-aware event streaming infrastructure.

## What is the Notifications Service?

The Notifications Service provides a way for applications to:

- **Publish events** when significant actions occur (e.g., file created, user updated, workflow completed)
- **Consume events** in real-time to react to changes happening across the system
- **Filter events** to receive only the specific events your application cares about
- **Control access** through fine-grained permissions on both publishing and consuming

## Architecture Overview

The service consists of two independent services:

1. **Event Aggregation Service** (Publisher API) - Accepts events from publishers and routes them to RabbitMQ
2. **Event Consumer Service** (Consumer API) - Allows consumers to stream events from RabbitMQ with filtering and permissions

```
 Your App                                                                      Your App
 (Publisher)         Notifications Service Infrastructure                     (Consumer)
     │                                                                            │
     │              ┌──────────────────────────────────────────┐                  │
     │              │                                          │                  │
     │              │  ┌────────────────┐    ┌─────────────┐   │                  │
     └─────────────>│  │  Event         │───>│  RabbitMQ   │   │──────────────────┘
                    │  │  Aggregation   │    │  (Message   │   │
                    │  │  Service (API) │    │   Broker)   │   │
                    │  └────────────────┘    └─────────────┘   │
                    │                              │           │
                    │                              ↓           │
                    │                        ┌─────────────┐   │
                    │                        │  Event      │   │
                    │                        │  Consumer   │   │
                    │                        │  Service    │   │
                    │                        │  (API)      │   │
                    │                        └─────────────┘   │
                    │                                          │
                    └──────────────────────────────────────────┘
```

**What this means:**
- **Your App (Publisher)**: Any service you build that needs to publish events (e.g., a storage service publishing "file created" events)
- **Event Aggregation Service**: The Notifications Service API that receives your published events
- **RabbitMQ**: The message broker (managed by the Notifications Service)
- **Event Consumer Service**: The Notifications Service API that streams events to consumers
- **Your App (Consumer)**: Any service you build that needs to consume events (e.g., a thumbnail generator listening for file created events)

The **Notifications Service** = Event Aggregation Service + RabbitMQ + Event Consumer Service (the infrastructure in the box). You just interact with the two APIs.

## Key Concepts

### Events

Events represent something that happened in your system. Each event has:

- **event_type**: A string identifying the type of event (e.g., "storage.file.created")
- **message**: JSON payload containing event-specific data
- **occurred_at**: Timestamp when the event occurred
- **resource**: Optional path-like identifier for filtering (e.g., "/folder/subfolder/file.txt")

### Event Types and Message Contracts

**Important**: The Notifications Service is **event-type agnostic**. This means:

- The service does not know or validate the structure of your event types or message bodies
- Event types and message schemas are contracts between publishers and consumers
- Publishers and consumers must agree on:
  - What event type strings to use
  - What structure the message JSON should have
  - What the resource_id paths represent

For example, if you're building a thumbnail service:
- Define your own event types like "thumbnail.created" or "thumbnail.updated"
- Document the message structure (e.g., `{"thumbnail_url": "...", "size": "..."}`)
- Ensure both publishers and consumers use the same schema

### Durable vs Non-Durable Queues

The Consumer API supports two consumption modes:

**Non-Durable Queues**:
- Receive only events that occur while actively connected
- Lightweight, no setup required
- Best for real-time updates where missing messages is acceptable
- Automatically cleaned up when disconnected

**Durable Queues**:
- Persist events across disconnects, retaining them up to a configurable message TTL and a bounded maximum queue length
- Must be explicitly created before use
- Receive events from the time the queue was created, subject to those bounds
- Deliver at-least-once with bounded redelivery: a message that cannot be acknowledged within the redelivery limit — or that exceeds the message TTL or the maximum-length bound — is moved to a dead-letter queue rather than retained or redelivered indefinitely
- Best for critical event processing where events should not be silently missed within those bounds
- Require explicit cleanup when no longer needed, and are deleted automatically if left without a consumer for the deployment's idle window (24 hours by default)

Durability is therefore bounded, not unconditional. See [Bounded Redelivery and the Dead-Letter Queue](consuming-events.md#bounded-redelivery-and-the-dead-letter-queue) in the [Consuming Events](consuming-events.md) guide for the redelivery, TTL, and max-length behavior, [Durable Queue Lifetime](consuming-events.md#durable-queue-lifetime) for the idle window and how a client recovers from it, and the same guide for guidance on when to use each queue type.

Dead-lettered messages are not a dead end: operators holding the appropriate permission can inspect them and replay, discard, or leave each one through a permission-gated, gRPC-only management RPC. See [Managing the Dead-Letter Queue (Operators)](consuming-events.md#managing-the-dead-letter-queue-operators) in the [Consuming Events](consuming-events.md) guide.

### Resource Filtering

Events can include a hierarchical `resource_id` (like a file path) that consumers can filter on:

- **Exact match**: Only events for a specific resource
- **Prefix match (lazy)**: Events for resources at a specific level
- **Prefix match (greedy)**: Events for resources and all sub-levels

Example:
```
Resource: "/projects/projectA/file.txt"

Filter "/projects/projectA/" (greedy)  → Matches ✓
Filter "/projects/" (greedy)           → Matches ✓
Filter "/projects/" (lazy)             → No match ✗ (file is nested too deep)
```

## Available APIs

Both services provide dual APIs:

- **gRPC API**: High-performance, strongly-typed, ideal for service-to-service communication
- **REST API**: HTTP/JSON-based, easier for testing and web clients, uses Server-Sent Events (SSE) for streaming

## Documentation Structure

```{toctree}
:maxdepth: 2

getting-started
publishing-events
consuming-events
permissions
api-reference
```

## Quick Links

- [Getting Started Guide](getting-started.md) - Set up and run your first example
- [Publishing Events](publishing-events.md) - Learn how to publish events (REST and gRPC)
- [Consuming Events](consuming-events.md) - Learn how to consume events (REST and gRPC)
- [Permissions & Authorization](permissions.md) - Understanding access control
- [API Reference](api-reference.md) - Detailed API documentation

## Support and Contributing

For questions, issues, or contributions, please contact the Omniverse Notifications Service team.

