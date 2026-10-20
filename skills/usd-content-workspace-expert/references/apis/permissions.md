# Permissions API

> **Applies to:** the Permission API contract and the reference Permission Service's extra management APIs · **Released:** `permission-api` 1.0.0-beta.1 ([HTML docs](https://docs.omniverse.nvidia.com/ovstorage/permission-api/1.0.0-beta.1/index.html), [NGC resource](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/permission-api)); Permission Service 0.0.53 · **This repo's snapshot:** proto package `nvidia.omniverse.permission.v1beta` (unchanged in Permission Service 0.0.54); OpenAPI `info.version` still 0.0.53 at the 0.0.54 snapshot, with no path or schema changes from 0.0.53 · **Sources:** `permissions/reference/permission-service/protos/nvidia/omniverse/permission/v1beta/permission_service.proto`, `openapi.json`, `docs/api/docs/{index,rest-api,grpc,authorization-for-a-user-principal,authorization-for-a-service-principal}.md`, `docs/service/docs/{policy-management,metadata,diagnostics}.md`, the [source skill package](../../NOTICE.md)

## What the contract covers

One question: **can principal P perform action A on resource R, given context C (PARC)?** The contract defines only decision endpoints. It does not prescribe how rules are stored or administered; the reference Permission Service adds policy, metadata, and diagnostics APIs on top (see [Management APIs](#reference-service-management-apis-not-in-the-contract)).

| Wire surface | Definition in this repo | Version |
|---|---|---|
| gRPC `nvidia.omniverse.permission.v1beta.PermissionService` | [`permission_service.proto`](../../../../permissions/reference/permission-service/protos/nvidia/omniverse/permission/v1beta/permission_service.proto) (imports `google/protobuf/struct.proto`) | package `v1beta` |
| REST under `/v1beta/` | [`openapi.json`](../../../../permissions/reference/permission-service/openapi.json) (OpenAPI 3.1.0) and [`rest-api.md`](../../../../permissions/reference/permission-service/docs/api/docs/rest-api.md) | path `v1beta`; document version 0.0.53 |
| Discovery `CapabilitiesService` (optional) | [`grpc.md`](../../../../permissions/reference/permission-service/docs/api/docs/grpc.md#service-discovery--capabilitiesservice) | `nvidia.omniverse.discovery.capabilities.v2alpha`, service type `permission` |

The NGC `permission-api:1.0.0-beta.1` resource is the released contract bundle. Generate bindings from the definitions that match the service you call: the released resource for a 1.3.0 deployment, or the files above for this snapshot. The Storage Service carries its own vendored copy of the permission protos (see [../versions/api-contracts.md](../versions/api-contracts.md)). Record the deployed Permission Service version first ([../versions/capture-a-deployment.md](../versions/capture-a-deployment.md)).

## Authentication

- REST: `Authorization: Bearer <access token>`. gRPC: metadata key `authorization` with `Bearer <token>`.
- The token is an access token from the OpenID Connect provider for a user (authorization code with PKCE) or a service (client credentials). Its `aud` must be accepted by the Permission Service: `openId.clientRegistrations[*].clientId` plus `openId.additionalJwtAudience` in the released 0.0.53, `openId.allowedJwtAudience` in the 0.0.54 snapshot. The 0.0.54 service also rejects JWTs without `aud`, without `iss` when an issuer list is set, before their `nbf`, or that fail `openId.requiredJwtClaims`; each is `401` / `UNAUTHENTICATED`. See [../deploy/permissions-and-auth/oidc-and-token-audiences.md](../deploy/permissions-and-auth/oidc-and-token-audiences.md).
- `principal` in the body is optional. When omitted, the caller (token subject) is evaluated. When present and different from the caller, the reference service first checks `permissions:view` for the caller; this is how a storage or notification service asks about an end user. The contract lets an implementation answer a caller without that right with `403`/`PERMISSION_DENIED` or with `deny`. The reference service, without evaluating the service's policies, rejects a **single** request with `403 Forbidden` (gRPC `PERMISSION_DENIED`, body `Permission denied`) and records a per-action `deny` in a **batch** ([`policy-evaluation.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-evaluation.md), stage 3 "Access Control Gate"; `service/authorization.rs` `ensure_permissions_can_be_viewed`). The comparison uses the resolved principal ids (service `idClaim`, then `openId.principalIdClaim`, then `sub`). This is the behaviour of both 0.0.53 and 0.0.54; the 0.0.53 service docs described a `deny` for single requests, which the 0.0.54 docs correct.

The two flow descriptions are [`authorization-for-a-user-principal.md`](../../../../permissions/reference/permission-service/docs/api/docs/authorization-for-a-user-principal.md) and [`authorization-for-a-service-principal.md`](../../../../permissions/reference/permission-service/docs/api/docs/authorization-for-a-service-principal.md).

## Models

| Message / JSON object | Fields | Constraints (REST) |
|---|---|---|
| `Principal` | `sub` (string, required); `info` (object of other claims) | `sub` ≤ 255 chars, no control chars, `"`, `'`, `\` |
| `Action` | `name`, `service` (both required) | each ≤ 255, pattern `^[a-zA-Z0-9_.:/-]+$` |
| `Resource` | `id`, `type` (required); `data` (object, optional) | `id` ≤ 2048, no control chars, `"`, `\`; `type` pattern as above |
| `CheckPermissionRequest` / `AuthorizationPayload` | `principal?`, `action`, `resource?`, `context?` | — |
| `CheckPermissionResponse` / `AuthorizationResponse` | `decision`; `reason?` (explicit denials only); REST also echoes `action`, `service` | `reason` ≤ 2048 printable ASCII |
| `CheckPermissionBatchRequest` / `AuthorizationBatchRequest` | `condition?`, `batches[]` of `{principal?, actions[], resource?, context?}` | ≤ 100 batches; ≤ 1000 actions per batch; `(service, name)` pairs distinct within a batch |
| `CheckPermissionBatchResponse` / `AuthorizationBatchResponse` | `summary?`, `decisions[]` | REST: each decision object is keyed `"<service>:<action>"`; gRPC: `results[]` in action order |

Claims other than `sub` go in `principal.info`. Examples in the source skill package put `email` and `exp` at the top level of `principal`; this repo's contract does not.

`Decision`: `DECISION_UNSPECIFIED` 0, `DECISION_DENY` 1, `DECISION_ALLOW` 2, `DECISION_SKIP` 3 (REST `allow`/`deny`/`skip`). `Condition`: `CONDITION_UNSPECIFIED` 0 (REST `none`, default), `CONDITION_OR` 1, `CONDITION_AND` 2.

## Endpoints

| REST | gRPC | Purpose |
|---|---|---|
| `POST /v1beta/authorization/` | `CheckPermission` | One action on one resource |
| `POST /v1beta/authorization/batch/` | `CheckPermissionBatch` | Many actions, resources, principals |
| `GET /health` | `grpc.health.v1.Health/Check` | Liveness (reference service) |

Both return HTTP `200` / gRPC `OK` for any evaluated request; the outcome is in the body. A denial with `reason` is explicit; without `reason` it is implicit (no rule matched). The reference service returns `reason: "Explicit deny."` only when `config.enableDenyReason` is true (default true).

### Batch conditions

Evaluation runs across all batches in order (batch 0's actions, then batch 1's).

| `condition` | Stops at | Unevaluated actions | `summary` |
|---|---|---|---|
| `none` (default) | never | none | omitted |
| `and` | first `deny` | `skip` | `deny` if any denied, else `allow` |
| `or` | first `allow` | `skip` | `allow` if any allowed, else `deny` |

`skip` means "not evaluated", never "permitted". A caller that ignores `summary` and treats `skip` as allow will over-grant. (The User Info Service treats `skip` as permitted, but it only sends `none` batches, where `skip` does not occur.)

### Example: one check for an end user, made by a service

The calling service authenticates with its own token and names the end user as `principal`. The service principal therefore needs `permissions:view` (see [../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md)).

```http
POST /v1beta/authorization/ HTTP/1.1
Host: permission-service.<namespace>.svc.cluster.local:3000
Authorization: Bearer <service-access-token>
Content-Type: application/json

{
  "principal": { "sub": "<user-oid>", "info": { "groups": ["<group-id>"] } },
  "action":    { "service": "storage-service", "name": "read" },
  "resource":  { "type": "object", "id": "https://<bucket-host>/Projects/Scene.usd" }
}
```

```json
{ "decision": "allow", "action": "read", "service": "storage-service" }
```

### Example: "may the caller write all of these?"

```json
{
  "condition": "and",
  "batches": [
    { "actions": [ { "service": "storage-service", "name": "read" },
                   { "service": "storage-service", "name": "write" } ],
      "resource": { "type": "folder", "id": "https://<bucket-host>/Projects/" } }
  ]
}
```

```json
{ "summary": { "decision": "deny" },
  "decisions": [ { "storage-service:read":  { "decision": "allow" },
                   "storage-service:write": { "decision": "deny" } } ] }
```

The action service and resource type names must match what the deployment registers in metadata. The reference chart seeds `storage-service` with `object` and `folder`; examples elsewhere that use `storage`/`File` are illustrative and match no seeded policy.

### Caching

The contract encourages callers to cache decisions for a limited time. The reference service sends `Cache-Control` only when `config.cacheControlMaxAge` is non-zero (default `0`, disabled). Storage, notifications, and User Info keep their own caches; their TTLs bound how long a policy change takes to apply (for example User Info `permission.cacheTtl` 5m).

## Errors

| Condition | REST | gRPC |
|---|---|---|
| Malformed request or constraint violation (including missing `action`, duplicate actions in a batch) | `400` | `INVALID_ARGUMENT` |
| Missing, invalid, or expired token | `401` | `UNAUTHENTICATED` |
| Caller may not check this principal (the reference service does this for single requests; batches get a per-action `deny`) | `403` | `PERMISSION_DENIED` |
| Rate limited or message too large | `429` (`Retry-After` optional) | `RESOURCE_EXHAUSTED` |
| Server error | `500` | `INTERNAL` |

REST error bodies are a plain JSON string, for example `"The principal token is expired."`.

The repo contract says malformed input **must** be rejected with `400`/`INVALID_ARGUMENT` and **must not** be returned as `DECISION_DENY`. The source skill package said a missing action or unknown resource returned an explicit `DECISION_DENY`, and listed `413` and `422`; follow this repo's [`grpc.md`](../../../../permissions/reference/permission-service/docs/api/docs/grpc.md#error-codes) and [`rest-api.md`](../../../../permissions/reference/permission-service/docs/api/docs/rest-api.md#error-codes). The reference service's gRPC maximum decode size is `grpc.tonicMaxDecodingMessageSize` (4194304 bytes).

## Reference service management APIs (not in the contract)

These exist only on the reference Permission Service. Custom adapters need not implement them, but the Permission UI and the Storage Permission UI depend on them.

| Area | Endpoints | Gate (when auth enabled) | Doc |
|---|---|---|---|
| Policies | `GET /v1beta/policies/` (`page`, `limit` 1–50, `principal`, `group`, `action`, `resource`, `scope=exact\|inherited`, `order`); `GET /v1beta/policies/{id}`; `PUT /v1beta/policies/` (upsert by canonical text: `201` new, `200` updated); `PUT /v1beta/policies/batch/` (≤ 100, all-or-nothing); `DELETE /v1beta/policies/{id}` (`204`, idempotent) | `permissions:view` / `permissions:edit` | [`policy-management.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-management.md) |
| Service metadata | `GET`/`PUT`/`DELETE /v1beta/services/{service}/`, `.../actions/`, `.../actions/{action}/`, `.../resource-types/`, `.../resource-types/{type}/`; `GET /v1beta/services/` | `permissions:meta:read` / `permissions:meta:write` | [`metadata.md`](../../../../permissions/reference/permission-service/docs/service/docs/metadata.md) |
| Diagnostics | `POST /v1beta/diagnostics/authorize/`, `POST /v1beta/diagnostics/authorize/batch/` | `permissions:diagnostics` (checked even for self) | [`diagnostics.md`](../../../../permissions/reference/permission-service/docs/service/docs/diagnostics.md) |
| JWKS passthrough | `GET /v1beta/idp/keys/` (public) | none | code only: `crates/permission-service/src/http/idp.rs` |
| API browser | `GET /` (Swagger UI), `GET /openapi.json` | none | [`policy-management.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-management.md#interactive-api-reference) |

In config-file mode every write returns `501 Not Implemented`. A policy payload is `{ "policy": "<one Cedar statement>", "order"?: int, "comment"?: string }` (policy ≤ 65 535 chars, comment ≤ 4 096). Scopes are inferred from the Cedar text; client-sent scope fields are ignored. Policies are validated as one Cedar statement against their inferred scopes, **not** against registered service metadata, so a policy naming an unregistered action or resource type is accepted (per the corrected 0.0.54 [`policy-management.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-management.md)). Writes publish `omni.permissions.changed` / `omni.permissions.meta.changed` when notifications are enabled ([`notifications.md`](../../../../permissions/reference/permission-service/docs/service/docs/notifications.md); event plumbing in [notifications.md](notifications.md)).

## Testing a compatible implementation

The [conformance suite](../../../../permissions/reference/permission-service/crates/permission-service-conformance-test/README.md) checks health, allow/deny, and `and`/`or`/`none` batches over REST and gRPC. It needs one pre-provisioned allowed tuple (`conformance-test-setup` creates it on the reference service). Details and in-cluster runs: [../test/conformance.md](../test/conformance.md). Building your own implementation: [../develop/custom-permissions-adapter.md](../develop/custom-permissions-adapter.md).
