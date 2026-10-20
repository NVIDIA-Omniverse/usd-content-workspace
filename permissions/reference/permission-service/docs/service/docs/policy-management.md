(permission-service-policy-management-managing-policies-through-the-rest-api)=
# Managing policies through the REST API

Permission Service exposes a REST API under `/v1beta/policies/` for managing Cedar policies at runtime. Use it to list, fetch, create, update, and delete the policies the service evaluates.

This API is specific to this service and is not part of the published Omniverse Storage Permission API — that contract covers only the authorization endpoints — so it is documented in full here. Runtime management requires **Postgres mode**; in **config-file** mode the store is read-only and every write is answered with `501 Not Implemented`.

For the Cedar syntax used in the `policy` field, see {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>`. For how policies are loaded from Helm or YAML at startup, see {ref}`Deployment <permission-service-deployment-permission-service-configuration>` and {ref}`Database configuration <permission-service-database-configuration-database-configuration>`.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(permission-service-policy-management-overview)=
## Overview

The policy management API is a CRUD surface over Cedar policy records:

| Method | Path                      | Purpose                                            |
|--------|---------------------------|----------------------------------------------------|
| GET    | `/v1beta/policies/`       | List policies with offset pagination and filters.  |
| GET    | `/v1beta/policies/{id}`   | Fetch a single policy by identifier.               |
| PUT    | `/v1beta/policies/`       | Add a policy, or update one with the same text.    |
| PUT    | `/v1beta/policies/batch/` | Add or update many policies in one request.        |
| DELETE | `/v1beta/policies/{id}`   | Remove a policy by identifier.                     |

Key behaviors that apply to all endpoints:

- **Storage backend.** Write operations require **Postgres mode**. In **config-file** mode the storage is read-only and write endpoints return **`501 Not Implemented`**. See {ref}`Deployment <permission-service-deployment-permission-service-configuration>` for how to select a backend.
- **Scopes are inferred from the Cedar text.** When you add a policy, the service parses the policy head and derives the optional **principal**, **group**, **action**, and **resource** scopes. You do not send these fields; any extra JSON fields in the payload are ignored. Scope inference rules are described in {ref}`Policy scopes <permission-service-policy-language-policy-scopes-principal-action-resource>`.
- **Writes are upserts.** Submitting a policy whose canonical text already exists updates that record in place instead of creating a duplicate. The single-policy endpoint signals which happened through its status code and the `created` field; the batched endpoint signals it per item.
- **Policy validation is enabled by default.** Every submitted policy is parsed as a single Cedar statement, and validation then checks it against the scopes inferred from its head before it is stored. Policies are not validated against registered service metadata, so a policy may reference actions and resource types that are not registered. Validation is controlled by the `POLICY_VALIDATION` environment variable, which defaults to `true` and is not exposed as a Helm value, so chart deployments run with validation on. Validation failures return **`400 Bad Request`**.
- **Resource ID normalization.** When a policy pins a resource (for example `resource == ResourceAddress::"https://example.com/file name.usd"`), the service URL-encodes the resource id before storage. The same encoding — and the {ref}`URL remapper <permission-service-deployment-url-remapper>`, when configured — is applied to the `resource` query parameter on `GET /v1beta/policies/`, so clients should pass the id in the same textual form the API returns.
- **Cache invalidation and events.** Successful writes invalidate the in-memory evaluation cache for the affected `(principal, action, resource)` tuple and, when notifications are enabled, publish a permission-change event.

(permission-service-policy-management-authentication-and-authorization)=
## Authentication and authorization

When authentication is enabled on the service, callers MUST include a valid bearer token:

```text
Authorization: Bearer <token>
```

Requests without a token receive **`401 Unauthorized`**. When authentication is disabled all endpoints are reachable without a token and the gates below are not evaluated.

Two **meta-permissions** gate access to the API:

- **View** — reading policies requires the caller to be authorized for `Action::"permissions:view"`.
- **Edit** — creating, updating, and deleting policies requires the caller to be authorized for `Action::"permissions:edit"`.

Both gates are evaluated using the service's own Cedar engine against the currently stored policies, and both evaluate against the **real target resource** (the resource governed by the edited policy for `permissions:edit`; the probed resource for `permissions:view`), so these grants can be scoped to a single resource or a folder subtree, and refined with `context`. See {ref}`Meta-permissions <permission-service-policy-language-meta-permissions>`.

Callers who lack the required meta-permission receive **`403 Forbidden`**. When the service bootstraps an empty database, a typical set of admin policies is seeded through `database.init.policies` (see {ref}`Granting administrative permissions <permission-service-deployment-granting-administrative-permissions>`) so that administrators can manage policies through this API.

(permission-service-policy-management-error-codes)=
## Error codes

All error responses carry a plain JSON string describing the failure.

| Status                      | Condition                                                                                                                                                                     |
|-----------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `400 Bad Request`           | An `action` or `resource` filter cannot be parsed; the Cedar text failed to parse; policy validation rejected the policy; a resource id cannot be encoded; or the storage backend rejected the record. In batch requests the message is prefixed with `batches.<index>:` to identify the offending item. |
| `401 Unauthorized`          | The bearer token is missing, invalid, or expired (authentication enabled).                                                                                                     |
| `403 Forbidden`             | The caller lacks `permissions:view` for a read, or `permissions:edit` for a write.                                                                                             |
| `404 Not Found`             | No policy exists with the requested identifier (`GET /v1beta/policies/{id}`).                                                                                                  |
| `422 Unprocessable Entity`  | A path or query parameter failed deserialization (a non-integer `id`, `page` below 1, `limit` outside 1–50), the JSON body failed to deserialize, `policy` exceeded 65 535 characters, `comment` exceeded 4 096 characters, or a batch carried more than 100 items. |
| `500 Internal Server Error` | A storage failure occurred while reading or writing policies.                                                                                                                  |
| `501 Not Implemented`       | The configured storage backend does not support writes (config-file mode).                                                                                                     |

(permission-service-policy-management-request-and-response-models)=
## Request and response models

(permission-service-policy-management-putpolicypayload)=
### PutPolicyPayload

Request body of `PUT /v1beta/policies/`, and each item of `PUT /v1beta/policies/batch/`.

| Field     | Type    | Required | Description                                                                                                                                                     |
|-----------|---------|----------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `policy`  | string  | yes      | Cedar policy text. MUST contain exactly one `permit` or `forbid` statement. Maximum length 65 535 characters.                                                    |
| `order`   | integer | no       | Evaluation order to assign to the stored policy. Defaults to the deployment-wide `config.defaultPolicyOrder` (`0` by default). Lower values are evaluated first. |
| `comment` | string  | no       | Free-form note describing what the policy does or who it was created for. Stored as metadata and never used during authorization. Maximum length 4 096 characters; surrounding whitespace is trimmed and an empty result is stored as unset. |

Any additional JSON fields in the body are silently ignored.

(permission-service-policy-management-policyresponse)=
### PolicyResponse

A stored policy, as returned by the read endpoints.

```json
{
  "id": 1,
  "order": 0,
  "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
  "resource": {
    "id": "Astronaut.usd",
    "type": "ResourceAddress",
    "data": null
  },
  "action": {
    "name": "get",
    "service": "tags"
  },
  "principal": {
    "sub": "test-user",
    "info": null
  },
  "created_at": "2026-04-16T12:34:56Z",
  "created_by": "",
  "comment": null
}
```

| Field             | Type    | Required | Description                                                                                                                                      |
|-------------------|---------|----------|--------------------------------------------------------------------------------------------------------------------------------------------------|
| `id`              | integer | yes      | Unique policy identifier, assigned by the service.                                                                                               |
| `order`           | integer | yes      | Evaluation order; lower values are evaluated first.                                                                                              |
| `policy`          | string  | yes      | The Cedar policy text (exactly one statement).                                                                                                   |
| `principal`       | object  | no       | Principal scope inferred from the policy head, or `null` when the head does not pin a principal. `sub` is the principal id; `info` carries additional claim data when present. |
| `group`           | object  | no       | Group scope inferred from a `principal in Group::"<id>"` head, as `{ "id": "<id>" }`. Absent when the head pins no group.                         |
| `action`          | object  | no       | Action scope inferred from the policy head, or `null` when unset.                                                                                |
| `resource`        | object  | no       | Resource scope inferred from the policy head, or `null` when unset. `data` is an optional map of resource attributes.                              |
| `resource_prefix` | object  | no       | Hierarchy scope of a `resource in <Type>::"<url>/"` policy. Mutually exclusive with `resource`, and absent for policies that pin a single resource. |
| `created_at`      | string  | yes      | RFC 3339 timestamp of when the policy was created.                                                                                               |
| `created_by`      | string  | yes      | Principal id of the caller that created the policy; an empty string when the service was running with authentication disabled.                     |
| `comment`         | string  | no       | Administrator-supplied note stored with the policy, or `null` when none was provided.                                                            |

(permission-service-policy-management-putpolicyresponse)=
### PutPolicyResponse

Response of `PUT /v1beta/policies/`. Carries every {ref}`PolicyResponse <permission-service-policy-management-policyresponse>` field plus:

| Field     | Type    | Required | Description                                                                                            |
|-----------|---------|----------|--------------------------------------------------------------------------------------------------------|
| `created` | boolean | yes      | `true` when the policy was newly inserted, `false` when an existing policy with the same canonical text was updated in place. |

(permission-service-policy-management-putpolicybatchresponse)=
### PutPolicyBatchResponse

Response of `PUT /v1beta/policies/batch/`.

| Field     | Type                                  | Required | Description                                                       |
|-----------|---------------------------------------|----------|-------------------------------------------------------------------|
| `results` | array of {ref}`PutPolicyResponse <permission-service-policy-management-putpolicyresponse>` | yes | One entry per submitted item, in request order. |

(permission-service-policy-management-policy-page)=
### Policy page

Response of `GET /v1beta/policies/`: one page of policies.

| Field        | Type                                | Required | Description                                                                            |
|--------------|-------------------------------------|----------|----------------------------------------------------------------------------------------|
| `items`      | array of {ref}`PolicyResponse <permission-service-policy-management-policyresponse>` | yes | Policies on this page.                                       |
| `page`       | integer                             | yes      | 1-based number of the returned page.                                                   |
| `page_size`  | integer                             | yes      | Number of items returned on this page.                                                 |
| `page_count` | integer                             | no       | Total number of pages when the backend can compute it; `null` for backends that do not report a total. |

(permission-service-policy-management-get-v1beta-policies-list-policies)=
## GET /v1beta/policies/ — list policies

Returns policies from the database using offset pagination, as a {ref}`policy page <permission-service-policy-management-policy-page>`. Optional query parameters narrow the result set by scope.

This is a **scope filter, not an authorization query**: global policies (`permit(principal, action, resource)`) are never attached to a specific resource and so are never returned by a resource-filtered listing. Use the authorization API to see the full evaluated set for a request, or the {ref}`Diagnostics API <permission-service-diagnostics-diagnostics-api>` to see the candidate policies for one request.

(permission-service-policy-management-query-parameters)=
### Query parameters

| Name        | Type    | Default | Description                                                                                                                                                      |
|-------------|---------|---------|------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `page`      | integer | `1`     | 1-based page number. MUST be `1` or greater.                                                                                                                     |
| `limit`     | integer | `10`    | Page size. MUST be between `1` and `50`.                                                                                                                         |
| `principal` | string  | —       | Filter by principal id, as a plain string (for example `test-user`). Use the literal string `NULL` to match policies whose principal scope is unset.              |
| `group`     | string  | —       | Filter by the group id of `principal in Group::"<id>"` policies, as a plain string. Use `NULL` to match policies whose group scope is unset.                      |
| `action`    | string  | —       | Filter by action in `service:name` form, for example `tags:get`. Use `NULL` to match policies whose action scope is unset.                                        |
| `resource`  | string  | —       | Filter by resource in `Type::id` form, for example `ResourceAddress::"Astronaut.usd"`. Use `NULL` to match policies whose resource scope is unset.                |
| `scope`     | string  | `exact` | Resource scope-matching mode. `exact` returns only policies scoped directly to `resource`; `inherited` also returns hierarchy policies whose prefix is an ancestor of `resource`. |
| `order`     | integer | —       | Filter by the exact evaluation order of the policy.                                                                                                              |

The `action` and `resource` filters are parsed from their string form and MUST be syntactically valid; values that cannot be parsed return **`400 Bad Request`**. When a filter is omitted, policies with **any** scope on that dimension (including unset) match.

With `scope=inherited`, the ancestor prefixes are derived from the resource type's `parentType` registered in metadata, falling back to the resource's own type when the type is unregistered or declares no parent. See {ref}`Parent type for resource hierarchies <permission-service-metadata-parent-type-for-resource-hierarchies>`.

(permission-service-policy-management-get-v1beta-policies-list-policies-example)=
### Example

```http
GET /v1beta/policies/?page=1&limit=10&action=tags:get HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "items": [ /* array of PolicyResponse */ ],
  "page": 1,
  "page_size": 10,
  "page_count": 3
}
```

(permission-service-policy-management-get-v1beta-policies-id-fetch-a-policy)=
## GET /v1beta/policies/{id} — fetch a policy

Returns the {ref}`PolicyResponse <permission-service-policy-management-policyresponse>` for a single policy, or `404 Not Found` when no policy has that identifier.

(permission-service-policy-management-get-v1beta-policies-id-fetch-a-policy-path-parameters)=
### Path parameters

| Name | Type    | Description                                       |
|------|---------|---------------------------------------------------|
| `id` | integer | Policy identifier, as returned by previous calls. |

(permission-service-policy-management-get-v1beta-policies-id-fetch-a-policy-example)=
### Example

```http
GET /v1beta/policies/42 HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
```

(permission-service-policy-management-put-v1beta-policies-add-a-policy)=
## PUT /v1beta/policies/ — add or update a policy

Stores a single Cedar policy supplied as a {ref}`PutPolicyPayload <permission-service-policy-management-putpolicypayload>`. The principal, group, action, and resource scopes are inferred from the Cedar text; there is no need (and no way) to send them explicitly.

The endpoint is an upsert keyed by the canonical policy text:

- **`201 Created`** — the policy was newly inserted; the {ref}`PutPolicyResponse <permission-service-policy-management-putpolicyresponse>` body reports `"created": true`.
- **`200 OK`** — an existing policy with the same canonical text was updated in place; the body reports `"created": false`.

(permission-service-policy-management-put-v1beta-policies-add-a-policy-example)=
### Example

```http
PUT /v1beta/policies/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

