(permission-service-access-logs-api-access-logs)=
# API access logs

An **access log** is a structured record of REST and gRPC service access: who called which operation, the parameters that identify the request, and the status and outcome of the response. Records cover both handled business calls and requests the service rejects before a handler runs (authentication failures, unknown routes and methods, malformed payloads), so administrators can track and debug invalid requests as well as successful ones. Access logs are meant for audit trails, request-level troubleshooting, and feeding a log pipeline that answers "which principal asked what, and what did the service decide". They are separate from operational logs and metrics, and are **disabled by default**.

This page is the access-log **specification**: it defines the record contract, what must never be logged, which calls produce records, and how records are shaped for request/response and streaming operations. It describes *what* must be emitted, not how a service collects the information internally — the mechanism (where the data is gathered, when the record is written) is left to each implementation. The {ref}`endpoint table <permission-service-access-logs-permission-service-endpoints>` at the end applies the contract to this service.

```{note}
The keywords **must**, **must not**, **should**, and **may** in this document are used in the normative sense: **must**/**must not** are hard requirements, **should** is a strong recommendation, and **may** marks an option left to the implementer.
```

(permission-service-access-logs-enabling-access-logs)=
## Availability

Access logging is optional and **must be disabled by default**. When disabled, no records are produced. When enabled, one or more records are produced for each qualifying call (see {ref}`Which calls produce records <permission-service-access-logs-when-records-appear>`).

(permission-service-access-logs-record-contract)=
## Record contract

Every access record describes **one** operation. Most calls produce a single **combined** record that carries both `request` and `response`; operations that cover a list (batch authorization, bulk metadata writes) produce one combined record per item — see {ref}`Unwinding <permission-service-access-logs-unwinding>`. Streaming RPCs instead produce one-sided message records plus a closing record. Which shape applies is decided by the operation's cardinality — see {ref}`Format selection <permission-service-access-logs-format-selection>`. A request rejected before it resolves to an operation (a failed authentication, an unknown route or method, or a malformed payload) produces a single **call-level** record that carries `response` but no `request.params` — see {ref}`Which calls produce records <permission-service-access-logs-when-records-appear>`.

The canonical serialization is a nested JSON envelope:

```json
{
  "type": "access",
  "access": {
    "transport": "REST",
    "ip": "203.0.113.10",
    "ua": "Mozilla/5.0",
    "operation": "POST /v1beta/authorization/",
    "request": {
      "params": {
        "principal": { "sub": "user-1" },
        "action": { "name": "read", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    },
    "response": {
      "status": 200,
      "detail": {
        "decision": "deny",
        "reason": "Explicit deny."
      }
    }
  }
}
```

A record may instead be rendered in a human-readable form; any such rendering must preserve the same `access` fields. For example, one indented `path: value` line per leaf field (nested keys joined with `.`):

```text
2026-09-03T03:12:00Z  INFO access:
    transport: REST
    ip: 203.0.113.10
    ua: Mozilla/5.0
    operation: POST /v1beta/authorization/
    request.params.principal.sub: user-1
    request.params.action.name: read
    request.params.action.service: storage
    request.params.resource.id: /Projects/Scene.usd
    request.params.resource.type: File
    response.status: 200
    response.detail.decision: deny
    response.detail.reason: Explicit deny.
```

The fields under `access` are:

