(permission-service-metadata-managing-service-metadata-through-the-rest-api)=
# Managing service metadata through the REST API

Permission Service tracks **service metadata** — the catalog of services that integrate with authorization, the **actions** each service exposes, and the **resource types** those actions operate on. This catalog is exposed as a REST API under `/v1beta/services/` so that clients and management tools can discover and update it at runtime.

This API is specific to this service and is not part of the published Omniverse Storage Permission API — that contract covers only the authorization endpoints — so it is documented in full here. Runtime writes require **Postgres mode**; in **config-file** mode the catalog is read-only.

Metadata is primarily **reference information** used to understand other service operations. Callers rarely need it to evaluate a request; instead, the {ref}`Permission Web UI <permission-service-web-ui-permission-web-ui>` uses it to drive autocomplete when composing Cedar policies, so policy authors can pick a known service, one of its registered actions, and one of its registered resource types instead of typing identifiers by hand. The API described on this page is what the UI (and any equivalent tooling) calls to populate those suggestions.

A small number of metadata fields **are** consumed by the service itself at authorization time:

- **`id_claim`** on a service record selects which claim from the caller's token is used as the principal id for requests that target that service. See {ref}`idClaim for principals <permission-service-metadata-idclaim-for-principals>` below.
- **`evaluation_priority`** on a resource type record controls how competing policies are combined for resources of that type. See {ref}`Evaluation priority for resources <permission-service-metadata-evaluation-priority-for-resources>` below.
- **`parent_type`** on a resource type record names the resource type used for the URL folders that contain a resource, so `resource in <parentType>::"<url>/"` hierarchy policies match. See {ref}`Parent type for resource hierarchies <permission-service-metadata-parent-type-for-resource-hierarchies>` below.

All other metadata — the list of services, the names of actions, the set of resource types — is advisory: it is not required for a policy to be stored or evaluated, and the service does not reject unknown services, actions, or resource types at authorization time.

When notifications are enabled and the event publisher is connected, every successful write through these endpoints publishes an `omni.permissions.meta.changed` notification so that downstream consumers can react to catalog changes; otherwise the write still succeeds without publishing. See {ref}`Service notifications <permission-service-notifications-service-notifications>` for how to enable and connect the publisher.

