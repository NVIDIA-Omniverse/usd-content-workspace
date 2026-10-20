(permission-api-rest-api-rest-api)=
# REST API

The Permission Service exposes an HTTP/JSON API for checking authorization decisions. It provides two endpoints:

- `POST /v1beta/authorization/` – checks whether a principal is allowed to perform a single action on a resource;
- `POST /v1beta/authorization/batch/` – checks whether a principal is allowed to perform several actions in one request.

Both endpoints return HTTP `200` for any request that is evaluated; the authorization outcome is carried in the response body, not in the status code. Failures that prevent evaluation are reported with the error status codes listed in {ref}`Error codes <permission-api-rest-api-error-codes>`, whose body is always an {ref}`ErrorString <permission-api-rest-api-model-error-string>`.

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED",  "MAY", and "OPTIONAL" in this section are to be interpreted as described in [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

(permission-api-rest-api-authentication)=
## Authentication

Callers SHOULD authenticate with the `Authorization` header in the `Bearer <token>` format. When authentication is enabled, the token MUST be a valid credential accepted by the service (by default a JWT issued by the Identity Provider for a user or a service). Requests with a missing, invalid, or expired token MUST be rejected with `401`.

An implementation MAY support other authentication mechanisms (for example Basic Auth, API keys, or SAML2) and MAY change how authentication information is passed. When authentication is disabled, the service MAY process requests without credentials.

The underlying authorization system MAY allow a caller to check permissions for a `principal` different from the authenticated caller (for example an administrator checking another user). When this is not permitted, the service MUST reject the request with `403`.

(permission-api-rest-api-error-codes)=
## Error codes

The following status codes report failures that prevent a request from being evaluated. The response body for every error is an {ref}`ErrorString <permission-api-rest-api-model-error-string>`.