| Field         | Description |
|---------------|-------------|
| `transport`   | Which API surface served the call: `"REST"` for REST-over-HTTP, `"gRPC"` for gRPC. Consumers use it to interpret `response.status` — an HTTP status code when `"REST"`, a numeric gRPC status code when `"gRPC"` — without parsing `operation`. Always present. |
| `operation`   | For REST, `{METHOD} {route template}` using the route template (for example `PUT /v1beta/policies/{id}`), not the substituted URL — path parameter values go in `request.params`. For gRPC, the full method path (for example `/nvidia.omniverse.permission.v1beta.PermissionService/CheckPermission`). For a request that never matched a business operation (unknown REST route or gRPC method), the raw `{METHOD} {path}` (REST) or requested method path (gRPC) is used instead. Always present. |
| `request`     | Present on combined and streaming message records. Omitted entirely on a **call-level** record for a request rejected before a handler, since no input was parsed. |
| `request.params` | Operation-specific object describing the **input** (query, path, and selected body fields). Empty (`{}`) when the operation has no such fields; absent on a call-level record. |
| `request.index`  | 0-based position of this record among the request items/messages of the call. Present whenever a call produces more than one record — on every record of an unwinding call and on every request-message record of a streaming call — and absent for a single-record call. Consumers reconstruct item/message order from this value, not from the order records arrive in a log pipeline. |
| `response.index` | 0-based position among the response messages of a streaming call. Present only on streaming response-message records; never on a combined record. `request.index` and `response.index` are independent counters. |
| `response.status` | The call's terminal status: for REST the HTTP status code as a number, for gRPC the numeric status code (`0` = OK). Present on every combined record — repeated identically across the unwound records of one call — and on the streaming closing record. Absent on intermediate streaming message records, whose outcome is not yet known. |
| `response.detail` | Always a JSON **object** when present; never a bare string or array. The `detail` key is omitted entirely (never emitted as an empty `{}`) when there is nothing to add. |
| `response.final` | Boolean; present and `true` only on the streaming closing record — the one record that reports how a streaming call ended. Never present on a combined record or an intermediate message record. |
| header fields | Extra top-level fields under `access` copied from request headers per the configured header map (see {ref}`Configuration <permission-service-access-logs-configuration>`), for example `ip`, `ua`. A field is omitted when its header is missing or empty. |

Required fields, at a glance:

- Every record must include `transport` and `operation`.
- A **combined** record must include `request` and `response`.
- A **call-level** record (a request rejected before a handler) must include `response` and must omit `request`.
- A streaming **message** record must include exactly one of `request` or `response`.
- The streaming **closing** record must include `response.final: true` and `response.status`.

`response.detail` is homogeneous: when the only thing to report is a text message, it must be wrapped as `{ "message": "..." }` rather than logged as a bare string.

```json
"response": {
  "status": 404,
  "detail": { "message": "Policy with ID '1' could not be found." }
}
```

The keys that may appear inside `response.detail` (all optional; empty keys are omitted):

- `message` — a human-readable error message on failures. It may differ from the body returned to the caller: a failure may log a more specific internal reason than the generic message the client receives (for example, a `403` may return "permission denied" to the caller while the log records the precise reason).
- `code` — for gRPC, the status name (`OK`, `INVALID_ARGUMENT`, `PERMISSION_DENIED`, `UNAUTHENTICATED`, `INTERNAL`, …).
- `decision` (`allow` / `deny` / `skip`) and `reason` — for authorization checks.
- `evaluation_priority`, `policy_count` — for diagnostics checks.
- `created` — for policy writes. `item_count` — for list reads.

```{note}
There is no built-in per-request identifier and no `access.principal` field. To correlate the several records produced by an unwinding call, or to group a call with an upstream trace, map a gateway correlation header (such as `x-request-id`) through the header map so it appears as a top-level field on every record of the call.
```

(permission-service-access-logs-what-is-never-logged)=
## What is never logged

Access records are deliberately minimal. They must never contain:

- **Credentials** — the `Authorization` header, bearer tokens, or cookies are never copied, regardless of the header map.
- **Full response bodies** — for example policy lists, key material, or free-form policy text; only counts or summaries appear in `response.detail`.
- **Sensitive request payloads** — free-form principal `info`, resource `data`, request `context`, and similar caller-supplied nested objects are omitted from `request.params`, since they may carry sensitive information.

(permission-service-access-logs-configuration)=
## Configuration

An implementation must expose:

- an **enable switch**, defaulting to disabled; and
- a **header map** from access field name to request header name.

The header map obeys these rules:

- Credential headers (`Authorization`, `Cookie`, and equivalents) must never be copied, regardless of the map.
- The reserved field names `transport`, `operation`, `request`, and `response` must not be overridden by the map.
- Header lookups are case-insensitive; for gRPC, metadata keys are lowercase, so configure header names accordingly.
- Values carried with a transport-level encoding (such as the percent-encoded gRPC status message) must be decoded to their human-readable form before they are recorded.
- A mapped field is omitted when its header is absent or empty.
- Copied fields appear on every record of a call, including unwound records.

(permission-service-access-logs-helm-chart-values)=
### Helm chart values

Services deployed with the shared Helm chart expose these knobs under `observability.logging.access`, each with a matching environment variable, so the configuration structure is identical across services:

| Value | Env var | Default | Meaning |
|-------|---------|---------|---------|
| `observability.logging.access.enabled` | `ACCESS_LOG_ENABLED` | `false` | Master switch. When `false`, no records are produced. When `true`, records are produced for qualifying calls. |
| `observability.logging.access.headers` | `ACCESS_LOG_HEADERS` | `{}` | Map of access field name to request header name. Each present header is copied as a top-level field under `access`. `Authorization` and `Cookie` are never copied, and the reserved keys `transport`, `operation`, `request`, and `response` are ignored. |

```yaml
observability:
  logging:
    level: "info"
    structured:
      enabled: false
    access:
      enabled: true
      headers:
        ip: "x-forwarded-for"
        ua: "user-agent"
```

(permission-service-access-logs-when-records-appear)=
## Which calls produce records

When enabled, the service logs **all service access** except infrastructure and discovery endpoints, so administrators can track and debug rejected and invalid requests, not just successful business calls. Specifically:

- A record must be produced for every business call the service accepts, whether it succeeds or fails.
- A record must also be produced for a request the service **rejects before a handler runs**, so failed and invalid access is auditable. This includes authentication failures (`401` / `UNAUTHENTICATED`), authorization failures (`403` / `PERMISSION_DENIED`), unknown routes and methods (`404` / `405` / `UNIMPLEMENTED`), malformed or unparseable payloads (`400` / `INVALID_ARGUMENT`), rate limiting (`429`), and server errors (`500` / `INTERNAL`).
- A record must **not** be produced for infrastructure or discovery endpoints, even when they fail — for example health checks, the schema/discovery document and its UI (`GET /health`, `GET /openapi.json`, the bundled Swagger UI), and gRPC health, reflection, and capability discovery.

A request rejected before it resolves to a business operation produces a single **call-level** record: it carries `operation`, `transport`, any header fields, and `response` (the status and, on a best-effort basis, a `detail.message` and — for gRPC — `code`), but has **no `request.params`** because the input was never parsed into operation fields. For an unknown REST route or gRPC method the `operation` is the raw `{METHOD} {path}` (REST) or requested method path (gRPC) actually attempted, so administrators can see exactly what was requested.

`request.params` are recorded on a **best-effort** basis: they reflect only the input fields determined before a failure occurred. A call that fails early may therefore log fewer `params` (or an empty `{}`) than a successful call to the same operation, and a call rejected before a handler logs none at all.

(permission-service-access-logs-format-selection)=
## Format selection

Two record shapes exist, and which one applies is decided by the **cardinality of the operation**, not by how many records it produces. It can be read off the method definition:

- **Request/response operations** — all REST calls, and unary RPCs (one request message, one response message), whether they carry a single item or a list. The whole outcome is known for the record, so each record is **combined**: it carries both `request` and `response`. A call that covers a list produces one combined record per item (see {ref}`Unwinding <permission-service-access-logs-unwinding>`). This is the only shape that maps identically onto REST and gRPC, so it is the default for anything that is not a gRPC stream.
- **Streaming RPCs** — client-, server-, or bidirectional-streaming methods. A per-message outcome is not known while the stream is open, so a streaming call produces one-sided message records plus a single closing record for the terminal status (see {ref}`Streaming RPCs <permission-service-access-logs-grpc-streaming-rpcs>`). This shape is gRPC-specific and has no REST equivalent.