For how policies are written and evaluated, see {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>`. For how metadata is seeded from Helm or YAML at startup, see {ref}`Deployment <permission-service-deployment-permission-service-configuration>` and {ref}`Database configuration <permission-service-database-configuration-database-configuration>`.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(permission-service-metadata-overview)=
## Overview

The metadata API is a CRUD surface organized around three nested records — services, their actions, and their resource types:

| Method | Path                                                | Purpose                                                    |
|--------|-----------------------------------------------------|------------------------------------------------------------|
| GET    | `/v1beta/services/`                                 | List all registered services.                              |
| GET    | `/v1beta/services/{service}/`                       | Fetch the record for one service.                          |
| PUT    | `/v1beta/services/{service}/`                       | Create or update the record for one service.               |
| DELETE | `/v1beta/services/{service}/`                       | Remove the record for one service.                         |
| GET    | `/v1beta/services/{service}/actions/`               | List registered actions for a service.                     |
| PUT    | `/v1beta/services/{service}/actions/`               | Replace the full set of actions for a service.             |
| PUT    | `/v1beta/services/{service}/actions/{action}/`      | Add or update a single action.                             |
| DELETE | `/v1beta/services/{service}/actions/{action}/`      | Remove a single action.                                    |
| GET    | `/v1beta/services/{service}/resource-types/`        | List registered resource types for a service.              |
| PUT    | `/v1beta/services/{service}/resource-types/`        | Replace the full set of resource types for a service.      |
| GET    | `/v1beta/services/{service}/resource-types/{type}/` | Fetch the record for one resource type.                    |
| PUT    | `/v1beta/services/{service}/resource-types/{type}/` | Create or update the record for one resource type.         |
| DELETE | `/v1beta/services/{service}/resource-types/{type}/` | Remove the record for one resource type.                   |

Key behaviors that apply to all endpoints:

- **Storage backend.** Write operations require **Postgres mode**. In **config-file** mode the storage is read-only and write endpoints return **`501 Not Implemented`**. Read endpoints are available in both modes. See {ref}`Deployment <permission-service-deployment-permission-service-configuration>` for how to select a backend.
- **Path parameters win over the body.** The `service` value — and, where the path carries one, the resource type — is taken from the URL and overwrites any matching field in the JSON body before the record is stored. Clients can safely omit those fields.
- **Collection writes replace the whole set.** The two collection endpoints delete every existing action or resource type for the service and store the submitted list in its place; an empty list clears the collection. The single-record endpoints add or update one entry and leave the rest untouched.
- **Deletes are idempotent.** Removing a service, action, or resource type that is not registered still reports success.
- **Cache invalidation.** Successful writes invalidate the in-memory metadata and evaluation caches for the affected service so that subsequent reads and authorization requests see the new state immediately.

(permission-service-metadata-seeding-metadata-at-deployment-time)=
### Seeding metadata at deployment time

Instead of (or in addition to) calling this API at runtime, metadata can be populated at deployment time through the Helm value **`database.init.services`**. Each entry describes one service and optionally its principal `idClaim`, its registered actions, and its resource types with evaluation priorities. The chart renders the list into a ConfigMap and mounts it at `/etc/permission-config/` so the service can load it on startup. See also {ref}`Metadata file format <permission-service-database-configuration-metadata-file-format>`.

How the seed is applied depends on the storage backend:

- **Config-file mode** — the rendered file is the read-only metadata store; the only way to change metadata in this mode is to update the Helm value and redeploy. The service reloads the file automatically when its modification time changes.
- **Postgres mode** — seeded entries are written into the database on startup, and can be changed afterwards through the REST API described on this page.

The example below seeds the four standard Omniverse services — `storage-service`, `event-consumer-service`, `event-aggregation-service`, and `userinfo` — with the actions and resource types listed in {ref}`Actions and resources for Omniverse services <permission-service-policy-language-actions-and-resources-for-omniverse-services>`:

```yaml
database:
  init:
    services:
      - name: "storage-service"
        principal:
          idClaim: "sub"
        actions:
          - "read"
          - "write"
        resourceTypes:
          - type: "object"
            evaluationPriority: "permit"
            parentType: "folder"
          - type: "folder"
            evaluationPriority: "permit"

      - name: "event-consumer-service"
        actions:
          - "consume-all-storage-create-events"
          - "consume-all-storage-delete-events"
          - "consume-durable-queues"
          - "create-durable-queues"
          - "delete-durable-queues"
          - "inspect-and-replay-all-dlq"

      - name: "event-aggregation-service"
        actions:
          - "publish-event"
        resourceTypes:
          - type: "EventType"
            evaluationPriority: "forbid"

      - name: "userinfo"
        actions:
          - "list-users"
          - "get-user"
          - "list-user-groups"
          - "get-user-group"
          - "list-groups"
          - "get-group"
          - "list-group-members"
          - "get-group-member"
        resourceTypes:
          - type: "User"
          - type: "Group"
```

A few notes on the schema:

- `principal.idClaim` is optional. When it is omitted, the service falls back to the deployment-wide claim set by the Helm value `openId.principalIdClaim` (default `sub`). See {ref}`idClaim for principals <permission-service-metadata-idclaim-for-principals>`.
- `actions` are plain names; they are referenced from Cedar policies as `Action::"<service>:<name>"`.
- `resourceTypes[].evaluationPriority` is optional and defaults to `"forbid"`. See {ref}`Evaluation priority for resources <permission-service-metadata-evaluation-priority-for-resources>`.
- `resourceTypes[].parentType` is optional. It names the resource type of the URL folders that contain resources of this type and enables `resource in <parentType>::"<url>/"` hierarchy policies (above, `object`s are contained by `folder`s). When omitted, a resource's folders are typed with its own type. See {ref}`Parent type for resource hierarchies <permission-service-metadata-parent-type-for-resource-hierarchies>`.
- A service that registers actions only (like `event-consumer-service` above) simply omits `resourceTypes`; Cedar policies for those actions leave the `resource` slot unconstrained.
- The `Group` resource type registered above (under `userinfo`) is a **resource being acted upon** — a group record the `userinfo` APIs read or list. It is unrelated to the `Group::"<id>"` entities used to model the **caller's** group membership in policy heads (`principal in Group::"<id>"`); see {ref}`Groups and membership <permission-service-policy-language-groups-and-membership>`.

(permission-service-metadata-authentication-and-authorization)=
## Authentication and authorization

When authentication is enabled on the service, callers MUST include a valid bearer token:

```text
Authorization: Bearer <token>
```

Requests without a token receive **`401 Unauthorized`**. When authentication is disabled all endpoints are reachable without a token and the gates below are not evaluated.

Every endpoint on this page is gated by one of two **meta-permissions**, split by whether the request reads or modifies metadata:

- **Meta read** — the `GET` endpoints require the caller to be authorized for `Action::"permissions:meta:read"`.
- **Meta write** — the `PUT` and `DELETE` endpoints require the caller to be authorized for `Action::"permissions:meta:write"`.

Both gates are evaluated using the service's own Cedar engine against the currently stored policies. Callers who lack the relevant meta-permission receive **`403 Forbidden`**. When the service bootstraps an empty database, a typical set of admin policies is seeded through `database.init.policies` (see {ref}`Granting administrative permissions <permission-service-deployment-granting-administrative-permissions>`) so that administrators can manage metadata through this API.

(permission-service-metadata-error-codes)=
## Error codes

All error responses carry a plain JSON string describing the failure.

| Status                      | Condition                                                                                      |
|-----------------------------|------------------------------------------------------------------------------------------------|
| `401 Unauthorized`          | The bearer token is missing, invalid, or expired (authentication enabled).                      |
| `403 Forbidden`             | The caller lacks `permissions:meta:read` for a read, or `permissions:meta:write` for a write.   |
| `404 Not Found`             | The requested service or resource type is not registered (single-record `GET` endpoints).       |
| `422 Unprocessable Entity`  | The JSON body does not match the expected shape.                                                |
| `500 Internal Server Error` | A storage failure occurred while reading or writing metadata.                                   |
| `501 Not Implemented`       | The configured storage backend does not support the operation (config-file mode for any write). |

(permission-service-metadata-records)=
## Records

Three record shapes appear in request and response bodies.

(permission-service-metadata-servicemeta)=
### ServiceMeta

One per registered service.

```json
{
  "service": "storage-service",
  "id_claim": "email"
}
```

| Field      | Type   | Required | Description                                                                                                                                                                  |
|------------|--------|----------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `service`  | string | yes      | Service name. Matches the `service` segment in the URL.                                                                                                                       |
| `id_claim` | string | no       | Claim from the caller's token used as the principal id for requests targeting this service. An empty string means "unset"; the alias `idClaim` is accepted on input. See {ref}`idClaim for principals <permission-service-metadata-idclaim-for-principals>`. |

(permission-service-metadata-action)=
### Action

One per registered action.

```json
{
  "name": "read",
  "service": "storage-service"
}
```

| Field     | Type   | Required | Description                                                                                                    |
|-----------|--------|----------|----------------------------------------------------------------------------------------------------------------|
| `name`    | string | yes      | Action name, up to 255 characters. Used as the `<name>` portion of `Action::"<service>:<name>"` in Cedar policies. |
| `service` | string | yes      | Service the action belongs to. Always matches the `service` segment in the URL.                                 |

(permission-service-metadata-resourcetypemeta)=
### ResourceTypeMeta

One per registered resource type.

```json
{
  "service": "storage-service",
  "type": "object",
  "evaluation_priority": "forbid",
  "parent_type": "folder"
}
```

| Field                 | Type   | Required | Description                                                                                                                                                                                 |
|-----------------------|--------|----------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `service`             | string | yes      | Service the resource type belongs to. Matches the `service` segment in the URL.                                                                                                              |
| `type`                | string | yes      | Resource type name. Used as the Cedar entity type (for example `object::"<id>"`).                                                                                                            |
| `evaluation_priority` | string | no       | One of `"forbid"` (default) or `"permit"`. Controls how competing policies are combined for this resource type. The alias `evaluationPriority` is accepted on input. See {ref}`Evaluation priority for resources <permission-service-metadata-evaluation-priority-for-resources>`. |
| `parent_type`         | string | no       | Resource type of the URL folders that contain resources of this type; enables `resource in <parentType>::"<url>/"` hierarchy policies. When unset, those folders use this type's own name. The alias `parentType` is accepted on input. See {ref}`Parent type for resource hierarchies <permission-service-metadata-parent-type-for-resource-hierarchies>`. |

(permission-service-metadata-get-v1beta-services-list-services)=
## GET /v1beta/services/ — list services

Returns every service registered in the database, as an array of {ref}`ServiceMeta <permission-service-metadata-servicemeta>` records. The array is empty when no service is registered.

(permission-service-metadata-get-v1beta-services-list-services-example)=
### Example

```http
GET /v1beta/services/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

[
  {"service": "storage-service", "id_claim": "sub"},
  {"service": "userinfo", "id_claim": "email"}
]
```

(permission-service-metadata-get-v1beta-services-service-fetch-service-metadata)=
## GET /v1beta/services/{service}/ — fetch service metadata

Returns the {ref}`ServiceMeta <permission-service-metadata-servicemeta>` for a single service, or `404 Not Found` when no service with that name is registered.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-update-service-metadata)=
## PUT /v1beta/services/{service}/ — update service metadata

Creates the service record if it does not exist, or updates it in place. The `service` field of the payload is overwritten with the URL path segment, so the body effectively only needs to carry `id_claim`. Returns the stored {ref}`ServiceMeta <permission-service-metadata-servicemeta>`.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-update-service-metadata-example)=
### Example

```http
PUT /v1beta/services/storage-service/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

{
  "id_claim": "email"
}
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "service": "storage-service",
  "id_claim": "email"
}
```

(permission-service-metadata-delete-v1beta-services-service-delete-service-metadata)=
## DELETE /v1beta/services/{service}/ — delete service metadata

Removes the service record, along with its actions and resource types in storage backends that cascade deletes. Returns `204 No Content`, also when no service with that name existed.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-get-v1beta-services-service-actions-list-actions)=
## GET /v1beta/services/{service}/actions/ — list actions

Returns every action registered for the service, as an array of {ref}`Action <permission-service-metadata-action>` records. The array is empty when none are registered.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-actions-replace-all-actions)=
## PUT /v1beta/services/{service}/actions/ — replace all actions

Replaces the complete set of actions registered for the service: all existing actions for `{service}` are deleted and replaced with the supplied list. The `service` field on every item is overwritten with the URL path segment, so clients can omit it. An empty array is accepted and clears all actions for the service. Returns `200 OK` with an empty body.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-actions-replace-all-actions-example)=
### Example

```http
PUT /v1beta/services/storage-service/actions/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

