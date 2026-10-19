# Permission Service Conformance Test — Agent Context

You are building a **permission service** — a centralized authorization service that other microservices call to determine whether a given principal (user or service account) is allowed to perform a specific action on a specific resource.

The service does not manage users, roles, or resources directly. It evaluates authorization rules that have been configured ahead of time and returns allow/deny decisions. Callers send a (principal, action, resource) tuple and receive a decision in response.

The service exposes two equivalent APIs — REST and gRPC — that must behave identically. It must pass the conformance tests in this crate.

## Domain Model

- **Principal**: `{ sub: string, info?: object }` — the caller identity
- **Action**: `{ name: string, service: string }` — the operation being authorized
- **Resource**: `{ id: string, type: string, data?: object }` — the target of the operation
- **Context**: `object` — optional extra information about the request (e.g. caller IP, geolocation) that authorization rules may use during evaluation
- **Decision**: `allow`, `deny`, or `skip`

## REST API

### `GET /health`

- Return HTTP 200 when the service is healthy

### `POST /v1beta/authorization/`

Request body (`AuthorizationPayload`):

```json
{
  "principal": { "sub": "<string>", "info": {} },
  "action": { "name": "<string>", "service": "<string>" },
  "resource": { "id": "<string>", "type": "<string>", "data": {} },
  "context": {}
}
```

- `action` is required
- `principal`, `resource`, `context` are optional

Response body (`AuthorizationResponse`):

```json
{
  "decision": "allow" | "deny" | "skip",
  "reason": "<string or null>",
  "action": "<string or null>",
  "service": "<string or null>"
}
```

- `decision` is required
- `reason`, `action`, `service` are optional
- No other fields are permitted in the response object

### `POST /v1beta/authorization/batch/`

Request body (`AuthorizationBatchRequest`):

```json
{
  "condition": null | "none" | "or" | "and",
  "batches": [
    {
      "principal": { "sub": "<string>", "info": {} },
      "actions": [
        { "name": "<string>", "service": "<string>" }
      ],
      "resource": { "id": "<string>", "type": "<string>", "data": {} },
      "context": {}
    }
  ]
}
```

- `batches` is required (max 100 items); each batch requires `actions` (max 1000 per batch)
- `condition`, `principal`, `resource`, `context` are optional

Response body (`AuthorizationBatchResponse`):

```json
{
  "decisions": [
    {
      "<service>:<action>": {
        "decision": "allow" | "deny" | "skip",
        "reason": "<string or null>",
        "action": "<string or null>",
        "service": "<string or null>"
      }
    }
  ],
  "summary": null | {
    "decision": "allow" | "deny" | "skip",
    "reason": "<string or null>"
  }
}
```

- `decisions` is required; there is exactly one item per batch in the request
- Each item is a JSON object (map) keyed by `"<service>:<action>"` containing all actions from that batch
- `decisions` order must match the order of batches in the request; keys within each map must match the actions in that batch
- `summary` is present only when `condition` is `"or"` or `"and"`

### Error Responses

All error bodies are a plain JSON string (`ErrorString`), max 2048 characters.

| Status | Meaning |
|--------|---------|
| 400 | Bad request |
| 401 | Missing or invalid authentication credentials |
| 403 | Caller lacks permission to check authorization |
| 429 | Rate limited |
| 500 | Internal server error |

## gRPC API

Service: `nvidia.omniverse.permission.v1beta.PermissionService`

### `CheckPermission(CheckPermissionRequest) returns (CheckPermissionResponse)`

```protobuf
message CheckPermissionRequest {
  optional Principal principal = 1;
  Action action = 2;
  optional Resource resource = 3;
  optional google.protobuf.Struct context = 4;
}

message CheckPermissionResponse {
  Decision decision = 1;
  optional string reason = 2;
}
```

### `CheckPermissionBatch(CheckPermissionBatchRequest) returns (CheckPermissionBatchResponse)`