The one-sided message records, the independent `response.index`, and the `response.final` closing record exist because a stream's per-message outcome is not known up front; they never appear for request/response operations. Conversely, list unwinding is an **application-level** granularity that is independent of gRPC message cardinality: a unary batch is still one request message and one response message on the wire, so it is logged with combined records — one per item — not as a stream of messages.

(permission-service-access-logs-grpc-request-response-model)=
## gRPC request/response model

gRPC records use the full method path as `operation`, a numeric `response.status` (`0` for OK), and `response.detail.code` carrying the status name.

```{note}
gRPC percent-encodes the status message (`grpc-message`) on the wire. Records store the **decoded**, human-readable status message, never the on-the-wire form: for example, `response.detail.message` reads `"action" is required.`, not `%22action%22%20is%20required.`.

gRPC does not percent-encode custom metadata. ASCII metadata values are printable ASCII and are copied through the header map exactly as received. Binary metadata (keys ending in `-bin`) is base64-encoded on the wire, and a mapped `-bin` key is recorded in that base64 form.
```

(permission-service-access-logs-grpc-unary-rpcs)=
### Unary RPCs

A unary RPC — one request message, one response message — produces its record(s) with `response.status` and `response.detail.code` set from the final status. `CheckPermission` produces one record; `CheckPermissionBatch` produces one record per action (see {ref}`Unwinding <permission-service-access-logs-unwinding>`), all within the single unary call.

(permission-service-access-logs-grpc-streaming-rpcs)=
### Streaming RPCs

A streaming RPC has no single request/response pair, so it is logged as a **sequence of records**: one record for each request message, one record for each response message, and one closing record for the terminal status. `operation` and any header fields appear on every record of the call.

Each streaming record therefore describes a single message and carries **exactly one** of `request` or `response`. The irrelevant side is omitted, which is what tells the two kinds apart:

- **Request record** — describes a request message. Contains `request.index` (0-based position in the inbound stream) and `request.params` (selected fields of that message). It has **no `response`**: the outcome is not known for a request message.
- **Response record** — describes a response message. Contains `response.index` (0-based position in the outbound stream) and `response.detail` (selected fields of that message). It has **no `request`**, and **no `response.status`**: the call has not terminated at an intermediate response.
- **Closing record** — describes how the call ended. It is explicitly marked with `response.final` set to `true` and carries `response.status` (terminal numeric gRPC code) and `response.detail.code` (its name; plus `detail.message` on error). It has no `request` and no `response.index`. This `response.final` flag — not the mere absence of `index` — is what distinguishes the closing record from an ordinary response message record; it is the only record that reports how the call ended.

`request.index` and `response.index` are independent counters, so a request record and a response record may share the same index within one call. The mapping is the same for all three streaming kinds — the kinds only differ in how many of each record appear:

- **Client-streaming** (many requests, one response): several request records, one response record, and the closing record.
- **Server-streaming** (one request, many responses): one request record, several response records, and the closing record.
- **Bidirectional** (many requests, many responses): request and response records interleaved by message order, and the closing record.

Message payloads are never logged wholesale — only the selected `params` and `detail` fields are recorded.

(permission-service-access-logs-what-to-log-on-a-stream)=
#### What to log on a stream

One record per message is the reference model, not a mandate. For a high-throughput stream — for example telemetry or state updates pushed dozens of times per second — a record per message produces log volume that is expensive and rarely useful for audit. An implementation should choose a per-operation strategy rather than logging every message:

- **Always emit the closing record.** Whatever strategy is chosen for message records, the closing record (`response.final: true`) is mandatory: it captures who called, the `operation`, and how the call ended. An audit trail that keeps only closing records still answers "which principal opened which stream and what was the outcome".
- **Log stream lifecycle, not every message.** A reasonable default for chatty streams is to record the opening (the first request message, so the caller and initial parameters are captured) and the closing record, and to omit intermediate message records.
- **Sample or aggregate** when per-message visibility is needed but full fidelity is not: record every Nth message, or a single summary record (for example a count of messages and decisions seen) instead of one record per message. When sampling, keep `request.index` / `response.index` as the true stream position so gaps are visible.
- **Prefer safe defaults.** Intermediate message logging should be off by default and enabled deliberately, so a high-frequency stream cannot accidentally flood the log pipeline.