[
  { "name": "read" },
  { "name": "write" }
]
```

(permission-service-metadata-put-v1beta-services-service-actions-action-add-or-update-a-single-action)=
## PUT /v1beta/services/{service}/actions/{action}/ — add or update a single action

Registers a single action for the service, or leaves it in place when it already exists. The request has no body; the action name is taken from the URL. Returns the stored {ref}`Action <permission-service-metadata-action>`.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |
| `action`       | string | Action name.  |

(permission-service-metadata-delete-v1beta-services-service-actions-action-delete-an-action)=
## DELETE /v1beta/services/{service}/actions/{action}/ — delete an action

Removes a single action from the service. Returns `204 No Content`, also when no such action existed.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |
| `action`       | string | Action name.  |

(permission-service-metadata-get-v1beta-services-service-resource-types-list-resource-types)=
## GET /v1beta/services/{service}/resource-types/ — list resource types

Returns every resource type registered for the service, as an array of {ref}`ResourceTypeMeta <permission-service-metadata-resourcetypemeta>` records. The array is empty when none are registered.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-resource-types-replace-all-resource-types)=
## PUT /v1beta/services/{service}/resource-types/ — replace all resource types

Replaces the complete set of resource types registered for the service: all existing resource types for `{service}` are deleted and replaced with the supplied list. The `service` field on every item is overwritten with the URL path segment. An empty array is accepted and clears all resource types for the service. Returns `200 OK` with an empty body.

| Path parameter | Type   | Description   |
|----------------|--------|---------------|
| `service`      | string | Service name. |

(permission-service-metadata-put-v1beta-services-service-resource-types-replace-all-resource-types-example)=
### Example

```http
PUT /v1beta/services/storage-service/resource-types/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