```protobuf
message CheckPermissionBatchRequest {
  optional Condition condition = 1;
  repeated CheckPermissionBatch batches = 2;
}

message CheckPermissionBatchResponse {
  optional CheckPermissionBatchResponseSummary summary = 1;
  repeated ResourceActionDecisionBatch decisions = 2;
}

message ResourceActionDecisionBatch {
  repeated ResourceActionDecision results = 1;
}

message ResourceActionDecision {
  string action = 1;
  string service = 2;
  Decision decision = 3;
  optional string reason = 4;
}
```

### Enums

```protobuf
enum Decision {
  DECISION_UNSPECIFIED = 0;
  DECISION_DENY = 1;
  DECISION_ALLOW = 2;
  DECISION_SKIP = 3;
}

enum Condition {
  CONDITION_UNSPECIFIED = 0;
  CONDITION_OR = 1;
  CONDITION_AND = 2;
}
```

### Shared Messages

```protobuf
message Principal {
  string sub = 1;
  google.protobuf.Struct info = 2;
}

message Action {
  string name = 1;
  string service = 2;
}

message Resource {
  string id = 1;
  string type = 2;
  optional google.protobuf.Struct data = 3;
}
```

## Behavior Rules

### Single Authorization

- Return `decision: allow` when an authorization rule matches the request principal, action, and resource
- Return `decision: deny` when no rule matches
- Populate `reason` on explicit denials; omit `reason` on implicit denials (no matching rules)
- When present, `action` and `service` in the response must echo the request values

### Batch Authorization — Evaluation Order

- Actions are evaluated across all batches in request order: all actions in batch 0 first, then batch 1, etc.
- Condition-based short-circuiting spans across batches (not per-batch)

### Batch Authorization — `condition: "and"` / `CONDITION_AND`

- Evaluate actions in order across all batches; stop after the first `deny`
- Set remaining unevaluated actions (including actions in subsequent batches) to `skip`
- Set `summary.decision` to `deny` if any action is denied, `allow` if all are allowed
- `summary` must be present

### Batch Authorization — `condition: "or"` / `CONDITION_OR`

- Evaluate actions in order across all batches; stop after the first `allow`
- Set remaining unevaluated actions (including actions in subsequent batches) to `skip`
- Set `summary.decision` to `allow` if any action is allowed, `deny` if all are denied
- `summary` must be present

### Batch Authorization — `condition: null`/`"none"` / `CONDITION_UNSPECIFIED`

- Evaluate all actions in all batches individually; do not short-circuit
- Do not populate `summary` (must be null or absent)
- No decisions should have `skip`

### REST Batch Response Format

- The `decisions` array has exactly one item per batch in the request
- Each item is a JSON object (map) keyed by `"<service>:<action>"` containing all action results for that batch
- The value for each key is an `AuthorizationResponse` object

### gRPC Batch Response Format

- The `decisions` array has exactly one `ResourceActionDecisionBatch` per batch in the request
- `decisions` order must match the order of batches in the request; `results` order must match the order of actions within each batch
- Each `ResourceActionDecision` must set `action` and `service` fields echoing the request values

## Authentication

- Accept `Authorization: Bearer <JWT>` headers on both REST and gRPC
- When authentication is enabled, return 401 (REST) or `UNAUTHENTICATED` (gRPC) for missing or invalid tokens
- When authentication is disabled, process requests without credentials

## Field Validation Constraints

- `Action.name`, `Action.service`: max 255 chars, pattern `^[a-zA-Z0-9_.:/-]+$`
- `Principal.sub`: max 255 chars, pattern `^[^\x00-\x1F\x22\x27\x5C]+$`
- `Resource.id`: max 2048 chars, pattern `^[^\x00-\x1F\x22\x27\x5C]+$`
- `Resource.type`: max 255 chars, pattern `^[a-zA-Z0-9_.:/-]+$`
- `ErrorString`: max 2048 chars, pattern `^[\x20-\x7E\r\n]+$`
