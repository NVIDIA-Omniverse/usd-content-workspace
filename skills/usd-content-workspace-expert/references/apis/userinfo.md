# User Info API

> **Applies to:** the User Info API contract and its reference implementation, the Microsoft Graph User Info Service · **Released:** `userinfo-api` 1.0.0-alpha.1 (NGC); service 0.0.8 (usd-content-workspace Developer Guide 1.3.0) · **This repo's snapshot:** proto package `nvidia.omniverse.userinfo.v1alpha`, REST `/api/v1alpha/`, OpenAPI `info.version: v1alpha`; service 0.0.9 (`Cargo.toml`, `CHANGELOG.md`) · **Sources:** `user-identity/reference/microsoft-graph-userinfo-service/protos/nvidia/omniverse/userinfo/v1alpha/user_info_service.proto`, `openapi.yaml`, `docs/api/docs/{index,rest-api,grpc,authentication-and-authorization,use-cases,directory-synchronization}.md`, `docs/service/docs/authorization.md`, the [source skill package](../../NOTICE.md)

## What the contract covers

A read-only, provider-agnostic identity lookup: who a user is, which groups they are in, which users or groups match a name. NVIDIA's implementation serves a synchronized replica of Microsoft Entra ID; another provider can implement the same contract and validate it with the conformance suite. The API answers *who*, not *what they may do* — that is the [Permissions API](permissions.md).

| Wire surface | Definition in this repo | Version |
|---|---|---|
| gRPC `nvidia.omniverse.userinfo.v1alpha.UserInfoService` | [`user_info_service.proto`](../../../../user-identity/reference/microsoft-graph-userinfo-service/protos/nvidia/omniverse/userinfo/v1alpha/user_info_service.proto) | `v1alpha` |
| REST `/api/v1alpha/…` | [`openapi.yaml`](../../../../user-identity/reference/microsoft-graph-userinfo-service/openapi.yaml) (OpenAPI 3.0.3) and [`rest-api.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/rest-api.md) | `v1alpha` |
| Discovery `CapabilitiesService` (optional) | [`grpc.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/grpc.md) | `v2alpha`, service type `userinfo`, REST `userinfo` `v1alpha` `/api/v1alpha/` |

