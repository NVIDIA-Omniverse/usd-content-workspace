(userinfo-service-access-logs-api-access-logs)=
# API Access Logs

An **access log** is a structured record of REST and gRPC service access: who called which
operation, the parameters that identify the request, and the status and outcome of the response.
Records cover both business calls the service processes and requests it rejects before the
requested operation runs — authentication failures, unknown routes and methods, and malformed
requests — so administrators can track and debug invalid access as well as successful calls. Access
logs are meant for audit trails and request-level troubleshooting, and are separate from the
operational logs and metrics described in
{ref}`Observability <userinfo-service-observability-observability>`. They are **disabled by
default**; when disabled, no access records are produced and the standard output carries only the
operational logs described in {ref}`Logging <userinfo-service-observability-logging>`.

Access records are emitted on the standard output alongside the operational logs, as a dedicated
record type a log pipeline can filter on, so no additional collector or endpoint is required. When
structured logging is enabled (see
{ref}`Logging <userinfo-service-observability-logging>`), each record is emitted as JSON.

(userinfo-service-access-logs-what-is-recorded)=
## What Is Recorded

When enabled, the service logs **all service access** except infrastructure and discovery
endpoints. Specifically:

- A record is produced for every business call the service accepts and processes, whether it
  succeeds or fails. This is a **combined** record carrying both `request` and `response`, because
  each published operation is a single request/response exchange.
- A record is also produced for a request the service **rejects before the requested operation
  runs**, so failed and invalid access is auditable — authentication failures
  (`401` / `UNAUTHENTICATED`), authorization failures (`403` / `PERMISSION_DENIED`), unknown routes and methods
  (`404` / `405` / `UNIMPLEMENTED`), malformed or unparseable requests (`400` / `INVALID_ARGUMENT`),
  rate limiting (`429`), and server errors (`500` / `INTERNAL`). A request that never resolves to a
  business operation produces a **call-level** record: it carries `operation`, `transport`, header
  fields, and `response`, but **no `request`**, because no input was parsed. For an unknown REST
  route or gRPC method the `operation` is the raw `{METHOD} {path}` (REST) or requested method path
  (gRPC) actually attempted.
- Records are **not** produced for infrastructure or discovery endpoints, even when they fail — the
  health and readiness probes, and the gRPC health, reflection, and capability-discovery services.

Each record contains the following fields:

| Field | Description |
|---|---|
| `transport` | Which API surface served the call: `"REST"` for REST-over-HTTP, `"gRPC"` for gRPC. It tells a consumer how to read `response.status`. Always present. |
| `operation` | For REST, the method and route template (for example `GET /api/v1alpha/users/{userId}`), not the substituted URL — path parameter values appear in `request.params`. For gRPC, the full method path (for example `/nvidia.omniverse.userinfo.v1alpha.UserInfoService/GetUser`). For a request that never matched a business operation, the raw method and path actually attempted. Always present. |
| `request.params` | The identifying inputs of the call: path parameters (such as `user_id`, `group_id`) and the query or message filters that were supplied (`email`, `display_name`, and the page size and page cursor). Empty when the operation has none. Absent entirely on a call-level record. |
| `response.status` | The call's terminal status: for REST the HTTP status code, for gRPC the numeric status code (`0` = OK). |
| `response.detail` | Present only when there is something to add: `item_count` for list operations, and, on failure, a human-readable `message`. gRPC records additionally carry `code`, the status name (`OK`, `NOT_FOUND`, `PERMISSION_DENIED`, ...). Omitted entirely when empty. |
| header fields | Extra top-level fields copied from request headers per the configured header map (see {ref}`Configuration <userinfo-service-access-logs-configuration>`), for example `ip` and `ua`. A field is omitted when its header is missing or empty. |

A REST record for a successful user lookup looks like this:

```json
{
  "type": "access",
  "access": {
    "transport": "REST",
    "ip": "203.0.113.10",
    "ua": "Mozilla/5.0",
    "operation": "GET /api/v1alpha/users/{userId}",
    "request": { "params": { "user_id": "abc-123" } },
    "response": { "status": 200 }
  }
}
```

A call-level record for an unauthenticated request omits `request` entirely:

```json
{
  "type": "access",
  "access": {
    "transport": "REST",
    "operation": "GET /api/v1alpha/users",
    "response": { "status": 401, "detail": { "message": "missing or invalid Authorization header" } }
  }
}
```