{
  "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
  "order": 10,
  "comment": "Grants the test user read access to the sample asset."
}
```

```http
HTTP/1.1 201 Created
Content-Type: application/json

{
  "id": 42,
  "order": 10,
  "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
  "resource": {"id": "Astronaut.usd", "type": "ResourceAddress", "data": null},
  "action": {"name": "get", "service": "tags"},
  "principal": {"sub": "test-user", "info": null},
  "created_at": "2026-04-16T12:34:56Z",
  "created_by": "admin-user",
  "comment": "Grants the test user read access to the sample asset.",
  "created": true
}
```

(permission-service-policy-management-put-v1beta-policies-batch-add-many-policies)=
## PUT /v1beta/policies/batch/ — add or update many policies

Batched version of the single-policy endpoint. The request body is a JSON array; each element is a {ref}`PutPolicyPayload <permission-service-policy-management-putpolicypayload>`. Scopes are inferred per item from the Cedar text.

The service validates every item up front. If any item fails validation or scope inference, the whole request is rejected before any policy is written. A single request MAY carry at most **100** items; larger arrays are rejected with `422 Unprocessable Entity`, so split the work across multiple requests on the caller side.

A successful request always returns **`200 OK`** with a {ref}`PutPolicyBatchResponse <permission-service-policy-management-putpolicybatchresponse>`; each item's `created` field reports whether that policy was inserted or updated in place. An empty array is accepted and produces an empty result without contacting the storage backend.

(permission-service-policy-management-put-v1beta-policies-batch-add-many-policies-example)=
### Example

```http
PUT /v1beta/policies/batch/ HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
Content-Type: application/json