[
  { "type": "object", "evaluation_priority": "forbid", "parent_type": "folder" },
  { "type": "folder", "evaluation_priority": "permit" }
]
```

(permission-service-metadata-get-v1beta-services-service-resource-types-type-fetch-resource-type-metadata)=
## GET /v1beta/services/{service}/resource-types/{type}/ — fetch resource type metadata

Returns the {ref}`ResourceTypeMeta <permission-service-metadata-resourcetypemeta>` for a single resource type, or `404 Not Found` when it is not registered for this service.

| Path parameter | Type   | Description         |
|----------------|--------|---------------------|
| `service`      | string | Service name.       |
| `type`         | string | Resource type name. |

(permission-service-metadata-put-v1beta-services-service-resource-types-type-update-resource-type-metadata)=
## PUT /v1beta/services/{service}/resource-types/{type}/ — update resource type metadata

Creates the resource type record if it does not exist, or updates it in place. The `service` and `type` fields of the payload are overwritten with the URL path segments, so the body effectively only needs to carry `evaluation_priority` and, for hierarchical resource types, `parent_type`. Returns the stored {ref}`ResourceTypeMeta <permission-service-metadata-resourcetypemeta>`.

| Path parameter | Type   | Description         |
|----------------|--------|---------------------|
| `service`      | string | Service name.       |
| `type`         | string | Resource type name. |

(permission-service-metadata-put-v1beta-services-service-resource-types-type-update-resource-type-metadata-example)=
### Example

```http
PUT /v1beta/services/storage-service/resource-types/object/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

