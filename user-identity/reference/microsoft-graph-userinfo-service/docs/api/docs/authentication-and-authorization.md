(userinfo-api-authentication-and-authorization-title)=
# Authentication and Authorization

(userinfo-api-authentication-and-authorization-api-versioning)=
## API versioning

The API follows a graduation model with three maturity levels: **alpha**, **beta**, and **stable**. The current API version is `v1alpha`.

(userinfo-api-authentication-and-authorization-maturity-levels)=
### Maturity levels

| Level | Path / package prefix | Stability guarantee |
|---|---|---|
| **Alpha** (`v1alpha`) | `/api/v1alpha/…`, `nvidia.omniverse.userinfo.v1alpha` | No backward compatibility guarantee. Endpoints, request/response shapes, and behavior MAY change or be removed in any release. Intended for early integration and feedback. |
| **Beta** (`v1beta`) | `/api/v1beta/…`, `nvidia.omniverse.userinfo.v1beta` | Backward-compatible changes only (additive fields, new optional parameters, new endpoints). Existing request/response contracts MUST NOT break. Intended for production use with the understanding that minor adjustments may occur. |
| **Stable** (`v1`) | `/api/v1/…`, `nvidia.omniverse.userinfo.v1` | Full backward compatibility within the major version. Breaking changes require a new major version (`v2`). |

(userinfo-api-authentication-and-authorization-graduation)=
### Graduation

When an API graduates from one level to the next (e.g. `v1alpha` → `v1beta`), the previous version MUST remain available for a deprecation period to allow consuming services to migrate. The deprecation period MUST be communicated in advance and SHOULD be at least one release cycle.

During the deprecation period, both the old and new versions MUST be served simultaneously. Deprecated versions SHOULD return a warning header or response metadata indicating the deprecation status and the target removal date.

(userinfo-api-authentication-and-authorization-version-coexistence)=
### Version coexistence

Multiple API versions MAY be served concurrently. Each version MUST be independently addressable by its path prefix (REST) or package name (gRPC). Consuming services MUST explicitly select the version they depend on and MUST NOT assume that the latest version is backward-compatible with an earlier one.

---

(userinfo-api-authentication-and-authorization-authentication)=
## Authentication

All API operations require the caller to present a valid bearer token in the request's `Authorization` header (REST) or `authorization` metadata (gRPC). The service MUST reject requests that do not carry a bearer token with `401 Unauthorized` (REST) or `UNAUTHENTICATED` (gRPC).

Token authentication MUST be performed by the deployment infrastructure before requests reach the service — for example, by an API gateway, ingress controller, or service mesh sidecar. Implementations MAY additionally validate tokens within the service itself, but MUST NOT rely on this as the sole authentication mechanism.

Beyond authentication, implementations MAY use the bearer token for purposes such as:

- **Caller identity resolution.** Extracting claims from the token to determine which directory user the caller corresponds to — for example, to serve the get-current-user endpoint.
- **Authorization forwarding.** Forwarding the token to the Permission Service for per-endpoint authorization checks when that integration is enabled.
- **Identity provider delegation.** Passing the token to the identity provider's API on behalf of the caller when the implementation queries the provider directly rather than serving a copy of the directory.
- **Audit and logging.** Extracting caller identity or session information from the token for request tracing and audit trails.

---

(userinfo-api-authentication-and-authorization-permission-api-integration)=
## Authorization — Permission API integration

User Info Service MAY integrate with the Omniverse Storage Permission Service for per-endpoint directory authorization. Integration MUST be disabled by default and MUST be enabled only through deployment configuration so that deployments using other authorization mechanisms are not required to deploy the Permission Service.

When integration is enabled, the implementation MUST authorize every REST and gRPC request as specified in this section before processing. Each required check MUST be answered by the Permission Service, or by a cached Permission Service decision only as permitted in {ref}`Decision caching <userinfo-api-authentication-and-authorization-decision-caching>`. When it is disabled, the implementation MUST omit those checks and MUST NOT apply the authorization model defined below; {ref}`Authentication <userinfo-api-authentication-and-authorization-authentication>` requirements continue to apply, but User Info Service MUST NOT impose further restrictions on which authenticated callers may read user or group data.

Deploying organizations that operate with integration disabled MUST either enforce equivalent directory authorization through external controls appropriate to the deployment's threat model (for example, an API gateway or separate AuthZ product), or explicitly accept that authenticated callers able to reach the service may access the directory APIs and user and group data in this specification without additional authorization enforced by User Info Service.

(userinfo-api-authentication-and-authorization-token-forwarding)=
### Token forwarding

