# Build a custom permissions adapter

> **Applies to:** implementations of the Permission API other than the reference Permission Service, and authorization backends behind it · **Released:** `permission-api` 1.0.0-beta.1 (NGC resource and [HTML docs](https://docs.omniverse.nvidia.com/ovstorage/permission-api/1.0.0-beta.1/index.html)); reference Permission Service 0.0.53 · **This repo's snapshot:** `nvidia.omniverse.permission.v1beta` proto and `openapi.json` in Permission Service 0.0.54 (contract unchanged from 0.0.53; `openapi.json` still says 0.0.53); conformance crate in the same workspace · **Sources:** `permissions/reference/permission-service/{protos,openapi.json,docs/api/docs,crates/permission-service-conformance-test,crates/permission-service/src/grpc/capabilities.rs,README.md}`, caller charts (`storage/reference/storage-service/helm/values.yaml` and `docs/helm/storage-permissions.rst`, notification charts, `user-identity/.../helm/.../values.yaml`), the [source skill package](../../NOTICE.md)

## Choose the smallest change that works

| Need | Do this | Effort |
|---|---|---|
| Different rules, same engine | Keep the reference Permission Service; express your RBAC/ABAC/ReBAC model as Cedar policies and metadata ([../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md)) | lowest |
| Rules owned by GitOps, no runtime edits | Reference service in `database.type: config-file`; policy and metadata YAML in reviewed source control; files reload on change | low |
| Decisions must come from an existing enterprise authorization system | Implement the Permission API contract in front of that system (this page) | high |
| Different edge authentication | Any Envoy `ext_authz` implementation can replace the Envoy auth extension in Contour's `ExtensionService`; the extension is Apache-2.0, so you can fork it or write your own | medium |

An adapter replaces only the decision endpoints. The reference service's policy, metadata, and diagnostics APIs are not part of the contract; without them the [Permission UI and Storage Permission UI](../components/permission-ui.md) do not work, and policy administration happens in your system's own tools.

## The contract you must implement

Definitions: [`permission_service.proto`](../../../../permissions/reference/permission-service/protos/nvidia/omniverse/permission/v1beta/permission_service.proto) (package `nvidia.omniverse.permission.v1beta`), [`openapi.json`](../../../../permissions/reference/permission-service/openapi.json), normative text in [`rest-api.md`](../../../../permissions/reference/permission-service/docs/api/docs/rest-api.md) and [`grpc.md`](../../../../permissions/reference/permission-service/docs/api/docs/grpc.md). For a 1.3.0 deployment, generate from the released `permission-api` 1.0.0-beta.1 resource; check that it matches these files before mixing them. Field-by-field summary: [../apis/permissions.md](../apis/permissions.md).

Implement both transports unless you control every caller. In this stack the callers default to different ones:

| Caller | Transport and default endpoint | Config |
|---|---|---|
| Storage Service | gRPC `http://permission-service:3010` (REST optional) | `config.permissions.client.grpc.endpoint` / `.rest`; batches of `config.permissions.batchSize` (100) |
| Event Aggregation / Consumer | REST `http://permission-service:3000` | `permissions.endpoint` ([../deploy/notifications.md](../deploy/notifications.md)) |
| User Info Service | REST `http://permission-service:3000/` | `permission.url` |
| Your services | your choice | [add-your-service.md](add-your-service.md) |

MUST-level behaviour (from the repo contract):

1. `POST /v1beta/authorization/` and `CheckPermission`; `POST /v1beta/authorization/batch/` and `CheckPermissionBatch`. Same inputs, same decisions on both transports.
2. Evaluated requests return HTTP `200` / gRPC `OK`, with the decision in the body. REST responses echo `action` and `service`; gRPC `ResourceActionDecision` sets both.
3. Explicit denials set `reason`; implicit denials (no rule) omit it.
4. Batch: one decision object per request batch, in order. REST keys are `"<service>:<action>"`; duplicate `(service, name)` pairs in a batch are a `400`. `condition` `none` (no summary, no `skip`), `and` (stop at first deny), `or` (stop at first allow); short-circuit spans batches; un-evaluated actions are `skip`; `summary` present only for `and`/`or`.
5. Malformed input (including missing `action`, constraint violations) is `400` / `INVALID_ARGUMENT`, never a `DECISION_DENY`. Bad credentials are `401` / `UNAUTHENTICATED`. Rate limiting or oversize is `429` / `RESOURCE_EXHAUSTED`. REST error bodies are a plain JSON string.
6. If a caller asks about a `principal` other than itself and is not allowed to, either reject (`403` / `PERMISSION_DENIED`) or return `deny` — consistently on both transports.
7. Field limits: `sub` ≤ 255; `action.name`/`service`/`resource.type` ≤ 255 matching `^[a-zA-Z0-9_.:/-]+$`; `resource.id` ≤ 2048; ≤ 100 batches; ≤ 1000 actions per batch.

The source skill package described malformed or unknown actions as explicit `DECISION_DENY` and listed `413`/`422`. Follow the repo contract above; the conformance suite and current callers expect it.

Also implement, for operability:

- `GET /health` (the conformance suite polls it) and `grpc.health.v1.Health`.
- `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices`, unauthenticated, returning one `ServiceEntry` with service type `permission`, gRPC service `nvidia.omniverse.permission.v1beta.PermissionService`, and REST endpoint `permission` version `v1beta` path `/v1beta/`. The reference implementation is [`capabilities.rs`](../../../../permissions/reference/permission-service/crates/permission-service/src/grpc/capabilities.rs). Register the Service with the Discovery annotations ([../deploy/discovery.md](../deploy/discovery.md)).
- Optionally `Cache-Control: max-age=<n>` on decisions, to let clients cache.

## Authentication inside the adapter

Validate every bearer token yourself; do not rely only on the edge, because in-cluster callers reach you directly. Mirror the reference service's checks as of 0.0.54: signature against the IdP JWKS (cache keys, back off on fetch failure), `exp` and `nbf` with bounded leeway (at most 300 s), `aud` **required** and matched against an explicit list (refuse to start without one), `iss` matched against an explicit list and required whenever the list is set, optional claim rules (the reference `requiredJwtClaims` operators `equals`, `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, `exists`), a maximum token length before parsing, and bounded IdP request timeouts. Released 0.0.53 accepted tokens without `aud`, or without `iss` when issuers were listed, and did not check `nbf`; do not copy that behaviour. Resolve the principal id from a configurable claim (`oid` on Entra) and expose the groups claim to your engine. Cache validated tokens keyed by a hash of the token, never the raw value, with a separate, smaller negative cache that also records tokens failing claim rules. The Apache-2.0 [`permission-auth`](../../../../permissions/reference/permission-service/crates/permission-auth/) crate implements exactly this in Rust if you can reuse it (MSRV 1.88 from 0.0.54).

If your adapter supports URL rewriting like the reference `config.urlRemapper`, resolve `.` and `..` path segments (including percent-encoded `%2e%2e`) before matching prefixes; otherwise a caller can name one location's id under another location's prefix (fixed in the reference service in 0.0.54).

## Mapping the stack's requests onto your model

Callers send these PARC values; your system must understand them or translate them.

| Service (`action.service`) | Actions (`action.name`) | Resource `type` / `id` |
|---|---|---|
| `storage-service` | `read`, `write` by default; remappable per operation through Storage `config.permissions.actionNames` (`stat`, `read`, `write`, `delete`, `list`, `list_top_level_addresses`, `list_routes`, metadata operations, …) | `object` / `folder`, id = storage URL (folders end in `/`) |
| `event-aggregation-service` | `publish-event` | `EventType` |
| `event-consumer-service` | `consume-all-storage-create-events`, `consume-all-storage-delete-events`, `consume-durable-queues`, `create-durable-queues`, `delete-durable-queues`, `inspect-and-replay-all-dlq` | none |
| `userinfo` (configurable `permission.serviceName`) | `list-users`, `get-user`, `list-user-groups`, `get-user-group`, `list-groups`, `get-group`, `list-group-members`, `get-group-member` | `User` / `Group`, id = directory object id; post-filter requests carry entity attributes in `resource.data` |

Names come from the reference chart's metadata seed, Permission Service [`policy-language.md`](../../../../permissions/reference/permission-service/docs/service/docs/policy-language.md#actions-and-resources-for-omniverse-services), Storage [`storage-permissions.rst`](../../../../storage/reference/storage-service/docs/helm/storage-permissions.rst), and User Info [`authorization.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/authorization.md). Capture real traffic from your deployment (with tokens redacted) before finalizing a mapping; caller versions can add actions.

Design points that change outcomes:

- **Hierarchy.** Storage ids are URLs. Decide how a grant on `https://host/Projects/` applies to `https://host/Projects/a/b.usd`, and match on folder boundaries, not raw string prefixes (`/Projects-archive/` must not match).
- **Canonical ids.** If Storage uses a URL remapper or normalizer, authorize the canonical form, or both services will disagree ([../components/common-libraries.md](../components/common-libraries.md)).
- **Unknown actions and resources.** Return `deny` with no matching rule; never default to allow.
- **Failure.** If your backend is unreachable, return `500` / `INTERNAL`, never a synthetic `allow`. User Info is documented to fail closed on errors; confirm each other caller's behaviour before relying on it.
- **Batch cost.** Storage sends up to 100 checks per batch and User Info sends one batch per filtered page. Evaluate batches in one backend round trip where possible, and keep p99 latency low; every storage listing waits on you.
- **Change propagation.** Callers cache decisions (User Info `permission.cacheTtl` 5m; notifications `permissions.ttl_seconds`). Document revocation latency as the sum of your cache and theirs. Publishing `omni.permissions.changed` events (schema in the reference [`notifications.md`](../../../../permissions/reference/permission-service/docs/service/docs/notifications.md)) lets consumers invalidate sooner.
- **Meta-checks.** If services ask about users other than themselves, enforce an equivalent of `permissions:view` so an arbitrary authenticated caller cannot enumerate another user's rights.

## Deploy it in place of the reference service

Keep the Service name `permission-service` and ports 3000 (REST) / 3010 (gRPC), or change every caller's endpoint. Run it as non-root with a read-only root filesystem, drop all capabilities, and keep it ClusterIP behind a NetworkPolicy. Package it with a Helm chart following [add-your-service.md](add-your-service.md).

## Prove it conforms

The suite is [`permission-service-conformance-test`](../../../../permissions/reference/permission-service/crates/permission-service-conformance-test/README.md) (REST, gRPC, or both). Provision one allowed tuple in your backend first; the deny cases use the action `conformance-test-nonexistent:deny-action`, which must not match any rule.

```bash
# from permissions/reference/permission-service; token flags as in the crate README
cargo run --bin permission-service-conformance-test -- \
  --rest-url http://localhost:3000 --grpc-url http://localhost:3010 \
  --principal <allowed-principal> --action-service <svc> --action-name <action> \
  --resource-type <type> --resource-id <id>
```

In-cluster runs use the [`permission-service-conformance-test`](../../../../permissions/reference/permission-service/helm/permission-service-conformance-test/) chart (`conformanceTest.restUrl`, `grpcUrl`, `principal`, `actionName`, `actionService`, `resourceId`, `resourceType`, plus `serviceIdentity` for authenticated targets). Its default `serviceAccount.automount: true` and empty security contexts are looser than the service charts; tighten them for shared clusters. More on running suites: [../test/conformance.md](../test/conformance.md).

A pass covers the decision contract only. Add your own tests for hierarchy, unknown actions, token validation (wrong `aud`, wrong `iss`, expired, oversized), backend outage behaviour, and the full stack: Storage with `config.permissions.enabled: true` against your adapter ([../test/integration-tests.md](../test/integration-tests.md)).
