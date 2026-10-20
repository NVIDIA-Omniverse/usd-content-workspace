# Permissions & Authorization

This guide explains how authentication and authorization work in the Notifications Service, including special cases for storage events and configurable per-event-type permissions.

## Overview

The Notifications Service integrates with a permissions API to control:
- **Who can publish** events of specific types
- **Who can create, delete, and consume from durable queues**
- **Who can consume specific events** from non-durable queues
- **Fine-grained access control** based on event content

## Authentication

All requests to both the Event Aggregation Service and Event Consumer Service require authentication via bearer tokens.

### Including Auth Token

**REST API**:
```
Authorization: Bearer YOUR_JWT_TOKEN
```

**gRPC**:
```python
metadata = [('authorization', 'Bearer YOUR_JWT_TOKEN')]
stub.PublishEvent(request, metadata=metadata)
```

### Token Expiration

**Important**: Tokens can expire **while you're streaming events**. When this happens:

1. The stream will terminate with a 401 (UNAUTHENTICATED) error
2. Your client must refresh the token
3. Reconnect with the fresh token
   - **Non-durable queues**: Use the `reconnect_token` to avoid missing events
   - **Durable queues**: Simply reconnect with the same `queue_id`

See the [Consuming Events guide](consuming-events.md#authorization-errors-during-streaming) for code examples.

## Publishing Permissions

### Event Type Permissions

To publish an event, the authenticated principal must have permission for that specific event type.

The Event Aggregation Service checks permissions using:
- **Action**: `publish-event`
- **Resource Type**: `EventType`
- **Resource ID**: The event_type string (e.g., "storage.file.created")

### Example Policy

In your permissions service (e.g., Cedar policy):

```
// Allow user to publish storage events
permit(
  principal == Principal::"user@example.com",
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"storage.file.created"
);

// Allow a service to publish workflow events
permit(
  principal == Principal::"workflow-service",
  action == Action::"event-aggregation-service:publish-event",
  resource == EventType::"workflow.completed"
);
```

### REST Example

```bash
curl -X POST https://your-aggregation-service.example.com/api/v1beta/events \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "event": {
      "event_type": "myapp.user.created",
      ...
    }
  }'
```

If the principal doesn't have `publish-event` permission for `EventType::"myapp.user.created"`, you'll receive:

```
HTTP 403 Forbidden
{
  "detail": "Not allowed to publish events of type myapp.user.created"
}
```

### Configuration

The Event Aggregation Service is configured via environment variables:

| Variable | Description |
|----------|-------------|
| `OMNI_EVENTS_PERMISSIONS_ENDPOINT` | URL of the permissions service. If not set, permissions checks are **disabled**. |
| `OMNI_EVENTS_PERMISSIONS_TTL_SECONDS` | Cache TTL for permissions checks (default: 600) |

**Example**:
```bash
OMNI_EVENTS_PERMISSIONS_ENDPOINT=https://permissions.example.com
OMNI_EVENTS_PERMISSIONS_TTL_SECONDS=300
```

## Consuming Permissions

Consuming permissions are more complex because they involve:
1. General permissions for queue management
2. Special handling for storage events
3. Optional fine-grained per-event-type permissions

### Durable Queue Permissions

To work with durable queues, principals need specific permissions:

There are three of them — `create-durable-queues`, `consume-durable-queues`, and `delete-durable-queues` — covering four operations: updating a queue's filters is authorized by `create-durable-queues`, and there is no separate update action.

Each is a service-level permission, not a per-queue one: a principal granted `consume-durable-queues` may consume from any durable queue whose `queue_id` it knows, one granted `delete-durable-queues` may delete any durable queue in the deployment, and one granted `create-durable-queues` may re-filter any of them as well as create new ones. Grant them to the service accounts that operate durable queues rather than to broad user groups — see [Use Service Accounts for Durable Queues](#1-use-service-accounts-for-durable-queues).

#### Create Durable Queue

- **Action**: `create-durable-queues`
- **Resource**: None (service-level permission)

**Policy Example**:
```
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:create-durable-queues"
);
```

#### Update Durable Queue

- **Action**: `create-durable-queues`
- **Resource**: None (service-level permission)

Updating the filters on an existing durable queue is authorized by the same action as creating one, so the policy example above covers it. No `update-durable-queues` action exists.

#### Consume from Durable Queue

- **Action**: `consume-durable-queues`
- **Resource**: None (service-level permission)

**Policy Example**:
```
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:consume-durable-queues"
);
```

#### Delete Durable Queue

- **Action**: `delete-durable-queues`
- **Resource**: None (service-level permission)

**Policy Example**:
```
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:delete-durable-queues"
);
```

### Durable Queue Event Permissions

When consuming from a durable queue, **additional permissions checks** are performed on individual events based on their event type.

**For storage events**, the service checks:
- `storage.file.created` → Action: `consume-all-storage-create-events`
- `storage.file.deleted` → Action: `consume-all-storage-delete-events`

**Policy Example**:
```
// Allow thumbnail service to consume all file creation events
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:consume-all-storage-create-events"
);
```

**For non-storage events** with configured permissions (see the Fine-Grained Permissions for Arbitrary Event Types section below), the action and resource from the configuration are used.

### Non-Durable Queue Permissions

Non-durable queues don't require special queue management permissions, but **every event** is checked for permissions before being delivered.

This is where the sophisticated permission models come into play:

1. **Storage events**: Use storage-API-based "docs" permissions
2. **Configured event types**: Use permissions API with configured actions
3. **Unconfigured event types**: No permissions check (allowed by default)

## Storage Events: Special "Docs" Permissions

Storage events have special handling because they relate to files and directories that have their own access control in the Storage API.

### Storage Event Types

The following event types are treated as storage events:

- `omni.storage.created` - File created
- `omni.storage.deleted` - File deleted  
- `omni.storage.dir_created` - Directory created
- `omni.storage.dir_deleted` - Directory deleted

(Note: The actual event type strings may vary based on your storage service configuration)

### How Storage Permissions Work

**For non-durable queues**, the Event Consumer Service queries the **Storage API** to check if the user has access to the files/directories in the events.

This is sometimes called **"docs" permissions** because it's based on the actual documents/files the user can access in storage.

#### For `storage.file.created` Events

When a file is created:
1. The consumer service checks if the user has access to the **parent directory**
2. If they have any files in that directory (i.e., they can "see" the directory), they receive the event
3. The file is added to a cache so that if it's deleted later, the service knows they should receive the delete event

#### For `storage.file.deleted` Events

When a file is deleted:
1. The service checks if the user had access to that file (from the cache or by checking the parent directory)
2. If they had access, they receive the delete event
3. The file is removed from the cache

#### For `storage.dir_created` Events

When a directory is created:
1. The service checks if the user has access to the **parent directory**
2. It queries the Storage API to see if they have any files in the **new directory**
3. If they have files in the new directory, they receive the event
4. The directory is added to the cache

#### For `storage.dir_deleted` Events

When a directory is deleted:
1. The service checks if the user had access to the directory (from the cache)
2. It checks if they still have files in that "deleted" directory (in case it's a synthetic delete event)
3. If the directory was truly deleted for them, they receive the event
4. The directory is removed from the cache

### Why "Docs" Permissions?

This approach ensures that:
- Users only see events for files they have access to
- A user creating a file in a shared folder only sees that event if they can access the folder
- Delete events are only sent if the user could see the file/directory in the first place
- Permissions are enforced at the storage layer, not duplicated in the events system

### Configuration for Storage Events

The Event Consumer Service needs to know which Storage API endpoints to query:

**Environment Variable**: `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH`

**YAML Configuration File**:
```yaml
storage_permissions_endpoints:
  - https://storage-api-1.example.com
  - https://storage-api-2.example.com
```

**Example**:
```bash
OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH=/etc/event-consumer/permissions.yml
```

**In the YAML file** (`/etc/event-consumer/permissions.yml`):
```yaml
storage_permissions_endpoints:
  - https://azure-blob-storage-api.example.com
  - https://s3-storage-api.example.com
```

Currently, the first endpoint in the list is used. In the future, routing rules will determine which endpoint to use based on the event's `resource_id`.

### Storage Permissions with Durable Queues

**Important difference**: When using **durable queues**, storage events don't use the Storage API "docs" permissions.

Instead, they use simple action-based permissions:
- `storage.file.created` → `consume-all-storage-create-events`
- `storage.file.deleted` → `consume-all-storage-delete-events`

**Why?** Durable queues are typically used by services (not end users) that need to process all storage events, not just those for files they own.

**Policy Example**:
```
// Thumbnail service can consume all file creation events
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:consume-all-storage-create-events"
);

// Audit service can consume all file deletion events
permit(
  principal == Principal::"audit-service",
  action == Action::"event-consumer-service:consume-all-storage-delete-events"
);
```

## Fine-Grained Permissions for Arbitrary Event Types

For non-storage event types, you can configure **fine-grained permissions** that check not just the event type, but also the **resource_id** from the event.

### When to Use Fine-Grained Permissions

Use fine-grained permissions when:
- Different users should see different events of the same type
- Events relate to resources with their own access control (like files, projects, workspaces)
- You need to enforce permissions based on the event's `resource_id`

**Examples**:
- A user should only see `project.updated` events for projects they have access to
- A user should only see `thumbnail.generated` events for their own files
- A user should only see `notification.created` events for notifications sent to them

### Configuration

Fine-grained permissions are configured via a YAML file specified by the `OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH` environment variable.

**Environment Variable**:
```bash
OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH=/etc/event-consumer/permissions.yml
```

**YAML Configuration File**:
```yaml
event_permissions:
  - event_type: project.updated
    action: read-project
  - event_type: thumbnail.generated
    action: read-thumbnail
  - event_type: notification.created
    action: read-notification
```

### How It Works

For each event of a configured type:
1. The Event Consumer Service makes a permissions API call
2. It uses the configured `action`
3. It uses the event's `resource_id` as the resource ID
4. The resource type is always `EventResourceId`
5. If the principal has permission, the event is delivered; otherwise, it's filtered out

### Permissions API Call

When consuming an event like:
```json
{
  "event_type": "project.updated",
  "resource_id": "/projects/project-123",
  "message": {}
}
```

The service makes this permissions check:
- **Principal**: The authenticated user
- **Action**: `read-project` (from config)
- **Resource Type**: `EventResourceId`
- **Resource ID**: `/projects/project-123` (from event)

### Policy Examples

**Scoped Policy** (using resource_id):
```
// Allow user to see events for project-123
permit(
  principal == Principal::"user@example.com",
  action == Action::"event-consumer-service:read-project",
  resource == EventResourceId::"/projects/project-123"
);
```

**Global Policy** (ignoring resource_id):
```
// Allow admin to see all project events
permit(
  principal == Principal::"admin@example.com",
  action == Action::"event-consumer-service:read-project"
);
```

**Hierarchical Policy**:
```
// Allow user to see events for any project they own
permit(
  principal == Principal::"user@example.com",
  action == Action::"event-consumer-service:read-project",
  resource == EventResourceId
)
when {
  resource in Principal::"user@example.com"
};
```

### Complete Configuration Example

**YAML file** (`/etc/event-consumer/permissions.yml`):
```yaml
# Storage API endpoints for "docs" permissions
storage_permissions_endpoints:
  - https://storage-api.example.com

# Fine-grained event permissions
event_permissions:
  # Project events
  - event_type: project.created
    action: read-project
  - event_type: project.updated
    action: read-project
  - event_type: project.deleted
    action: read-project
  
  # Thumbnail events
  - event_type: thumbnail.generated
    action: read-thumbnail
  
  # Notification events  
  - event_type: notification.created
    action: read-notification
  
  # Workflow events
  - event_type: workflow.completed
    action: read-workflow
```

**Corresponding Policies**:
```
// User can see their own project events
permit(
  principal == Principal::"user@example.com",
  action in [
    Action::"event-consumer-service:read-project"
  ],
  resource == EventResourceId
)
when {
  // Check if resource (project) is owned by user
  resource.owner == principal.id
};

// User can see thumbnails for files they can access
permit(
  principal == Principal::"user@example.com",
  action == Action::"event-consumer-service:read-thumbnail",
  resource == EventResourceId
)
when {
  // Check via storage permissions
  resource.accessible_by(principal)
};

// All users can see notification events for their own notifications
permit(
  principal,
  action == Action::"event-consumer-service:read-notification",
  resource == EventResourceId
)
when {
  resource.recipient == principal.id
};
```

## Permissions Decision Flow

Here's how permissions are checked when consuming events:

### Non-Durable Queues

```
┌─────────────────────────────────────────┐
│ Event Received                          │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ Is it a storage event?                  │
│ (storage.created, storage.deleted, etc) │
└────────┬─────────────────────┬──────────┘
         │ Yes                 │ No
         ▼                     ▼
┌────────────────────┐  ┌──────────────────────────┐
│ Check Storage API  │  │ Is event_type configured │
│ "docs" permissions │  │ in permissions.yml?      │
└────────┬───────────┘  └──────┬──────────────┬────┘
         │                     │ Yes          │ No
         │                     ▼              ▼
         │            ┌────────────────┐  ┌────────────┐
         │            │ Check          │  │ Allow      │
         │            │ permissions    │  │ (no check) │
         │            │ API with       │  └────────────┘
         │            │ configured     │
         │            │ action +       │
         │            │ resource_id    │
         │            └────────┬───────┘
         │                     │
         ▼                     ▼
┌─────────────────────────────────────────┐
│ Allow or Deny?                          │
└────────┬─────────────────────┬──────────┘
         │ Allow               │ Deny
         ▼                     ▼
┌────────────────┐    ┌───────────────────┐
│ Deliver Event  │    │ Filter Out Event  │
└────────────────┘    └───────────────────┘
```

### Durable Queues

```
┌─────────────────────────────────────────┐
│ Event Received                          │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ Is it a storage event?                  │
└────────┬─────────────────────┬──────────┘
         │ Yes                 │ No
         ▼                     ▼
┌────────────────────┐  ┌──────────────────────────┐
│ Check:             │  │ Is event_type configured │
│ - storage.created  │  │ in permissions.yml?      │
│   → consume-all-   │  └──────┬──────────────┬────┘
│     storage-create │         │ Yes          │ No
│     -events        │         ▼              ▼
│ - storage.deleted  │  ┌────────────────┐ ┌───────┐
│   → consume-all-   │  │ Check perms    │ │ Allow │
│     storage-delete │  │ API with       │ └───────┘
│     -events        │  │ action +       │
└────────┬───────────┘  │ resource_id    │
         │              └────────┬───────┘
         │                       │
         ▼                       ▼
┌─────────────────────────────────────────┐
│ Allow or Deny?                          │
└────────┬─────────────────────┬──────────┘
         │ Allow               │ Deny
         ▼                     ▼
┌────────────────┐    ┌───────────────────┐
│ Deliver Event  │    │ Filter Out Event  │
└────────────────┘    └───────────────────┘
```

## Summary Table

| Scenario | Queue Type | Event Type | Permission Check |
|----------|-----------|------------|------------------|
| Create queue | Durable | N/A | `create-durable-queues` |
| Delete queue | Durable | N/A | `delete-durable-queues` |
| Consume from queue | Durable | N/A | `consume-durable-queues` |
| Storage create event | Durable | storage.* | `consume-all-storage-create-events` |
| Storage delete event | Durable | storage.* | `consume-all-storage-delete-events` |
| Storage event | Non-durable | storage.* | Storage API "docs" permissions |
| Configured event | Either | Any | Configured action + EventResourceId |
| Unconfigured event | Either | Any | No check (allowed) |
| Publishing | N/A | Any | `publish-event` with EventType |

## Configuration Examples

### Example 1: Thumbnail Service (Durable Queue)

The thumbnail service needs to process all file creation events.

**Permissions Policies**:
```
permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:create-durable-queues"
);

permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:consume-durable-queues"
);

permit(
  principal == Principal::"thumbnail-service",
  action == Action::"event-consumer-service:consume-all-storage-create-events"
);
```

**No YAML configuration needed** - the service uses simple action-based permissions.

### Example 2: User Dashboard (Non-Durable Queue)

A web dashboard shows users file events for files they have access to.

**Permissions Policies**:
```
// No special policies needed - users use their existing storage permissions
// The Event Consumer Service will query the Storage API to check access
```

**YAML Configuration** (`/etc/event-consumer/permissions.yml`):
```yaml
storage_permissions_endpoints:
  - https://storage-api.example.com
```

### Example 3: Project Notification Service (Non-Durable + Fine-Grained)

A service that shows users events for projects they have access to.

**Permissions Policies**:
```
permit(
  principal,
  action in [
    Action::"event-consumer-service:read-project"
  ],
  resource == EventResourceId
)
when {
  // User has access if they're a member of the project
  resource.members.contains(principal.id)
};
```

**YAML Configuration**:
```yaml
event_permissions:
  - event_type: project.created
    action: read-project
  - event_type: project.updated
    action: read-project
  - event_type: project.deleted
    action: read-project
  - event_type: project.member.added
    action: read-project
```

## Best Practices

### 1. Use Service Accounts for Durable Queues

Create dedicated service principals for services using durable queues:

```
permit(
  principal == Principal::"service:thumbnail-generator",
  action == Action::"event-consumer-service:create-durable-queues"
);
```

The durable-queue actions are deployment-wide rather than per-queue, so anyone granted them can reach any durable queue whose `queue_id` they know. Keep the grants on named service principals rather than on broad user groups, and give each service a namespaced `queue_id` (`<service-name>-<purpose>`) so queues stay attributable.

### 2. Document Your Permission Requirements

When defining new event types, document what permissions are needed:

```python
"""
Event Type: project.updated

Publishing Permission:
  - Action: publish-event
  - Resource: EventType::"project.updated"

Consuming Permission (fine-grained):
  - Action: read-project
  - Resource: EventResourceId (project's resource_id)
  
Example policy:
  permit(
    principal,
    action == Action::"event-consumer-service:read-project",
    resource == EventResourceId
  )
  when { resource.members.contains(principal.id) };
"""
```

### 3. Test Permissions Thoroughly

Test both positive and negative cases:
- Can authorized users publish/consume?
- Are unauthorized users properly denied?
- Do permissions work correctly after token refresh?
- Are fine-grained permissions enforced correctly?

### 4. Monitor Permission Denials

Log and monitor permission denials to detect:
- Misconfigured policies
- Users attempting unauthorized access
- Services missing required permissions

### 5. Use Hierarchical Policies Where Appropriate

For resource hierarchies, use hierarchical policies:

```
// Allow user to see events for anything in their workspace
permit(
  principal == Principal::"user@example.com",
  action == Action::"event-consumer-service:read-file",
  resource == EventResourceId
)
when {
  resource.path.startsWith("/workspaces/" + principal.workspace_id + "/")
};
```

## Troubleshooting

### "Permission denied" when publishing

**Check**:
1. Is `OMNI_EVENTS_PERMISSIONS_ENDPOINT` configured?
2. Does your principal have `publish-event` permission for the EventType?
3. Is your auth token valid and not expired?

### "Permission denied" when creating durable queue

**Check**:
1. Does your principal have `create-durable-queues` permission?
2. Is your auth token valid?

### Not receiving storage events (non-durable)

**Check**:
1. Is `storage_permissions_endpoints` configured correctly in the YAML file?
2. Does the Storage API show that the user has access to files in the relevant directories?
3. Are the event types correct? (Check the actual event type strings used by your storage service)

### Not receiving events with fine-grained permissions

**Check**:
1. Is the event_type configured in the `event_permissions` section of the YAML file?
2. Does your policy grant the configured action for the resource_id in the events?
3. Is the `resource_id` in the events formatted as expected?

### Token expiring during stream

**Solution**: Implement automatic token refresh and reconnection. See [Consuming Events - Error Handling](consuming-events.md#authorization-errors-during-streaming).

## Next Steps

- Review [Publishing Events](publishing-events.md) to understand how to publish with proper authorization
- Review [Consuming Events](consuming-events.md) to understand queue types and consumption patterns
- Check the [API Reference](api-reference.md) for complete API documentation