Whatever is chosen, per-message records must still follow the record contract above (one side each, correct indexes), and the mandatory closing record must terminate the sequence.

The examples below use a generic streaming service (canonical JSON form), in a representative order. Because a log pipeline may reorder records, consumers reconstruct sequence from `request.index` / `response.index` and identify the terminal record by `response.final`.

**Client-streaming** — the client sends two request messages; the server replies with one response and closes with `OK`. Two request records, then the response record, then the closing record:

```json
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/CheckStream",
    "request": {
      "index": 0,
      "params": {
        "principal": { "sub": "user-1" },
        "action": { "name": "read", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/CheckStream",
    "request": {
      "index": 1,
      "params": {
        "principal": { "sub": "user-1" },
        "action": { "name": "write", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/CheckStream",
    "response": {
      "index": 0,
      "detail": { "decision": "allow" }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/CheckStream",
    "response": { "final": true, "status": 0, "detail": { "code": "OK" } }
  }
}
```

**Server-streaming** — the client sends one request; the server streams two response messages and closes with `OK`. One request record, each response record, then the closing record:

```json
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/WatchDecisions",
    "request": {
      "index": 0,
      "params": {
        "principal": { "sub": "user-1" },
        "action": { "name": "read", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/WatchDecisions",
    "response": {
      "index": 0,
      "detail": { "decision": "allow" }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/WatchDecisions",
    "response": {
      "index": 1,
      "detail": { "decision": "deny", "reason": "Explicit deny." }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/WatchDecisions",
    "response": { "final": true, "status": 0, "detail": { "code": "OK" } }
  }
}
```

**Bidirectional, terminated by an error** — the client sends two request messages; the stream ends with `INVALID_ARGUMENT` before any response is sent. Two request records, then the closing record carries the error:

```json
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/Exchange",
    "request": {
      "index": 0,
      "params": { "action": { "name": "read", "service": "storage" } }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/Exchange",
    "request": {
      "index": 1,
      "params": { "action": { "name": "write", "service": "storage" } }
    }
  }
}
{
  "type": "access",
  "access": {
    "operation": "/example.v1.StreamService/Exchange",
    "response": { "final": true, "status": 3, "detail": { "code": "INVALID_ARGUMENT", "message": "\"resource.id\" is required." } }
  }
}
```

(permission-service-access-logs-unwinding)=
## Unwinding

Operations that cover a list (batch authorization, bulk action and resource-type writes) produce **one record per item**, all sharing the same `operation`, header fields, and `response.status`, in request order. Each item record carries a `request.index`; only `response.detail` varies from item to item. A request whose list is empty still produces a single record carrying the call-level parameters and no `request.index`, so the call is not lost from the log.

For example, a `POST /v1beta/authorization/batch/` call with `condition: "and"` and two actions produces two records (canonical JSON form; `index` starts at `0`):

```json
{
  "type": "access",
  "access": {
    "operation": "POST /v1beta/authorization/batch/",
    "request": {
      "index": 0,
      "params": {
        "condition": "and",
        "principal": { "sub": "user-1" },
        "action": { "name": "read", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    },
    "response": { "status": 200, "detail": { "decision": "allow" } }
  }
}
{
  "type": "access",
  "access": {
    "operation": "POST /v1beta/authorization/batch/",
    "request": {
      "index": 1,
      "params": {
        "condition": "and",
        "principal": { "sub": "user-1" },
        "action": { "name": "write", "service": "storage" },
        "resource": { "id": "/Projects/Scene.usd", "type": "File" }
      }
    },
    "response": { "status": 200, "detail": { "decision": "deny", "reason": "Explicit deny." } }
  }
}
```

On a whole-call failure (for example a `403` or a `500`), the item records are still produced so the log shows what was asked, and the same error `response` is set on every record.

(permission-service-access-logs-permission-service-endpoints)=
## Permission Service endpoints