When structured logging is disabled (the default), the same record is rendered in the
human-readable text form used by the operational logs, with the `access` fields flattened into one
indented `key.path: value` line per value:

```text
2026-09-09T03:18:00.123456Z  INFO access:
    transport: REST
    ip: 203.0.113.10
    ua: Mozilla/5.0
    operation: GET /api/v1alpha/users/{userId}
    request.params.user_id: abc-123
    response.status: 200
```

The `request.params` are recorded on a best-effort basis: they reflect only the inputs determined
before a failure occurred, so a call that fails early may record fewer parameters than a successful
one, and a call rejected before its operation runs records none at all.

(userinfo-service-access-logs-what-is-never-logged)=
## What Is Never Logged

Access records are deliberately minimal. They never contain:

- **Credentials** — the `Authorization` header, bearer tokens, or cookies are never copied,
  regardless of the header map.
- **Response bodies** — user, group, and membership payloads are never logged; only a count of
  the items a list returned appears in `response.detail`.

(userinfo-service-access-logs-logged-operations)=
## Logged Operations

The table below summarizes what each published operation records. REST and gRPC behave
identically; the gRPC method that corresponds to each REST endpoint is listed alongside it.

| Operation | `request.params` | Success `response.detail` |
|---|---|---|
| `GET /users/me` · `GetCurrentUser` | (none) | (none) |
| `GET /users` · `ListUsers` | supplied `email`, `display_name`, page size, page cursor | `item_count` |
| `GET /users/{userId}` · `GetUser` | `user_id` | (none) |
| `GET /users/{userId}/groups` · `ListUserGroups` | `user_id`, supplied `display_name`, page size, page cursor | `item_count` |
| `GET /users/{userId}/groups/{groupId}` · `GetUserGroup` | `user_id`, `group_id` | (none) |
| `GET /groups` · `ListGroups` | supplied `display_name`, page size, page cursor | `item_count` |
| `GET /groups/{groupId}` · `GetGroup` | `group_id` | (none) |
| `GET /groups/{groupId}/users` · `ListGroupMembers` | `group_id`, supplied `email`, `display_name`, page size, page cursor | `item_count` |
| `GET /groups/{groupId}/users/{userId}` · `GetGroupMember` | `group_id`, `user_id` | (none) |

On any error, `response.status` carries the status code and `response.detail.message` describes the
failure; the logged message may be more specific than the generic message returned to the caller.
For gRPC, `response.detail.code` carries the status name.

Requests that do not match any operation above — unknown paths, unsupported methods, and
unauthenticated or malformed calls — are still logged as **call-level** records using the raw
requested `operation` and the resulting error `response`, with no `request.params`. Infrastructure
and discovery endpoints (the health and readiness probes and the gRPC health, reflection, and
capability-discovery services) are never logged, even on failure.

(userinfo-service-access-logs-configuration)=
## Configuration

Access logging exposes two settings: a master switch, disabled by default, and a header map that
selects which request headers are copied onto each record as top-level fields.

| Setting | Helm value (`observability.logging.access.*`) | Default | Description |
|---|---|---|---|
| Enable access logs | `enabled` | `false` | When `false`, no records are produced. When `true`, records are produced for qualifying calls. |
| Header map | `headers` | `{}` | Map of access-record field name to request header name. Each present header is copied as a top-level field. `Authorization` and `Cookie` are never copied, and the reserved names `transport`, `operation`, `request`, and `response` are ignored. Header names match case-insensitively. |

Both settings render as environment variables in the reader pod specification, so changing either
and upgrading the release rolls the reader pods automatically. Each value takes effect when the pod
starts.

(userinfo-service-access-logs-helm)=
### Configuring Access Logs in the Helm Chart

Add the settings under `observability.logging.access` in `userinfo-values.yaml`. The example below
enables access logging and copies the caller IP and user agent onto each record:

```yaml
observability:
  logging:
    access:
      enabled: true
      headers:
        ip: "x-forwarded-for"
        ua: "user-agent"
```

Install or upgrade the release with the same commands used for the synchronized directory
deployment in
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-access-logs-see-also)=
## See Also

- {ref}`Observability <userinfo-service-observability-observability>` — where access logs sit
  alongside the operational logs, traces, and metrics, and the request-header forwarding that
  shares the `observability` configuration section.
