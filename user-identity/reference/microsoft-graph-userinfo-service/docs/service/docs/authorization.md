(userinfo-service-authorization-authorization-with-the-permission-service)=
# Authorization with the Permission Service

User Info Service can optionally delegate authorization to the Omniverse Storage Permission
Service. When the integration is enabled, every directory request is authorized against a named
action before it runs, and each item in a list response is checked individually so that callers
only ever see the users and groups they are permitted to see. This page describes the actions the
service uses, how each endpoint is validated, how list responses are filtered, which endpoints are
exempt, how decisions are cached, and how the service behaves when the Permission Service is
unavailable.

The integration is off by default. When it is off, nothing on this page applies: the service
performs no authorization calls and serves every request that reaches it, relying on the platform's
authentication gateway to reject unauthenticated callers. Enabling it does not change the
service's own view of the directory — it only decides which of the directory records a given
caller is allowed to receive.

(userinfo-service-authorization-model)=
## Authorization Model

Authorization is evaluated per request against the caller's bearer token — the same token the
platform's authentication gateway validates. The service never inspects or trusts the token's
contents for the authorization decision; it forwards the token to the Permission Service and lets
the Permission Service decide. Each check names three things:

- **The caller** — identified by the bearer token, forwarded unchanged.
- **The action** — a named operation such as `list-users` or `get-user`, qualified by the service
  name the deployment is configured with (`userinfo` by default).
- **The resource** — for actions that target a specific user or group, the stable identifier and
  kind (`User` or `Group`) of that target. List actions that are not tied to a single target carry
  no resource.

The Permission Service returns one of three decisions for each check:

| Decision | Meaning | Effect |
|---|---|---|
| `allow` | The caller is explicitly permitted. | The request proceeds, or the item is kept. |
| `skip` | No policy applies; the Permission Service defers to the service. | Treated as permitted — the request proceeds, or the item is kept. |
| `deny` | The caller is explicitly refused. | The request is rejected, or the item is removed from the response. |

Both `allow` and `skip` let the request or item through; only `deny` blocks it.

(userinfo-service-authorization-actions)=
## Actions and the Endpoints They Guard

Each endpoint maps to exactly one named action. The same action names apply to both the REST and
gRPC surfaces, because the two APIs are equivalent. The table below lists every action, the REST
path and gRPC method it guards, and the resource the pre-check names.

| Action | REST endpoint | gRPC method | Pre-check resource |
|---|---|---|---|
| `list-users` | `GET /users` | `ListUsers` | _(none)_ |
| `get-user` | `GET /users/{userId}` | `GetUser` | the target user |
| `list-user-groups` | `GET /users/{userId}/groups` | `ListUserGroups` | the user whose groups are listed |
| `get-user-group` | `GET /users/{userId}/groups/{groupId}` | `GetUserGroup` | the user whose membership is checked |
| `list-groups` | `GET /groups` | `ListGroups` | _(none)_ |
| `get-group` | `GET /groups/{groupId}` | `GetGroup` | the target group |
| `list-group-members` | `GET /groups/{groupId}/users` | `ListGroupMembers` | the group whose members are listed |
| `get-group-member` | `GET /groups/{groupId}/users/{userId}` | `GetGroupMember` | the group whose membership is checked |

For the membership endpoints the pre-check names the container being queried — the user for
`list-user-groups` and `get-user-group`, the group for `list-group-members` and `get-group-member`
— not the items that will be returned. Those returned items are authorized separately, as
described below.

(userinfo-service-authorization-request-validation)=
## How Requests Are Validated

Every authorized request goes through a single pre-check before the requested operation runs:

1. The service determines the action and, where applicable, the target resource from the matched
   route and its path parameters. Requests to unrecognized paths are not authorized here and are
   processed normally.
2. The bearer token is read from the request. If the integration is enabled and no bearer token is
   present, the request is rejected as unauthorized (`401` / `UNAUTHENTICATED`) before any
   Permission Service call is made.
3. The service asks the Permission Service whether the caller may perform the action on the
   resource.
4. On `allow` or `skip`, the requested operation runs. On `deny`, the request is rejected as
   forbidden (`403` / `PERMISSION_DENIED`) and the operation never runs.

Because the pre-check runs before the operation, a caller who is not permitted to perform an
operation never causes the underlying directory to be read.

(userinfo-service-authorization-response-filtering)=
## How List Responses Are Filtered

The pre-check authorizes the operation as a whole; it does not decide which individual users or
groups a caller may see. For that, every list endpoint additionally applies a per-item check to the
page it is about to return:

- After a list operation produces a page of results, each item on that page is checked against the
  matching per-item action — `get-user` for every returned user, `get-group` for every returned
  group.
- Items the caller is not permitted to see (`deny`) are removed from the page. Items that are
  `allow` or `skip` are kept.
- Filtering is applied to the current page only; pagination is unchanged. A page may therefore
  return fewer items than the requested page size while still reporting a next-page cursor.

The per-item checks carry the full attributes of each candidate item, not just its identifier, so
that Permission Service policies can decide based on the record's contents. The mapping of list
endpoint to per-item action is:

| List endpoint | Per-item action | Applied to |
|---|---|---|
| `GET /users`, `ListUsers` | `get-user` | each returned user |
| `GET /groups`, `ListGroups` | `get-group` | each returned group |
| `GET /groups/{groupId}/users`, `ListGroupMembers` | `get-user` | each returned member |
| `GET /users/{userId}/groups`, `ListUserGroups` | `get-group` | each returned group |