The table below summarizes what each business endpoint records. "Unwind" means one record per list item; all other endpoints produce a single record.

| Endpoint / RPC | `request.params` | Records | Success `response.detail` |
|----------------|------------------|---------|----------------------------|
| `POST /v1beta/authorization/` · `CheckPermission` | `principal.sub`, `action`, `resource` id/type | one | `decision`, `reason` (gRPC also `code`) |
| `POST /v1beta/authorization/batch/` · `CheckPermissionBatch` | per action: `condition`, `principal.sub`, one `action`, `resource` id/type | unwind actions across batches | per action: `decision` (incl. `skip`), `reason` |
| `POST /v1beta/diagnostics/authorize/` | same as single authorize | one | `evaluation_priority`, `policy_count` |
| `POST /v1beta/diagnostics/authorize/batch/` | same unwind as authorize batch | unwind actions across batches | per action: `evaluation_priority`, `policy_count` |
| `GET /v1beta/policies/` | query filters (`principal`, `group`, `action`, `resource`, `order`, `scope`, `page`, `limit`) | one | `item_count` |
| `GET /v1beta/policies/{id}` | `id` | one | — |
| `PUT /v1beta/policies/` | `policy`, `order`, `comment` | one | `created` |
| `PUT /v1beta/policies/batch/` | per item: the submitted policy payload | unwind items | per item: `created` |
| `DELETE /v1beta/policies/{id}` | `id` | one | — |
| `GET /v1beta/services/` | `{}` (lists every service) | one | — |
| `GET /v1beta/services/{service}/` | `service` | one | — |
| `GET /v1beta/services/{service}/actions/` | `service` | one | — |
| `GET /v1beta/services/{service}/resource-types/` | `service` | one | — |
| `GET /v1beta/services/{service}/resource-types/{type}/` | `service`, `type` | one | — |
| `PUT /v1beta/services/{service}/` | `service`, `id_claim` | one | — |
| `DELETE /v1beta/services/{service}/` | `service` | one | — |
| `PUT /v1beta/services/{service}/actions/` | per action: `service`, `name` | unwind actions | — |
| `PUT` · `DELETE /v1beta/services/{service}/actions/{action}/` | `service`, `name` | one | — |
| `PUT /v1beta/services/{service}/resource-types/` | per type: `service`, `type`, `evaluation_priority`, `parent_type` | unwind types | — |
| `PUT /v1beta/services/{service}/resource-types/{type}/` | `service`, `type`, `evaluation_priority`, `parent_type` | one | — |
| `DELETE /v1beta/services/{service}/resource-types/{type}/` | `service`, `type` | one | — |
| `GET /v1beta/idp/keys/` | `{}` | one | — |

On any error, `response.status` carries the status code and `response.detail.message` (and, for gRPC, `code`) describes the failure; for unwinding endpoints the same error `response` appears on every record.

Requests that do not match any endpoint above — unknown paths, unsupported methods, and unauthenticated or malformed calls — are still logged as **call-level** records using the raw requested `operation` and the resulting error `response`, with no `request.params`. Infrastructure and discovery endpoints (`GET /health`, `GET /openapi.json`, the Swagger UI, and gRPC health, reflection, and capability discovery) are never logged, even on failure.

(permission-service-access-logs-references)=
## References

- [gRPC over HTTP2 protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md) — wire encoding of the `grpc-message` status message (percent-encoded), ASCII custom metadata values, and base64-encoded `-bin` metadata.

(permission-service-access-logs-see-also)=
## See also

- {ref}`Observability <permission-service-observability-observability>` — where access logs sit alongside other logs, and the full logging values reference.
- {ref}`Managing policies through the REST API <permission-service-policy-management-managing-policies-through-the-rest-api>` — the policy endpoints summarized above.
- {ref}`Managing service metadata through the REST API <permission-service-metadata-managing-service-metadata-through-the-rest-api>` — the metadata endpoints summarized above.
- {ref}`Diagnostics API <permission-service-diagnostics-diagnostics-api>` — the diagnostics endpoints summarized above.