[
  {
    "policy": "permit(principal == Principal::\"test-user-1\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
  },
  {
    "policy": "permit(principal == Principal::\"test-user-2\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
    "order": 5
  }
]
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "results": [ /* array of PutPolicyResponse, in the same order as the request */ ]
}
```

(permission-service-policy-management-delete-v1beta-policies-id-delete-a-policy)=
## DELETE /v1beta/policies/{id} — delete a policy

Removes a Cedar policy from the database. The endpoint is idempotent: deleting a policy that does not exist still returns **`204 No Content`**, and no change event is published because nothing was removed.

The `permissions:edit` gate is evaluated against the stored policy's own resource scope — canonicalized the same way the authorization path canonicalizes a request — so an edit grant scoped to a resource or to its folder hierarchy matches a policy stored under an alias of that URL.

(permission-service-policy-management-delete-v1beta-policies-id-delete-a-policy-path-parameters)=
### Path parameters

| Name | Type    | Description         |
|------|---------|---------------------|
| `id` | integer | Policy identifier.  |

(permission-service-policy-management-delete-v1beta-policies-id-delete-a-policy-example)=
### Example

```http
DELETE /v1beta/policies/42 HTTP/1.1
Host: permission-service.storage-apis.svc.cluster.local:3000
Authorization: Bearer <token>
```

```http
HTTP/1.1 204 No Content
```

(permission-service-policy-management-change-notifications)=
## Change notifications

When notifications are enabled, every successful write (`PUT /v1beta/policies/`, `PUT /v1beta/policies/batch/`, and `DELETE /v1beta/policies/{id}`) causes the service to publish a permission-change event to the configured **Event Aggregation Service**. Each event identifies the `(principal, action, resource)` tuple whose effective policies may have changed, so downstream services can invalidate their own caches or otherwise react to the update. Failed or rejected writes do not emit events.

Notifications are best-effort and do not block the API response: a caller receives its success status as soon as the policy is persisted, and the event is dispatched asynchronously.

For the event schema, delivery guarantees, and how to configure the Event Aggregation Service endpoint, see {ref}`Service notifications <permission-service-notifications-service-notifications>`.

(permission-service-policy-management-interactive-api-reference)=
## Interactive API reference

A running service publishes its own OpenAPI definition at `/openapi.json` and serves an interactive API browser at the root path (`/`) on the REST port (default `3000`). Both reflect the deployed build, so they are the quickest way to confirm the exact payloads a particular version accepts.

(permission-service-policy-management-see-next)=
## See next

- {ref}`Managing service metadata through the REST API <permission-service-metadata-managing-service-metadata-through-the-rest-api>` — the catalog of services, actions, and resource types the scopes above refer to.

Related pages:

- {ref}`Writing Cedar policies for Permission Service <permission-service-policy-language-writing-cedar-policies-for-permission-service>` — syntax, scopes, and conditions.
- {ref}`How authorization requests are evaluated <permission-service-policy-evaluation-how-authorization-requests-are-evaluated>` — how `order` and resource evaluation priority drive the decision for stored policies.
- {ref}`Deployment <permission-service-deployment-permission-service-configuration>` — selecting Postgres or config-file mode and seeding initial policies.
- {ref}`Database configuration <permission-service-database-configuration-database-configuration>` — on-disk policy format and scope inference details.
- {ref}`Policy management metrics <permission-service-observability-policy-management-metrics>` — counters and histograms emitted by these endpoints.