The single-item get endpoints (`get-user`, `get-group`, `get-user-group`, `get-group-member`) do
not need a separate filtering step: their pre-check already authorizes the one record they return.

To keep per-item checks efficient, all items on a page are authorized together in a single batch
request to the Permission Service rather than one call per item.

(userinfo-service-authorization-exempt)=
## Exempt Endpoints

Some endpoints are never subject to permission checks, regardless of whether the integration is
enabled:

- **Health and readiness probes** (`GET /healthz`, `GET /readyz`) — infrastructure endpoints that
  carry no directory data and must remain reachable by the platform.
- **Current-user requests** (`GET /users/me` and the `GetCurrentUser` gRPC method) — a caller is
  always allowed to retrieve their own directory record. The identity is taken from the caller's
  own token, so there is nothing to authorize beyond the authentication the gateway has already
  performed. See {ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>` for how the caller is
  resolved from the token.

(userinfo-service-authorization-caching)=
## Decision Caching

To avoid calling the Permission Service repeatedly for the same caller and operation, the service
caches each decision in memory. A cached entry is keyed by the combination of:

- a fingerprint of the bearer token — the token is hashed rather than stored directly, so raw
  credentials never sit in the cache;
- the action; and
- the target resource, where one applies.

Both pre-check and per-item decisions are cached the same way. When a page is filtered, items whose
decision is already cached are served from the cache and only the remaining items are sent to the
Permission Service in a batch. Entries expire after a configurable time-to-live, and the cache is
bounded by a configurable maximum number of entries; once the limit is reached, the least-recently
used entries are evicted.

Because decisions are cached per caller and action, a policy change on the Permission Service takes
effect for an already-cached caller only after the cached entry expires. Choose the time-to-live to
balance responsiveness to policy changes against the load placed on the Permission Service.

(userinfo-service-authorization-failure-behavior)=
## Behavior When the Permission Service Is Unavailable

Authorization fails closed. If the service cannot obtain a decision, it refuses the request rather
than serving data the caller might not be entitled to:

| Situation | Result |
|---|---|
| Permission Service rejects the token (`401`) | Request rejected as unauthorized (`401` / `UNAUTHENTICATED`). |
| Permission Service reports the caller is refused (`deny`) | Request rejected as forbidden (`403` / `PERMISSION_DENIED`). |
| Permission Service reports rate limiting (`429`) | Request rejected as rate-limited (`429` / `RESOURCE_EXHAUSTED`); the retry timing the Permission Service returns is passed back to the caller. |
| Permission Service unreachable, times out, or returns any other error | Request rejected as an internal error (`500` / `INTERNAL`). |

During per-item filtering, a batch check that cannot be completed fails the whole list request as
an internal error for the same reason: the service will not return a partially authorized page.

(userinfo-service-authorization-configuration)=
## Configuration

The integration is configured under the `permission` section of the Helm chart's values, which the
chart translates into the settings below. When authorization is disabled, none of the other
settings have any effect.

| Setting | Environment variable | Helm value (`permission.*`) | Default | Description |
|---|---|---|---|---|
| Enable authorization | `PERMISSION_ENABLED` | `enabled` | `false` | Authorize every non-exempt request against the Permission Service. |
| Permission Service URL | `PERMISSION_URL` | `url` | `http://permission-service:3000/` | Base URL of the Permission Service. Set it to the Permission Service address in your cluster when the default does not match. |
| Service name | `PERMISSION_SERVICE_NAME` | `serviceName` | `userinfo` | Name that qualifies the action in authorization requests; identifies this service in Permission Service policies. |
| Decision cache time-to-live | `PERMISSION_CACHE_TTL` | `cacheTtl` | `5m` | How long a cached decision remains valid before it is re-evaluated. |
| Decision cache capacity | `PERMISSION_CACHE_MAX_CAPACITY` | `cacheMaxCapacity` | `10000` | Maximum number of cached decisions before least-recently-used entries are evicted. |

The Permission Service must contain policies and service metadata for the configured service name
before the integration is enabled; otherwise every request is refused.

(userinfo-service-authorization-helm)=
### Configuring Authorization in the Helm Chart

Authorization is configured under the `permission` section of `userinfo-values.yaml`. It is off by
default; set `enabled` to `true` and point `url` at the Permission Service to turn it on:

```yaml
permission:
  enabled: true
  url: "http://permission-service.storage-apis.svc.cluster.local:3000/"
  serviceName: "userinfo"
  cacheTtl: "5m"
  cacheMaxCapacity: 10000
```

Every value has a default as listed in the table above; `url` defaults to
`http://permission-service:3000/`, so set it whenever the Permission Service runs at a different
address. The chart passes these settings to the reader pods only — the writer performs background
synchronization and does not serve the read APIs, so it makes no authorization calls. The reader
always receives the enable switch (`PERMISSION_ENABLED`); when `enabled` is `false`, the chart
omits the other authorization environment variables from the reader, and the rest of the section
has no effect.

Install or upgrade the release with the same commands used for the synchronized directory
deployment in {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

When authorization is enabled together with directory scoping, the two features are complementary:
scoping decides which groups and memberships exist in the served directory, while authorization
decides which of those the caller may see. Because scoping aligns the served groups with the
enterprise application's assignments, it matches the view the Permission Service itself has of the
caller's groups; see {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>` for details.

(userinfo-service-authorization-see-next)=
## See next

- {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`