| HTTP status | Condition |
|-------------|-----------|
| `400 Bad Request` | The payload was parsed but contains invalid information (for example a missing required field, duplicate `(service, name)` pairs within a batch's `actions` array, or a field that violates its constraints). |
| `401 Unauthorized` | Authentication credentials are missing, invalid, or expired. |
| `403 Forbidden` | The caller is not allowed to check authorization for the specified payload. |
| `429 Too Many Requests` | The client is rate limited and MUST slow down. The service MAY include a `Retry-After` header. |
| `500 Internal Server Error` | An unexpected server error occurred. |

(permission-api-rest-api-response-models)=
## Request and response models

(permission-api-rest-api-model-principal)=
### Principal

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `sub` | string | Yes | Unique principal identifier (the `sub` claim from the token). Max 255 characters; MUST NOT contain control characters, `"`, `'`, or `\`. |
| `info` | object | No | Additional claims carried in the principal token. |

(permission-api-rest-api-model-action)=
### Action

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Operation name. Max 255 characters; pattern `^[a-zA-Z0-9_.:/-]+$`. |
| `service` | string | Yes | Name of the service where the operation is performed. Max 255 characters; pattern `^[a-zA-Z0-9_.:/-]+$`. |

(permission-api-rest-api-model-resource)=
### Resource

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique resource identifier. Max 2048 characters; MUST NOT contain control characters, `"`, or `\`. |
| `type` | string | Yes | Resource classification used for policy evaluation. Max 255 characters; pattern `^[a-zA-Z0-9_.:/-]+$`. |
| `data` | object | No | Additional resource information that MAY be used to evaluate the authorization policy. |

(permission-api-rest-api-model-authorization-payload)=
### AuthorizationPayload

Request body of `POST /v1beta/authorization/`.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `action` | {ref}`Action <permission-api-rest-api-model-action>` | Yes | The operation to authorize. |
| `principal` | {ref}`Principal <permission-api-rest-api-model-principal>` | No | The identity being authorized. MAY be omitted when it is the same as the authenticated token payload. |
| `resource` | {ref}`Resource <permission-api-rest-api-model-resource>` | No | The target of the operation. |
| `context` | object | No | Additional request information (for example caller IP or geolocation) that the authorization policy MAY use. |

(permission-api-rest-api-model-authorization-response)=
### AuthorizationResponse

Result of a single authorization check. No fields other than those below are permitted.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `decision` | string | Yes | One of `allow`, `deny`, or `skip`. |
| `reason` | string \| null | No | Explanation for an explicit denial. Omitted for implicit denials (no matching rule). Max 2048 characters; pattern `^[\x20-\x7E]+$`. |
| `action` | string \| null | No | Echoes the request action name. Max 256 characters; pattern `^[a-zA-Z0-9_.:/-]+$`. |
| `service` | string \| null | No | Echoes the request service name. Max 256 characters; pattern `^[a-zA-Z0-9_.:/-]+$`. |

(permission-api-rest-api-model-authorization-batch-request)=
### AuthorizationBatchRequest

Request body of `POST /v1beta/authorization/batch/`.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `batches` | array of {ref}`AuthorizationBatch <permission-api-rest-api-model-authorization-batch>` | Yes | The batches to evaluate. Max 100 items. |
| `condition` | string \| null | No | One of `none`, `or`, or `and`. Controls short-circuiting and the summary. Defaults to `none` when omitted or null. |

(permission-api-rest-api-model-authorization-batch)=
### AuthorizationBatch

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `actions` | array of {ref}`Action <permission-api-rest-api-model-action>` | Yes | The operations to authorize against the resource. Max 1000 items. Within a batch, each entry MUST have a distinct `(service, name)` pair; duplicate pairs make the keyed `decisions` object ambiguous and MUST be rejected with `400 Bad Request`. |
| `principal` | {ref}`Principal <permission-api-rest-api-model-principal>` | No | The identity being authorized. MAY be omitted when it is the same as the authenticated token payload. |
| `resource` | {ref}`Resource <permission-api-rest-api-model-resource>` | No | The target of the operations. |
| `context` | object | No | Additional request information that the authorization policy MAY use. |

(permission-api-rest-api-model-authorization-batch-response)=
### AuthorizationBatchResponse

Result of a batch authorization check.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `decisions` | array of object | Yes | One object per batch, in request order. Each object is a map keyed by `<service>:<action>` (one key per action in that batch's `actions` array) whose value is an {ref}`AuthorizationResponse <permission-api-rest-api-model-authorization-response>`. Max 100 items. |
| `summary` | {ref}`AuthorizationResponse <permission-api-rest-api-model-authorization-response>` \| null | No | Overall decision. Present only when `condition` is `or` or `and`; MUST be omitted (or null) for `none`. |

(permission-api-rest-api-model-error-string)=
### ErrorString

The body of every error response is a plain JSON string (not an object). Max 2048 characters; pattern `^[\x20-\x7E\r\n]+$`. For example:

```json
"The principal token is expired."
```

(permission-api-rest-api-post-v1beta-authorization)=
## POST /v1beta/authorization/ — Check a single permission

Checks whether the specified principal is allowed to perform an action on a resource.

**Request body:** {ref}`AuthorizationPayload <permission-api-rest-api-model-authorization-payload>`.

The service returns `200` with an {ref}`AuthorizationResponse <permission-api-rest-api-model-authorization-response>`: `allow` when a rule matches, otherwise `deny`. An explicit denial SHOULD populate `reason`; an implicit denial (no matching rule) SHOULD omit it. When present, `action` and `service` MUST echo the request values.

The `context` field is OPTIONAL and MAY be omitted; an implementation MAY ignore it or use it during evaluation.

Request example:

```http
POST /v1beta/authorization/ HTTP/1.1
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
  "principal": {
    "sub": "DdxA9xDiqdUbv",
    "info": {"email": "user@test.com", "exp": 1727821346329}
  },
  "action": {
    "name": "read",
    "service": "storage"
  },
  "resource": {
    "id": "/Projects/Scene.usd",
    "type": "File",
    "data": {
      "metadata": {"size": 1024}
    }
  },
  "context": {
    "ip": "127.0.0.1",
    "location": {"lat": 54.32, "lon": 33.44}
  }
}
```

The `principal` field MAY be omitted when it matches the authenticated token payload:

```http
POST /v1beta/authorization/ HTTP/1.1
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
  "action": {
    "name": "download",
    "service": "storage"
  },
  "resource": {
    "id": "/Projects/Scene.usd",
    "type": "File"
  }
}
```

Allow response:

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "decision": "allow",
  "action": "read",
  "service": "storage"
}
```

Explicit deny response:

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "decision": "deny",
  "reason": "No rule grants read on this resource.",
  "action": "read",
  "service": "storage"
}
```

(permission-api-rest-api-post-v1beta-authorization-batch)=
## POST /v1beta/authorization/batch/ — Check permissions in batch

Batched variant of `POST /v1beta/authorization/` that evaluates multiple actions, resources, and principals in one request.

**Request body:** {ref}`AuthorizationBatchRequest <permission-api-rest-api-model-authorization-batch-request>`.

Instead of repeating the same resource, a client SHOULD list all actions to check for a resource in a single batch entry. Each `principal` MAY be omitted when it matches the authenticated token payload.

The service returns `200` with an {ref}`AuthorizationBatchResponse <permission-api-rest-api-model-authorization-batch-response>`. The `decisions` array contains exactly one object per request batch, in order; each object has one `<service>:<action>` key for each distinct action in that batch's `actions` array.

Actions are evaluated across all batches in request order (all actions in batch 0, then batch 1, and so on). The `condition` field controls short-circuiting, which spans all batches, and whether a `summary` is produced:

- `none` (default) – Every action in every batch is evaluated independently, as if sent as separate requests. No action is `skip`, and `summary` MUST be omitted.
- `and` – Evaluation stops after the first `deny`. Remaining unevaluated actions (including actions in later batches) are set to `skip`. `summary.decision` is `deny` if any action was denied, otherwise `allow`. `summary` MUST be present.
- `or` – Evaluation stops after the first `allow`. Remaining unevaluated actions (including actions in later batches) are set to `skip`. `summary.decision` is `allow` if any action was allowed, otherwise `deny`. `summary` MUST be present.

(permission-api-rest-api-examples)=
### Examples

Check several actions for one resource, evaluated independently (`none`):

```http
POST /v1beta/authorization/batch/ HTTP/1.1
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
  "batches": [
    {
      "principal": {"sub": "DdxA9xDiqdUbv", "info": {"email": "user@test.com"}},
      "actions": [
        {"name": "read", "service": "storage"},
        {"name": "write", "service": "storage"},
        {"name": "set", "service": "tags"},
        {"name": "get", "service": "tags"}
      ],
      "resource": {
        "id": "/Projects/Scene.usd",
        "type": "File",
        "data": {"metadata": {"size": 1024}}
      }
    }
  ]
}
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "decisions": [
    {
      "storage:read": {"decision": "allow"},
      "storage:write": {"decision": "deny"},
      "tags:set": {"decision": "deny", "reason": "No matching rule."},
      "tags:get": {"decision": "allow"}
    }
  ]
}
```

Check whether the caller has `storage:read` on any of several resources (`or`):

```http
POST /v1beta/authorization/batch/ HTTP/1.1
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
  "condition": "or",
  "batches": [
    {
      "actions": [{"name": "read", "service": "storage"}],
      "resource": {"id": "/Projects/Astronaut/Astronaut.usd", "type": "File"}
    },
    {
      "actions": [{"name": "read", "service": "storage"}],
      "resource": {"id": "/Projects/Marbles/Marbles_Assets.usd", "type": "File"}
    }
  ]
}
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "summary": {"decision": "allow"},
  "decisions": [
    {"storage:read": {"decision": "allow"}},
    {"storage:read": {"decision": "skip"}}
  ]
}
```

Check whether the caller has every listed action on a resource (`and`):

```http
POST /v1beta/authorization/batch/ HTTP/1.1
Authorization: Bearer eyJraWQiOiJvYXV0aC1zaWduL[...]cc5IgUXhY66ML-CZVlRw
Content-Type: application/json

{
  "condition": "and",
  "batches": [
    {
      "actions": [
        {"name": "read", "service": "storage"},
        {"name": "write", "service": "storage"},
        {"name": "set", "service": "tags"},
        {"name": "get", "service": "tags"}
      ],
      "resource": {
        "id": "/Projects/Scene.usd",
        "type": "File",
        "data": {"metadata": {"size": 1024}}
      }
    }
  ]
}
```

Response:

```http
HTTP/1.1 200 OK
Content-Type: application/json

{
  "summary": {"decision": "deny"},
  "decisions": [
    {
      "storage:read": {"decision": "allow"},
      "storage:write": {"decision": "deny"},
      "tags:set": {"decision": "skip"},
      "tags:get": {"decision": "skip"}
    }
  ]
}
```

(permission-api-rest-api-see-next)=
## See next

* {ref}`gRPC specification <permission-api-grpc-grpc-specification>`