The service MUST forward the caller's Bearer token to the Permission Service for every authorization check. The Permission Service determines the caller's identity from the token. For authorization checks, User Info Service MUST NOT interpret the token or substitute identity claims extracted from it for the Permission Service's decision; resolving the caller for the get-current-user endpoint is covered by {ref}`Authentication <userinfo-api-authentication-and-authorization-authentication>`.

This applies to both interfaces. REST requests carry the token in the `Authorization` header, and gRPC requests carry it in the `authorization` metadata. In both cases, the same token MUST be forwarded.

(userinfo-api-authentication-and-authorization-action-mapping)=
### Action mapping

Each API endpoint MUST be mapped to a named authorization action. Actions that target a specific entity MUST additionally pass the resource identity (type and identifier) to the Permission Service. Both REST and gRPC equivalents MUST use the same action names and authorization behavior.

| REST endpoint | gRPC RPC | Pre-request action | Resource | Post-filter action |
|---|---|---|---|---|
| `GET /users` | `ListUsers` | `list-users` | — | `get-user` per item |
| `GET /users/{userId}` | `GetUser` | `get-user` | User | — |
| `GET /users/{userId}/groups` | `ListUserGroups` | `list-user-groups` | User | `get-group` per item |
| `GET /users/{userId}/groups/{groupId}` | `GetUserGroup` | `get-user-group` | User | — |
| `GET /groups` | `ListGroups` | `list-groups` | — | `get-group` per item |
| `GET /groups/{groupId}` | `GetGroup` | `get-group` | Group | — |
| `GET /groups/{groupId}/users` | `ListGroupMembers` | `list-group-members` | Group | `get-user` per item |
| `GET /groups/{groupId}/users/{userId}` | `GetGroupMember` | `get-group-member` | Group | — |

Health and readiness probes, and get-current-user (`GET /users/me` / `GetCurrentUser`), MUST be exempt from permission checking — the former do not expose directory data, and the latter returns only the caller's own profile.

(userinfo-api-authentication-and-authorization-endpoint-authorization)=
### Endpoint authorization

Before processing any request to a mapped endpoint, the service MUST check the pre-request action against the Permission Service. If the action targets a specific resource, the resource MUST be included in the check. If the Permission Service denies the action, the service MUST reject the request immediately without invoking the handler.

List endpoints MUST perform a second authorization pass after the handler returns results. Each item in the response MUST be individually checked against the corresponding post-filter action from the action mapping table. Items the caller is not authorized to see MUST be removed from the response. This prevents a principal who is authorized to browse a directory listing from seeing individual entities they do not have access to. Resources passed during post-filtering MUST include all available entity attributes (such as display name, email, and group description) to support attribute-based access control (ABAC) policies.

The Permission Service returns one of three decisions: **allow**, **deny**, or **skip** (defer to the next applicable policy). The service MUST enforce them as follows:

- **Allow or skip** — the request proceeds.
- **Deny** — the request is rejected with `403 Forbidden` (REST) or `PERMISSION_DENIED` (gRPC).
- **Permission Service unreachable or error** — the request MUST fail with `500 Internal Server Error` (REST) or `INTERNAL` (gRPC). The service MUST fail closed: no request is allowed through when the authorization system is unavailable.

(userinfo-api-authentication-and-authorization-decision-caching)=
### Decision caching

Implementations MAY cache authorization decisions to reduce latency and load on the Permission Service. A cached decision stands in for a Permission Service check; it never removes a check that this section requires. When decisions are cached, the following observable behavior MUST hold:

- **Scope.** A cached decision MUST be reused only for a check with the same inputs: the same caller token, action, and resource type and identifier (if any). Any other check — for example, with a different token after a token refresh — MUST be answered by the Permission Service.
- **Staleness.** A cached decision MUST NOT be used after a deployment-configurable time-to-live has elapsed. Within that time-to-live the cached outcome is treated as unchanged, so a policy change in the Permission Service, or a change to the entity attributes passed during post-filtering, takes effect for every caller within that time-to-live.
- **Failures.** A failed or unreachable Permission Service call is not a decision and MUST NOT be cached.
- **Reuse across endpoints.** A decision obtained for an individual item while post-filtering a list response MAY be reused for a later pre-request check with the same token, action, and resource, such as a get-by-ID request for the same entity.

Implementations MAY reflect Permission Service policy changes sooner than the time-to-live, so that authorization decisions follow policy changes with minimal delay.

(userinfo-api-authentication-and-authorization-see-next)=
## See next

* {ref}`REST API <userinfo-api-rest-api-title>`