`v1alpha` carries **no backward-compatibility guarantee**: shapes may change in any release ([`authentication-and-authorization.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/authentication-and-authorization.md)). Pin generated clients to the definitions of the service you run (released `userinfo-api` 1.0.0-alpha.1, or this snapshot's files), and record the deployed version first ([../versions/capture-a-deployment.md](../versions/capture-a-deployment.md)). `openapi.yaml` declares fewer error statuses than `rest-api.md`; code against the full table below.

## Authentication — read this before exposing it

- Every call needs `Authorization: Bearer <token>` (REST) or `authorization` metadata (gRPC); missing token ⇒ `401` / `UNAUTHENTICATED`.
- The contract requires token **validation** to happen in the deployment infrastructure (gateway, ingress, mesh) before the request reaches the service. The reference service does not verify signatures: `/users/me` decodes `oid` from the token without verification. In this stack the gateway is the [Envoy auth extension](../deploy/permissions-and-auth/envoy-auth-extension.md) on the User Info `HTTPProxy` (`httpProxy.authExtension.enabled: true`, default `false`). In-cluster callers that bypass the ingress are not authenticated; restrict them with NetworkPolicy, or enable the Permission Service integration, which validates the forwarded token.
- The token's `aud` must be the audience your gateway accepts. A Microsoft Graph-scoped Entra token (`aud` = Graph) is rejected by a gateway configured for the User Info audience.

## Authorization (optional, off by default)

When `permission.enabled: true`, every non-exempt call is checked against the Permission Service by forwarding the caller's token unchanged; list results are then filtered item by item. Exempt: `/healthz`, `/readyz`, `/users/me` / `GetCurrentUser`.

| REST | gRPC | Pre-check action | Pre-check resource | Per-item filter |
|---|---|---|---|---|
| `GET /users` | `ListUsers` | `list-users` | — | `get-user` |
| `GET /users/{userId}` | `GetUser` | `get-user` | `User` | — |
| `GET /users/{userId}/groups` | `ListUserGroups` | `list-user-groups` | `User` | `get-group` |
| `GET /users/{userId}/groups/{groupId}` | `GetUserGroup` | `get-user-group` | `User` | — |
| `GET /groups` | `ListGroups` | `list-groups` | — | `get-group` |
| `GET /groups/{groupId}` | `GetGroup` | `get-group` | `Group` | — |
| `GET /groups/{groupId}/users` | `ListGroupMembers` | `list-group-members` | `Group` | `get-user` |
| `GET /groups/{groupId}/users/{userId}` | `GetGroupMember` | `get-group-member` | `Group` | — |

Actions are sent as `Action::"<permission.serviceName>:<action>"` (default service name `userinfo`). `allow` and `skip` pass; `deny` ⇒ `403`/`PERMISSION_DENIED`; Permission Service unreachable or erroring ⇒ `500`/`INTERNAL` (fails closed). A filtered page can be shorter than `limit` and still return a cursor. Policy examples: [../deploy/permissions-and-auth/cedar-policies.md](../deploy/permissions-and-auth/cedar-policies.md).

## Models

`User`: `id` (required), `display_name` (required), `username` (required, the UPN), `given_name?`, `family_name?`, `email?`, `active?`, `job_title?`.
`Group`: `id` (required), `display_name` (required), `description?`.

`id` is the provider's immutable identifier (Entra object id). Use it — not email or display name — in Cedar policies and stored metadata.

## REST endpoints (`/api/v1alpha`, all `GET`, JSON)

| Endpoint | Query | Returns |
|---|---|---|
| `/users/me` | — | `User` for the token's subject |
| `/users` | `email` (exact), `display_name`, `limit`, `cursor` | `{ "items": [User], "next_cursor": string\|null }` |
| `/users/{userId}` | — | `User` or `404` |
| `/users/{userId}/groups` | `display_name`, `limit`, `cursor` | page of `Group` (direct memberships) |
| `/users/{userId}/groups/{groupId}` | — | the `Group` if a direct member, else `404` |
| `/groups` | `display_name`, `limit`, `cursor` | page of `Group` |
| `/groups/{groupId}` | — | `Group` or `404` |
| `/groups/{groupId}/users` | `email`, `display_name`, `limit`, `cursor` | page of `User` (direct members) |
| `/groups/{groupId}/users/{userId}` | — | the `User` if a direct member, else `404` |

- `limit` default 20, maximum 100. Follow `next_cursor` until `null`.
- `display_name`: trailing `*` = prefix match (`Jan*`); otherwise substring. `email` is exact.
- Memberships are **direct only**; nested groups are not expanded.

```http
GET /api/v1alpha/users?email=jane.doe@example.com HTTP/1.1
Host: userinfo.<domain>
Authorization: Bearer <access-token>
```

```json
{ "items": [ { "id": "<object-id>", "display_name": "Jane Doe",
               "email": "jane.doe@example.com", "username": "jane.doe@example.com" } ],
  "next_cursor": null }
```

## gRPC methods

| RPC | Request → response | REST equivalent |
|---|---|---|
| `GetCurrentUser` | `GetCurrentUserRequest{}` → `GetCurrentUserResponse{ user }` | `/users/me` |
| `ListUsers` | `{ email?, display_name?, page_size, page_token }` → `{ users[], next_page_token }` | `/users` |
| `GetUser` | `{ user_id }` → `{ user }` | `/users/{userId}` |
| `ListUserGroups` | `{ user_id, display_name?, page_size, page_token }` → `{ groups[], next_page_token }` | `/users/{userId}/groups` |
| `GetUserGroup` | `{ user_id, group_id }` → `{ group }` | `/users/{userId}/groups/{groupId}` |
| `ListGroups` | `{ display_name?, page_size, page_token }` → `{ groups[], next_page_token }` | `/groups` |
| `GetGroup` | `{ group_id }` → `{ group }` | `/groups/{groupId}` |
| `ListGroupMembers` | `{ group_id, email?, display_name?, page_size, page_token }` → `{ users[], next_page_token }` | `/groups/{groupId}/users` |
| `GetGroupMember` | `{ group_id, user_id }` → `{ user }` | `/groups/{groupId}/users/{userId}` |

Every RPC returns a wrapper message, not a bare `User`/`Group` (the source skill package showed `GetCurrentUser → User`). `page_size` is capped at 100; `next_page_token` is empty on the last page.

```bash
# from a pod that can reach the Service; token supplied by your environment, never pasted inline
grpcurl -H "authorization: Bearer ${USERINFO_TOKEN}" \
  -d '{"display_name":"Jan*","page_size":5}' \
  userinfo-service.<namespace>.svc.cluster.local:50051 \
  nvidia.omniverse.userinfo.v1alpha.UserInfoService/ListUsers
```

gRPC reflection is enabled on the reference service, so `grpcurl` needs no local proto files. The call above is plaintext and therefore suitable only inside the cluster.

## Errors

| REST | `error` code | gRPC | Condition |
|---|---|---|---|
| `400` | `bad_request` | `INVALID_ARGUMENT` | Malformed request, e.g. unparseable cursor |
| `401` | `unauthorized` | `UNAUTHENTICATED` | Missing or invalid token |
| `403` | `forbidden` | `PERMISSION_DENIED` | Denied by authorization |
| `404` | `not_found` | `NOT_FOUND` | No such user/group, or not a direct member |
| `429` | `rate_limited` | `RESOURCE_EXHAUSTED` | Identity provider (or Permission Service) rate limit; honour `Retry-After` |
| `502` | `bad_gateway` | `UNAVAILABLE` | Identity provider returned an unexpected error |
| `500` | `internal_error` | `INTERNAL` | Internal error, including Permission Service unreachable |

REST error body: `{ "error": "<code>", "message": "<text>" }`.

## Freshness

Results come from an eventually consistent replica: a change in Entra ID appears after up to one writer sync interval (`providers.cache.syncInterval`, default `5m`) plus one reader reload interval (`reader.reloadInterval`, default `60s`), plus the sync time. Readers can briefly disagree. Use the identity provider directly when you need an authoritative read. Service-side details: [`directory-synchronization.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/service/docs/directory-synchronization.md); deployment knobs: [../deploy/userinfo.md](../deploy/userinfo.md).

## Use cases

From [`use-cases.md`](../../../../user-identity/reference/microsoft-graph-userinfo-service/docs/api/docs/use-cases.md): resolve email or group name to a stable id before writing policies or migrating Nucleus ACLs (see [../operate/migration.md](../operate/migration.md)); display names and autocomplete in the Permission UI, Storage Permission UI, and Navigator; identify the signed-in caller.

## Conformance

[`userinfo-conformance-test`](../../../../user-identity/reference/microsoft-graph-userinfo-service/crates/userinfo-conformance-test/README.md) validates paths, status codes, shapes, filters, pagination, and error mapping over REST and gRPC by sampling fixtures from the live directory (needs at least one user with an email and one group with a direct member within `--page-limit`). It does not assert particular users exist. See [../test/conformance.md](../test/conformance.md).