{
  "evaluation_priority": "forbid",
  "parent_type": "folder"
}
```

(permission-service-metadata-delete-v1beta-services-service-resource-types-type-delete-resource-type-metadata)=
## DELETE /v1beta/services/{service}/resource-types/{type}/ — delete resource type metadata

Removes a single resource type from the service. Returns `204 No Content`, also when no such resource type existed.

| Path parameter | Type   | Description         |
|----------------|--------|---------------------|
| `service`      | string | Service name.       |
| `type`         | string | Resource type name. |

(permission-service-metadata-evaluation-priority-for-resources)=
## Evaluation priority for resources

Each registered resource type carries an **evaluation priority** that controls how competing Cedar decisions are combined for requests targeting a resource of that type:

- **`forbid`** (default) — if any matching policy evaluates to `forbid`, the request is denied. This is Cedar's standard "deny overrides" behavior and is the safer default for resources where an explicit deny should always win.
- **`permit`** — if any matching policy evaluates to `permit`, the request is allowed, even when another policy evaluates to `forbid`. This is useful for resource types where permits are intended to be additive exceptions.

Evaluation priority only affects resource-scoped requests; requests with no resource follow the service-wide default. For a complete description of how priorities interact with policy scopes and Cedar decisions, see {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>`.

(permission-service-metadata-parent-type-for-resource-hierarchies)=
## Parent type for resource hierarchies

A resource type may declare a **parent type** through the optional `parent_type` field (alias `parentType`). It names the resource type of the URL folders that contain resources of this type, and it is what makes `resource in <parentType>::"<url>/"` hierarchy policies match.

When a request targets a resource whose id is a URL — for example `https://main/docs/report.pdf` — the service treats each containing folder (`https://main/`, `https://main/docs/`, and so on) as an entity of the parent type. A policy scoped to that parent type and a folder url then matches the resource and every resource nested under that folder. For example, registering `object` with `parentType: "folder"` lets a policy scoped to `folder::"https://main/docs/"` grant access to all `object`s under `https://main/docs/`.

When `parent_type` is unset, a resource's folders are typed with the resource's **own** type. For how these hierarchy policies are written, see {ref}`Resource hierarchies <permission-service-policy-language-resource-hierarchies>` in the Cedar policy guide.

(permission-service-metadata-idclaim-for-principals)=
## idClaim for principals

The `id_claim` field on a service record selects **which claim from the caller's bearer token is used as the principal id** when the caller targets that service. When unset, the service falls back to the deployment-wide claim set by the Helm value `openId.principalIdClaim` (default `sub`). Different services in the same deployment can therefore identify the same user by different ids — for example, `email` for one service and `sub` for another.

For the full resolution order (per-service `idClaim`, deployment-wide `openId.principalIdClaim`, `sub` fallback) and for the principal attributes this affects, see {ref}`Principal <permission-service-policy-language-principal>` in the Cedar policy guide.

(permission-service-metadata-interactive-api-reference)=
## Interactive API reference

A running service publishes its own OpenAPI definition at `/openapi.json` and serves an interactive API browser at the root path (`/`) on the REST port (default `3000`). Both reflect the deployed build, so they are the quickest way to confirm the exact payloads a particular version accepts.

(permission-service-metadata-see-next)=
## See next

- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — where `idClaim`, `evaluationPriority`, and `parentType` are consumed in the evaluation pipeline.

Related pages:

- {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>` — how `idClaim` and resource types are referenced from policies.
- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` — companion API for the policies whose scopes reference these services, actions, and resource types.
- {ref}`Database configuration <permission-service-database-configuration-database-configuration>` — metadata file format for config-file mode.
- {ref}`Deployment <permission-service-deployment-permission-service-configuration>` — seeding initial metadata through Helm.
